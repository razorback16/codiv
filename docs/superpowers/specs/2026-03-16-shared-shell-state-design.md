# Shared Shell State Design

## Problem

When the AI agent executes bash commands (e.g., `cd /tmp`), the state change is lost because each command runs in a one-shot subprocess. The user's terminal and the AI operate in disconnected shell environments. Environment variable changes, working directory changes, and other shell state mutations don't propagate between the user and the AI.

## Design Goals

- User and Orchestrator agent share a single shell state (cwd, env vars, shell variables)
- Independent agents (engineer, team lead, etc.) get their own isolated shell state on the daemon
- Orchestrator pauses bash execution if the client disconnects (no fallback shell)
- Independent agent shells survive client disconnects
- Maximize code reuse between client-side and daemon-side shell implementations

## Architecture Overview

Two shell execution paths:

1. **Shared shell (User + Orchestrator):** The client's existing bash coprocess is the single execution engine. AI bash commands route through the client via IPC. State changes from either side are immediately visible to the other.
2. **Independent agent shells:** Daemon-side pipe-based `bash -i` processes. Fresh state with initial cwd set by the parent agent. Survive client disconnects.

## IPC Message Changes

### New DaemonMessage variant

```rust
DaemonMessage::ExecuteCommand {
    command: String,
    execution_id: String,   // unique ID to correlate request/response
    timeout_ms: u64,
}
```

### New ClientMessage variant

```rust
ClientMessage::CommandExecutionResult {
    execution_id: String,
    output: String,
    exit_code: i32,
    cwd: String,            // captured via `pwd` after command runs
}
```

The `execution_id` is separate from `request_id` (the LLM request) because a single LLM turn may issue multiple bash tool calls.

Note: The existing `ClientMessage::CommandResult` handles user-initiated shell commands (typed at the terminal). The new `CommandExecutionResult` handles AI-initiated commands relayed through the client. Both coexist — they represent different flows (user → daemon vs. daemon → client → daemon).

## Sync/Async Boundary

### The constraint

The daemon runs on a tokio async runtime. Tool execution closures are synchronous (`Fn(Value) -> Result<String, String>` via `ToolExecute`). Both shell backends involve blocking I/O:

- **ClientRelay:** Sends an IPC message and waits for a response (async operation)
- **DaemonShell:** Writes to stdin, reads from stdout until sentinel (blocking I/O)

Currently, aisdk's `ToolList::execute` calls the sync closure inside `tokio::spawn` (see `references/aisdk/src/core/tools.rs:219-234`). This means the blocking sync closure runs on a **tokio worker thread**, which would starve the runtime during long-running commands.

### Solution: Change aisdk to use `spawn_blocking`

**Required aisdk change:** Modify `ToolList::execute` to use `tokio::task::spawn_blocking` instead of `tokio::spawn`. Since tool closures are explicitly synchronous (`Fn(Value) -> Result<String, String>`), they belong on the blocking thread pool, not on async worker threads. This is a one-line change:

```rust
// Before (aisdk/src/core/tools.rs):
tokio::spawn(async move { tool.execute.call(input) })

// After:
tokio::task::spawn_blocking(move || tool.execute.call(input))
```

This unblocks both shell backends:

- **DaemonShell:** `ShellSession<PipeIO>.execute()` does blocking pipe I/O. Runs safely on the blocking thread pool.
- **ClientRelay:** The closure captures a `tokio::runtime::Handle` (obtained via `Handle::current()` at `build_tools` time). Inside the blocking thread, it uses `handle.block_on(async { ... })` to send the IPC message and await the oneshot response. This is safe because `spawn_blocking` runs on a dedicated thread pool, not a tokio worker.

```rust
// Pseudocode for the ClientRelay bash tool closure
move |value| {
    let handle = handle.clone();  // tokio::runtime::Handle
    handle.block_on(async {
        let (tx, rx) = oneshot::channel();
        pending.lock().unwrap().insert(exec_id.clone(), tx);
        client_tx.send(frame_message(&ExecuteCommand { ... })?).await?;
        tokio::time::timeout(Duration::from_millis(timeout_ms), rx).await??
    })
}
```

Note: `client_tx` is `tokio::sync::mpsc::Sender<Vec<u8>>` (the existing IPC sender type). Calling `.send().await` on it requires async context, which `handle.block_on` provides.

### DaemonShell Mutex

`DaemonShell` wraps its `ShellSession<PipeIO>` in a `std::sync::Mutex` (not `tokio::sync::Mutex`). Since all access happens on `spawn_blocking` threads, `std::sync::Mutex` is correct. Tool calls from a single LLM turn are serialized by the aisdk tool loop, so contention is minimal.

## Shared Shell Module (codiv-common)

Extract the sentinel protocol and command execution logic from `bash_coprocess.rs` into a shared module, parameterized over the I/O backend.

```rust
// codiv-common/src/shell.rs

pub trait ShellIO: Send {
    fn write_command(&mut self, cmd: &str) -> io::Result<()>;
    fn read_until_sentinel(&mut self, sentinel: &str) -> io::Result<String>;
}

pub struct ShellSession<IO: ShellIO> {
    io: IO,
}

impl<IO: ShellIO> ShellSession<IO> {
    pub fn execute(&mut self, command: &str) -> Result<CommandOutput> { ... }
    pub fn capture_cwd(&mut self) -> Option<String> { ... }
    pub fn capture_env(&mut self) -> Vec<(String, String)> { ... }
}
```

### Output cleaning: PTY vs Pipe differences

The `ShellIO` trait handles raw I/O. Output cleaning differs by backend:

- **PtyIO:** PTY output contains `\r\n` line endings, ANSI escape sequences, and echoed command text. `PtyIO::read_until_sentinel` strips `\r` characters and ANSI escapes before returning. The `strip_ansi` and `clean_output` functions from the current `bash_coprocess.rs` move into the `PtyIO` implementation.
- **PipeIO:** Pipe output is clean (no `\r`, no ANSI escapes when echo is disabled). `PipeIO::read_until_sentinel` returns raw output with no cleaning needed.

This keeps `ShellSession` clean — it doesn't need to know about output format differences. Each `ShellIO` implementation is responsible for returning sanitized text.

### Stderr handling

- **PtyIO (shared shell):** PTY merges stdout and stderr into a single stream. There is no separate stderr. All output comes through `read_until_sentinel`. This matches the current client coprocess behavior.
- **PipeIO (daemon shell):** Stderr is a separate pipe. A background thread drains it continuously into a `Arc<Mutex<String>>` buffer. After `execute()` completes, the accumulated stderr is appended to the output tagged with `<stderr>`. The buffer is cleared between commands.

### PTY-specific concerns not in the trait

PTY resize (`resize`, `resize_full`) is not part of `ShellIO`. It remains on the client's `BashCoprocess` wrapper, which owns the `PtyIO` and the PTY master handle. `ShellSession` does not expose resize.

Two implementations:
- `PtyIO` — wraps `portable-pty` reader/writer (used by client coprocess)
- `PipeIO` — wraps `ChildStdin`/`ChildStdout` (used by daemon shells)

The existing `bash_coprocess.rs` is refactored to become a thin wrapper that spawns the PTY and creates `ShellSession<PtyIO>`.

## DaemonShell

A new struct for independent agent shells.

### Lifecycle
- Spawned when an independent agent is created
- Parent specifies initial cwd
- Process: `bash -i` with stdin/stdout/stderr as pipes (interactive so `.bashrc` is sourced, giving access to toolchains like nvm, pyenv, cargo, etc.)
- On spawn: `stty -echo`, `HISTFILE=/dev/null`, `PS1=""`
- After setup commands complete, the stderr buffer is cleared to discard `.bashrc` sourcing noise and job control messages from interactive bash startup
- Dropped when the agent is destroyed (SIGTERM to child)

### Command execution
Uses the shared `ShellSession<PipeIO>` with the same sentinel protocol as the client coprocess. All calls happen inside `spawn_blocking` (see Sync/Async Boundary section).

### Concurrency
Commands are serialized via a `std::sync::Mutex` around the `ShellSession<PipeIO>`. Only accessed from `spawn_blocking` threads, never from async tasks directly.

### Stderr handling
A background thread drains stderr continuously and appends it to the current command's output buffer, tagged with `<stderr>`.

## Bash Tool Routing

### ShellBackend enum

```rust
enum ShellBackend {
    ClientRelay {
        client_tx: mpsc::Sender<Vec<u8>>,
        pending: Arc<Mutex<HashMap<String, oneshot::Sender<CommandExecutionResult>>>>,
        handle: tokio::runtime::Handle,
    },
    DaemonShell {
        shell: Arc<std::sync::Mutex<ShellSession<PipeIO>>>,
        stderr_buf: Arc<std::sync::Mutex<String>>,
    },
}
```

`build_tools()` takes a `ShellBackend` and an `Arc<RwLock<String>>` for the shared cwd reference. The bash tool closure matches on the backend variant.

### Orchestrator flow (ClientRelay)

```
1. LLM emits tool_call: bash { command: "cd /tmp && ls" }
2. Bash tool closure runs (inside spawn_blocking thread)
3. Generates unique execution_id
4. Creates oneshot channel (tx, rx)
5. Stores tx in pending_executions map
6. Sends DaemonMessage::ExecuteCommand to client via client_tx
7. Uses handle.block_on(rx) to await response (safe: runs in spawn_blocking context)
8. Client runs command through ShellSession<PtyIO> coprocess
9. Client sends back CommandExecutionResult { output, exit_code, cwd }
10. Daemon dispatch loop receives result, looks up execution_id, sends through oneshot tx
11. Bash tool gets result, updates shared cwd Arc<RwLock<String>>
12. Returns output to LLM
```

### Independent agent flow (DaemonShell)

```
1. LLM emits tool_call: bash { command: "cargo build" }
2. Bash tool closure runs
3. Locks the DaemonShell mutex
4. Calls shell.execute(command) (blocking, in spawn_blocking context)
5. Appends any accumulated stderr
6. Captures new cwd, updates shared Arc<RwLock<String>>
7. Returns output + exit code to LLM
```

### Timeout and disconnect handling

- **Timeout:** If the client doesn't respond within `timeout_ms`, the relay returns a timeout error to the LLM. Implemented via `tokio::time::timeout` wrapping the oneshot rx.
- **Client disconnect:** The client's IPC connection drops. The daemon detects this and resolves all pending oneshot channels with an error. Returns `"Shell unavailable — client disconnected"` to the LLM.
- **Queuing:** Commands queue naturally — the client processes `ExecuteCommand` messages sequentially through its single coprocess.

### Client reconnect

When a client reconnects:
1. The daemon matches the reconnecting client to an existing session (via session ID or client identity).
2. The Orchestrator's `ShellBackend::ClientRelay` channel handles are updated to point to the new client's IPC sender.
3. Any pending `ExecuteCommand` that timed out during disconnect has already been resolved with an error. The LLM may retry on the next turn.
4. The client sends a fresh `EnvSnapshot` on connect, which updates `session.cwd` and `session.env_vars`.
5. No state reconciliation is needed — the client's coprocess is the source of truth and it was gone during the disconnect.

## Client-Side Event Loop (ExecuteCommand Handling)

When the client's event loop receives a `DaemonMessage::ExecuteCommand`:

1. Extracts `command`, `execution_id`, and `timeout_ms` from the message.
2. Runs the command through the existing `ShellSession<PtyIO>` coprocess (same path as user commands).
3. After execution completes, calls `capture_cwd()` to get the new working directory.
4. Constructs `ClientMessage::CommandExecutionResult { execution_id, output, exit_code, cwd }`.
5. Sends the result back to the daemon via the IPC connection.

This reuses the existing coprocess execution path — no new execution logic on the client side, just a new message handler that delegates to the same `ShellSession` methods.

## CWD Tracking

### Shared shell (Orchestrator)
The coprocess is the source of truth. After each relay round-trip, the shared `Arc<RwLock<String>>` cwd is updated from `CommandExecutionResult.cwd`, and `session.cwd` is kept in sync.

### Independent agents (DaemonShell)
After each `execute()` call, `capture_cwd()` is called and the result written to the shared `Arc<RwLock<String>>`.

### Other tools (glob, grep)
These read cwd from the shared `Arc<RwLock<String>>`. Since tool calls from a single LLM turn are serialized by the aisdk tool loop (not concurrent), there is no contention between a bash write and a glob/grep read. The `RwLock` is a safety measure, not a concurrency bottleneck.

## Disposition of codiv-tools/src/tools/bash.rs

The current `bash::execute()` function (one-shot subprocess via `std::process::Command`) is **removed** from the tool path. It is no longer called by `build_tools()`.

The file is either:
- **Deleted** if no other code depends on it, or
- **Kept as a utility** renamed to `bash::execute_oneshot()` for cases where a quick fire-and-forget command is needed outside the agent tool loop (e.g., daemon startup scripts). This is a minor decision for implementation time.

The bash tool in `build_tools()` is entirely replaced by the `ShellBackend` routing logic.

## File Changes

### New code
- `codiv-common/src/shell.rs` — `ShellIO` trait, `ShellSession<IO>`, sentinel protocol, cwd/env capture, `CommandOutput` struct
- `codiv-common/src/shell/pipe_io.rs` — `PipeIO` implementation (clean output, separate stderr thread)
- `codiv-common/src/shell/pty_io.rs` — `PtyIO` implementation (ANSI stripping, `\r` removal)
- `codivd/src/daemon_shell.rs` — `DaemonShell` struct, spawn/drop lifecycle, `Mutex`-guarded `ShellSession<PipeIO>`

### Modified code
- `references/aisdk/src/core/tools.rs` — Change `ToolList::execute` from `tokio::spawn` to `tokio::task::spawn_blocking` for tool closures
- `codiv/src/shell/bash_coprocess.rs` — Refactored to use `ShellSession<PtyIO>`, becomes thin PTY spawn wrapper. PTY resize stays here.
- `codiv-common/src/messages.rs` — Add `ExecuteCommand` to `DaemonMessage`, add `CommandExecutionResult` to `ClientMessage`
- `codivd/src/agent/tools.rs` — `build_tools()` takes `ShellBackend` + `Arc<RwLock<String>>` for cwd. Bash tool routes through relay or daemon shell. Glob/grep read cwd from the shared lock.
- `codiv-tools/src/tools/bash.rs` — Removed from tool path (kept as utility or deleted)
- `codivd/src/daemon.rs` — Handles `CommandExecutionResult` messages, manages `pending_executions` map, updates `ClientRelay` handles on reconnect, creates `DaemonShell` for independent agents
- `codiv/src/ui/terminal/event_loop.rs` — Handles `ExecuteCommand` from daemon, runs through coprocess, sends results back

### Unchanged
- `codiv-tools/src/tools/` (read, write, edit)
- Session persistence, SQLite store
- LLM streaming, agent history
