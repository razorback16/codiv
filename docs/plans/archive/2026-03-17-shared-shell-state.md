# Shared Shell State Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Unify shell state between user and orchestrator agent via command relay through the client coprocess, with independent daemon-side shells for background agents.

**Architecture:** Two shell paths. Shared shell (User + Orchestrator) routes AI bash commands through the client's PTY coprocess via IPC relay. Independent agents get daemon-side pipe-based persistent bash shells. A shared `ShellIO` trait + `ShellSession<IO>` generic provides code reuse for the sentinel protocol across both backends.

**Tech Stack:** Rust, tokio, portable-pty, bincode, crossbeam-channel, Unix pipes

**Spec:** `docs/superpowers/specs/2026-03-16-shared-shell-state-design.md`

---

## File Structure

### New files
| File | Responsibility |
|------|---------------|
| `crates/codiv-common/src/shell/mod.rs` | `ShellIO` trait, `ShellSession<IO>`, `CommandOutput`, sentinel protocol |
| `crates/codiv-common/src/shell/pipe_io.rs` | `PipeIO` struct implementing `ShellIO` for stdin/stdout pipes |
| `crates/codiv-common/src/shell/pty_io.rs` | `PtyIO` struct implementing `ShellIO` for portable-pty, includes ANSI stripping |
| `crates/codivd/src/daemon_shell.rs` | `DaemonShell` — spawns pipe-based `bash -i`, wraps `ShellSession<PipeIO>` |
| `crates/codivd/src/agent/shell_backend.rs` | `ShellBackend` enum, `CommandExecutionPending` map |

### Modified files
| File | Changes |
|------|---------|
| `crates/codiv-common/src/lib.rs:1-6` | Add `pub mod shell;` |
| `crates/codiv-common/src/messages.rs:75-133` | Add `ExecuteCommand` variant to `DaemonMessage`, `CommandExecutionResult` to `ClientMessage` |
| `crates/codiv/src/ipc/messages.rs:9-115` | Add `build_command_execution_result` helper |
| `crates/codiv/src/shell/bash_coprocess.rs` | Refactor to use `ShellSession<PtyIO>`, keep PTY spawn + resize |
| `crates/codivd/src/agent/tools.rs:33-90` | `build_tools` takes `ShellBackend` + `Arc<RwLock<String>>`, routes bash through backend |
| `crates/codivd/src/agent/mod.rs` | Add `pub mod shell_backend;` |
| `crates/codivd/src/daemon.rs:198-652` | Handle `CommandExecutionResult`, manage pending executions, wire `ShellBackend` |
| `crates/codiv/src/ui/terminal/event_loop.rs:263-273` | Handle `ExecuteCommand` from daemon |
| `crates/codiv/src/ui/terminal/daemon.rs:169-544` | Add `ExecuteCommand` match arm in `handle_single_message` |

---

## Chunk 1: Foundation — ShellIO trait, ShellSession, PipeIO

### Task 1: Add `shell` module to codiv-common with ShellIO trait and CommandOutput

**Files:**
- Create: `crates/codiv-common/src/shell/mod.rs`
- Modify: `crates/codiv-common/src/lib.rs:1-6`

- [ ] **Step 1: Create the shell module with trait and types**

```rust
// crates/codiv-common/src/shell/mod.rs
pub mod pipe_io;

use std::io;

/// Output from a shell command execution.
#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub stdout: String,
    pub exit_code: i32,
    pub stderr: Option<String>,
}

/// Abstraction over shell I/O backends (PTY vs pipes).
/// Implementations handle backend-specific output cleaning
/// (e.g., ANSI stripping for PTY, raw passthrough for pipes).
pub trait ShellIO: Send {
    /// Write a command string to the shell's stdin.
    fn write_command(&mut self, cmd: &str) -> io::Result<()>;

    /// Read output until the given sentinel string is found.
    /// Returns the output text with the sentinel line removed.
    /// Implementations should handle backend-specific cleaning
    /// (e.g., \r stripping and ANSI removal for PTY).
    fn read_until_sentinel(&mut self, sentinel: &str) -> io::Result<String>;
}

/// Generate a unique sentinel string for command boundary detection.
pub fn generate_sentinel() -> String {
    let a = rand::random::<u32>();
    let b = rand::random::<u32>();
    format!("__CODIV_SENTINEL_{a:08x}{b:08x}_")
}

/// Find the expanded sentinel in output text.
/// The shell expands `echo "{sentinel}${__CODIV_EXIT}__"` to
/// `{sentinel}{digit}__` where digit is the exit code.
/// Returns Some((position, exit_code)) if found.
pub fn find_expanded_sentinel(text: &str, sentinel: &str) -> Option<(usize, i32)> {
    // Search for sentinel followed by a digit (the exit code)
    let mut search_start = 0;
    while let Some(pos) = text[search_start..].find(sentinel) {
        let abs_pos = search_start + pos;
        let after = &text[abs_pos + sentinel.len()..];
        // Must be followed by a digit and "__"
        if let Some(first_char) = after.chars().next() {
            if first_char.is_ascii_digit() {
                // Find the "__" terminator
                if let Some(end_pos) = after.find("__") {
                    let code_str = &after[..end_pos];
                    if let Ok(code) = code_str.parse::<i32>() {
                        return Some((abs_pos, code));
                    }
                }
            }
        }
        search_start = abs_pos + sentinel.len();
    }
    None
}

/// Generic shell session that works with any ShellIO backend.
/// Provides the sentinel-based command execution protocol.
pub struct ShellSession<IO: ShellIO> {
    io: IO,
}

impl<IO: ShellIO> ShellSession<IO> {
    pub fn new(io: IO) -> Self {
        Self { io }
    }

    /// Execute a command and return its output with exit code.
    pub fn execute(&mut self, command: &str) -> Result<CommandOutput, String> {
        let sentinel = generate_sentinel();

        // Wrap command with sentinel echo for boundary detection
        let wrapped = format!(
            "{command}; __CODIV_EXIT=$?; echo \"{sentinel}${{__CODIV_EXIT}}__\"\n"
        );

        self.io
            .write_command(&wrapped)
            .map_err(|e| format!("write error: {e}"))?;

        let raw_output = self.io
            .read_until_sentinel(&sentinel)
            .map_err(|e| format!("read error: {e}"))?;

        // Parse exit code from sentinel
        let (sentinel_pos, exit_code) = find_expanded_sentinel(&raw_output, &sentinel)
            .ok_or_else(|| "sentinel not found in output".to_string())?;

        // Extract output before the sentinel line
        let output = raw_output[..sentinel_pos].trim_end_matches('\n').to_string();

        Ok(CommandOutput {
            stdout: output,
            exit_code,
            stderr: None, // Set by caller for PipeIO backend
        })
    }

    /// Capture the current working directory.
    pub fn capture_cwd(&mut self) -> Option<String> {
        self.execute("pwd").ok().map(|o| o.stdout.trim().to_string())
    }

    /// Capture all environment variables.
    pub fn capture_env(&mut self) -> Vec<(String, String)> {
        let output = match self.execute("env") {
            Ok(o) => o.stdout,
            Err(_) => return Vec::new(),
        };
        output
            .lines()
            .filter_map(|line| {
                let (key, value) = line.split_once('=')?;
                Some((key.to_string(), value.to_string()))
            })
            .collect()
    }

    /// Get mutable access to the underlying IO for backend-specific operations.
    pub fn io_mut(&mut self) -> &mut IO {
        &mut self.io
    }
}
```

- [ ] **Step 2: Add `pub mod shell;` to codiv-common lib.rs**

Add `pub mod shell;` to `crates/codiv-common/src/lib.rs` after the existing module declarations.

- [ ] **Step 3: Add `rand` dependency to codiv-common Cargo.toml**

Add `rand = "0.8"` to `[dependencies]` in `crates/codiv-common/Cargo.toml`.

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p codiv-common`
Expected: Compiles successfully (PipeIO module is empty, that's fine — it compiles as `pub mod pipe_io;` with an empty file).

- [ ] **Step 5: Commit**

```bash
git add crates/codiv-common/src/shell/ crates/codiv-common/src/lib.rs crates/codiv-common/Cargo.toml
git commit -m "Add ShellIO trait and ShellSession for shared shell protocol"
```

---

### Task 2: Implement PipeIO backend

**Files:**
- Create: `crates/codiv-common/src/shell/pipe_io.rs`
- Test: inline in `crates/codiv-common/src/shell/pipe_io.rs`

- [ ] **Step 1: Write a test for PipeIO sentinel protocol**

```rust
// At bottom of crates/codiv-common/src/shell/pipe_io.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::ShellSession;

    #[test]
    fn test_pipe_execute_echo() {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "bash".into());
        let mut child = std::process::Command::new(&shell)
            .arg("-i")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .env("PS1", "")
            .env("HISTFILE", "/dev/null")
            .spawn()
            .expect("failed to spawn bash");

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        let pipe_io = PipeIO::new(stdin, stdout, stderr);
        let mut session: ShellSession<PipeIO> = ShellSession::new(pipe_io);

        // Drain any startup output
        let _ = session.execute("true");

        let result = session.execute("echo hello").unwrap();
        assert_eq!(result.stdout.trim(), "hello");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_pipe_execute_exit_code() {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "bash".into());
        let mut child = std::process::Command::new(&shell)
            .arg("-i")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .env("PS1", "")
            .env("HISTFILE", "/dev/null")
            .spawn()
            .expect("failed to spawn bash");

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        let pipe_io = PipeIO::new(stdin, stdout, stderr);
        let mut session: ShellSession<PipeIO> = ShellSession::new(pipe_io);

        let _ = session.execute("true");

        let result = session.execute("exit 42").expect("execute failed");
        assert_eq!(result.exit_code, 42);
    }

    #[test]
    fn test_pipe_capture_cwd() {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "bash".into());
        let mut child = std::process::Command::new(&shell)
            .arg("-i")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .env("PS1", "")
            .env("HISTFILE", "/dev/null")
            .spawn()
            .expect("failed to spawn bash");

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        let pipe_io = PipeIO::new(stdin, stdout, stderr);
        let mut session: ShellSession<PipeIO> = ShellSession::new(pipe_io);

        let _ = session.execute("true");
        let _ = session.execute("cd /tmp");
        let cwd = session.capture_cwd();
        // /tmp may resolve to /private/tmp on macOS
        assert!(cwd.unwrap().contains("tmp"), "expected /tmp in cwd");
    }

    #[test]
    fn test_pipe_cd_persists() {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "bash".into());
        let mut child = std::process::Command::new(&shell)
            .arg("-i")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .env("PS1", "")
            .env("HISTFILE", "/dev/null")
            .spawn()
            .expect("failed to spawn bash");

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        let pipe_io = PipeIO::new(stdin, stdout, stderr);
        let mut session: ShellSession<PipeIO> = ShellSession::new(pipe_io);

        let _ = session.execute("true");
        let _ = session.execute("cd /tmp");
        let result = session.execute("pwd").unwrap();
        assert!(result.stdout.trim().contains("tmp"));
    }

    #[test]
    fn test_pipe_env_persists() {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "bash".into());
        let mut child = std::process::Command::new(&shell)
            .arg("-i")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .env("PS1", "")
            .env("HISTFILE", "/dev/null")
            .spawn()
            .expect("failed to spawn bash");

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        let pipe_io = PipeIO::new(stdin, stdout, stderr);
        let mut session: ShellSession<PipeIO> = ShellSession::new(pipe_io);

        let _ = session.execute("true");
        let _ = session.execute("export MY_TEST_VAR=hello123");
        let result = session.execute("echo $MY_TEST_VAR").unwrap();
        assert_eq!(result.stdout.trim(), "hello123");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p codiv-common shell::pipe_io -- --nocapture`
Expected: FAIL — `PipeIO` struct not defined yet.

- [ ] **Step 3: Implement PipeIO**

```rust
// crates/codiv-common/src/shell/pipe_io.rs
use std::io::{self, BufRead, BufReader, Read, Write};
use std::process::{ChildStdin, ChildStdout, ChildStderr};
use std::sync::{Arc, Mutex};

use super::ShellIO;

/// Shell I/O backend using stdin/stdout/stderr pipes.
/// Output is clean (no ANSI escapes, no \r) since we disable echo.
pub struct PipeIO {
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    /// Stderr is drained by a background thread into this buffer.
    pub stderr_buf: Arc<Mutex<String>>,
    _stderr_handle: Option<std::thread::JoinHandle<()>>,
}

impl PipeIO {
    pub fn new(stdin: ChildStdin, stdout: ChildStdout, stderr: ChildStderr) -> Self {
        let stderr_buf = Arc::new(Mutex::new(String::new()));
        let buf_clone = Arc::clone(&stderr_buf);

        // Background thread to drain stderr continuously
        let stderr_handle = std::thread::Builder::new()
            .name("pipe-stderr-drain".into())
            .spawn(move || {
                let mut reader = BufReader::new(stderr);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line) {
                        Ok(0) => break, // EOF
                        Ok(_) => {
                            if let Ok(mut buf) = buf_clone.lock() {
                                buf.push_str(&line);
                            }
                        }
                        Err(_) => break,
                    }
                }
            })
            .ok();

        Self {
            stdin,
            reader: BufReader::new(stdout),
            stderr_buf,
            _stderr_handle: stderr_handle,
        }
    }

    /// Take and clear accumulated stderr output.
    pub fn take_stderr(&self) -> String {
        let mut buf = self.stderr_buf.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *buf)
    }
}

impl ShellIO for PipeIO {
    fn write_command(&mut self, cmd: &str) -> io::Result<()> {
        self.stdin.write_all(cmd.as_bytes())?;
        self.stdin.flush()
    }

    fn read_until_sentinel(&mut self, sentinel: &str) -> io::Result<String> {
        let mut output = String::new();
        let mut line = String::new();

        loop {
            line.clear();
            let bytes_read = self.reader.read_line(&mut line)?;
            if bytes_read == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "shell process exited"));
            }
            output.push_str(&line);

            // Check if we've seen the expanded sentinel
            if super::find_expanded_sentinel(&output, sentinel).is_some() {
                return Ok(output);
            }
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p codiv-common shell::pipe_io -- --nocapture`
Expected: All 5 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/codiv-common/src/shell/pipe_io.rs
git commit -m "Implement PipeIO backend for pipe-based shell sessions"
```

---

### Task 3: Implement DaemonShell

**Files:**
- Create: `crates/codivd/src/daemon_shell.rs`
- Modify: `crates/codivd/src/main.rs` or `crates/codivd/src/lib.rs` (add `pub mod daemon_shell;`)
- Test: inline in `crates/codivd/src/daemon_shell.rs`

- [ ] **Step 1: Write tests for DaemonShell**

```rust
// At bottom of crates/codivd/src/daemon_shell.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daemon_shell_execute() {
        let shell = DaemonShell::spawn("/tmp").expect("failed to spawn");
        let result = shell.execute("echo hello").expect("execute failed");
        assert_eq!(result.stdout.trim(), "hello");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_daemon_shell_cwd_persists() {
        let shell = DaemonShell::spawn("/").expect("failed to spawn");
        shell.execute("cd /tmp").expect("cd failed");
        let cwd = shell.capture_cwd().expect("capture_cwd failed");
        assert!(cwd.contains("tmp"), "expected /tmp, got: {cwd}");
    }

    #[test]
    fn test_daemon_shell_env_persists() {
        let shell = DaemonShell::spawn("/tmp").expect("failed to spawn");
        shell.execute("export FOO=bar123").expect("export failed");
        let result = shell.execute("echo $FOO").expect("echo failed");
        assert_eq!(result.stdout.trim(), "bar123");
    }

    #[test]
    fn test_daemon_shell_exit_code() {
        let shell = DaemonShell::spawn("/tmp").expect("failed to spawn");
        let result = shell.execute("false").expect("execute failed");
        assert_ne!(result.exit_code, 0);
    }

    #[test]
    fn test_daemon_shell_drop_kills_child() {
        let shell = DaemonShell::spawn("/tmp").expect("failed to spawn");
        let pid = shell.child_pid();
        drop(shell);
        // Give it a moment to clean up
        std::thread::sleep(std::time::Duration::from_millis(100));
        // Check process no longer exists (kill with signal 0 checks existence)
        let alive = unsafe { libc::kill(pid as i32, 0) } == 0;
        assert!(!alive, "child process should be dead after drop");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p codivd daemon_shell -- --nocapture`
Expected: FAIL — `DaemonShell` not defined.

- [ ] **Step 3: Implement DaemonShell**

```rust
// crates/codivd/src/daemon_shell.rs
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use codiv_common::shell::pipe_io::PipeIO;
use codiv_common::shell::{CommandOutput, ShellSession};

/// A persistent pipe-based bash shell for independent agents.
/// Survives client disconnects. Commands are serialized via Mutex.
pub struct DaemonShell {
    session: Mutex<ShellSession<PipeIO>>,
    child: Mutex<Child>,
}

impl DaemonShell {
    /// Spawn a new daemon shell with the given initial working directory.
    pub fn spawn(initial_cwd: &str) -> Result<Self, String> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "bash".into());
        let mut child = Command::new(&shell)
            .arg("-i")
            .current_dir(initial_cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("PS1", "")
            .env("HISTFILE", "/dev/null")
            .spawn()
            .map_err(|e| format!("failed to spawn daemon shell: {e}"))?;

        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let stderr = child.stderr.take().ok_or("no stderr")?;

        let pipe_io = PipeIO::new(stdin, stdout, stderr);
        let mut session = ShellSession::new(pipe_io);

        // Suppress echo and clear startup noise
        let _ = session.execute("stty -echo 2>/dev/null; true");

        // Clear stderr buffer (discard .bashrc noise)
        session.io_mut().take_stderr();

        Ok(Self {
            session: Mutex::new(session),
            child: Mutex::new(child),
        })
    }

    /// Execute a command in this shell. Thread-safe (serialized by Mutex).
    pub fn execute(&self, command: &str) -> Result<CommandOutput, String> {
        let mut session = self.session.lock()
            .map_err(|_| "session mutex poisoned".to_string())?;

        let mut result = session.execute(command)?;

        // Append any accumulated stderr
        let stderr = session.io_mut().take_stderr();
        if !stderr.is_empty() {
            result.stderr = Some(stderr);
        }

        Ok(result)
    }

    /// Capture the current working directory.
    pub fn capture_cwd(&self) -> Result<String, String> {
        let mut session = self.session.lock()
            .map_err(|_| "session mutex poisoned".to_string())?;
        session.capture_cwd().ok_or_else(|| "failed to capture cwd".to_string())
    }

    /// Get the child process PID (for testing).
    pub fn child_pid(&self) -> u32 {
        self.child.lock().unwrap().id()
    }
}

impl Drop for DaemonShell {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
```

- [ ] **Step 4: Add `pub mod daemon_shell;` to codivd**

Check whether codivd uses `main.rs` or `lib.rs` and add the module declaration. Look at `crates/codivd/src/` for the module structure.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p codivd daemon_shell -- --nocapture`
Expected: All 5 tests PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/codivd/src/daemon_shell.rs crates/codivd/src/main.rs
git commit -m "Add DaemonShell for independent agent persistent shells"
```

---

## Chunk 2: IPC Messages and aisdk Change

### Task 4: Add IPC message variants

**Files:**
- Modify: `crates/codiv-common/src/messages.rs:24-71` (ClientMessage), `crates/codiv-common/src/messages.rs:75-133` (DaemonMessage)
- Modify: `crates/codiv/src/ipc/messages.rs:9-115` (add builder)

- [ ] **Step 1: Add `ExecuteCommand` to DaemonMessage**

In `crates/codiv-common/src/messages.rs`, add to the `DaemonMessage` enum (after the `Notice` variant around line 133):

```rust
    ExecuteCommand {
        command: String,
        execution_id: String,
        timeout_ms: u64,
    },
```

- [ ] **Step 2: Add `CommandExecutionResult` to ClientMessage**

In `crates/codiv-common/src/messages.rs`, add to the `ClientMessage` enum (after the `CancelRequest` variant around line 71):

```rust
    CommandExecutionResult {
        execution_id: String,
        output: String,
        exit_code: i32,
        cwd: String,
    },
```

- [ ] **Step 3: Add builder function for CommandExecutionResult**

In `crates/codiv/src/ipc/messages.rs`, add after `build_cancel_request`:

```rust
pub fn build_command_execution_result(
    execution_id: &str,
    output: &str,
    exit_code: i32,
    cwd: &str,
) -> Option<Vec<u8>> {
    let msg = ClientMessage::CommandExecutionResult {
        execution_id: execution_id.to_string(),
        output: output.to_string(),
        exit_code,
        cwd: cwd.to_string(),
    };
    frame_message(&msg).ok()
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check`
Expected: Compiles. There will be warnings about unmatched `ExecuteCommand` in client-side match arms and `CommandExecutionResult` in daemon-side match arms — those are addressed in later tasks.

- [ ] **Step 5: Commit**

```bash
git add crates/codiv-common/src/messages.rs crates/codiv/src/ipc/messages.rs
git commit -m "Add ExecuteCommand and CommandExecutionResult IPC message variants"
```

---

### Task 5: No aisdk modification needed — use `block_in_place` in tool closures

**Note:** The original plan called for modifying aisdk's `ToolList::execute` from `tokio::spawn` to `tokio::task::spawn_blocking`. This is no longer necessary. Instead, tool closures that perform blocking I/O wrap their blocking work in `tokio::task::block_in_place(|| { ... })`, which safely yields the current tokio worker thread without requiring any upstream library changes. The `block_in_place` + `handle.block_on()` pattern is applied in `ShellBackend::execute` (Task 6).

---

## Chunk 3: ShellBackend and Tool Routing

### Task 6: Create ShellBackend enum and implement routing

**Files:**
- Create: `crates/codivd/src/agent/shell_backend.rs`
- Modify: `crates/codivd/src/agent/mod.rs:1-10`

- [ ] **Step 1: Create ShellBackend module**

```rust
// crates/codivd/src/agent/shell_backend.rs
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use codiv_common::messages::{ClientMessage, DaemonMessage, frame_message};
use codiv_common::shell::CommandOutput;
use codiv_common::shell::pipe_io::PipeIO;
use codiv_common::shell::ShellSession;
use codiv_common::truncate::truncate_output;

/// Result type sent back through the oneshot channel when the client
/// responds to an ExecuteCommand.
#[derive(Debug)]
pub struct RelayResult {
    pub output: String,
    pub exit_code: i32,
    pub cwd: String,
}

/// Backend for bash tool execution routing.
#[derive(Clone)]
pub enum ShellBackend {
    /// Route commands through the client's shared coprocess via IPC.
    ClientRelay {
        client_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
        pending: Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<RelayResult>>>>,
        handle: tokio::runtime::Handle,
    },
    /// Execute commands in a daemon-side persistent shell.
    Local {
        shell: Arc<crate::daemon_shell::DaemonShell>,
    },
}

impl ShellBackend {
    /// Execute a bash command through the appropriate backend.
    /// Uses block_in_place internally for blocking I/O.
    pub fn execute(
        &self,
        command: &str,
        timeout_ms: u64,
        cwd_ref: &Arc<RwLock<String>>,
    ) -> Result<String, String> {
        match self {
            ShellBackend::ClientRelay { client_tx, pending, handle } => {
                let execution_id = uuid::Uuid::new_v4().to_string();
                let (tx, rx) = tokio::sync::oneshot::channel();

                // Store the sender for when the daemon receives the result
                pending
                    .lock()
                    .map_err(|_| "pending mutex poisoned".to_string())?
                    .insert(execution_id.clone(), tx);

                let msg = DaemonMessage::ExecuteCommand {
                    command: command.to_string(),
                    execution_id: execution_id.clone(),
                    timeout_ms,
                };
                let frame = frame_message(&msg)
                    .map_err(|e| format!("frame error: {e}"))?;

                // block_in_place + block_on is safe here — yields the tokio worker thread
                let result = tokio::task::block_in_place(|| handle.block_on(async {
                    client_tx.send(frame).await
                        .map_err(|_| "client disconnected".to_string())?;

                    tokio::time::timeout(
                        Duration::from_millis(timeout_ms),
                        rx,
                    )
                    .await
                    .map_err(|_| format!("command timed out after {timeout_ms}ms"))?
                    .map_err(|_| "shell unavailable — client disconnected".to_string())
                }))?;

                // Update shared cwd
                if let Ok(mut cwd) = cwd_ref.write() {
                    *cwd = result.cwd;
                }

                Ok(format_output(&result.output, result.exit_code))
            }

            ShellBackend::Local { shell } => {
                let result = shell.execute(command)?;

                // Update shared cwd
                if let Ok(new_cwd) = shell.capture_cwd() {
                    if let Ok(mut cwd) = cwd_ref.write() {
                        *cwd = new_cwd;
                    }
                }

                let mut output = result.stdout;

                // Append stderr if present
                if let Some(ref stderr) = result.stderr {
                    if !stderr.is_empty() {
                        if !output.is_empty() && !output.ends_with('\n') {
                            output.push('\n');
                        }
                        output.push_str("<stderr>\n");
                        output.push_str(stderr);
                        if !stderr.ends_with('\n') {
                            output.push('\n');
                        }
                        output.push_str("</stderr>");
                    }
                }

                Ok(format_output(&output, result.exit_code))
            }
        }
    }

    /// Resolve a pending execution with the result from the client.
    /// Called by the daemon when it receives CommandExecutionResult.
    pub fn resolve_pending(
        pending: &Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<RelayResult>>>>,
        execution_id: &str,
        result: RelayResult,
    ) {
        if let Ok(mut map) = pending.lock() {
            if let Some(tx) = map.remove(execution_id) {
                let _ = tx.send(result);
            }
        }
    }

    /// Fail all pending executions (e.g., on client disconnect).
    pub fn fail_all_pending(
        pending: &Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<RelayResult>>>>,
    ) {
        if let Ok(mut map) = pending.lock() {
            map.clear(); // Dropping senders causes rx to get RecvError
        }
    }
}

fn format_output(output: &str, exit_code: i32) -> String {
    let mut result = output.to_string();

    if exit_code != 0 {
        if result.is_empty() {
            result = format!("failed with exit code: {exit_code}");
        } else {
            if !result.ends_with('\n') {
                result.push('\n');
            }
            result.push_str(&format!("exit code: {exit_code}"));
        }
    } else if result.is_empty() {
        result = format!("success, exit code: {exit_code}");
    }

    truncate_output(&result, 200, 100)
}
```

- [ ] **Step 2: Add `pub mod shell_backend;` to agent/mod.rs**

In `crates/codivd/src/agent/mod.rs`, add `pub mod shell_backend;`.

- [ ] **Step 3: Add uuid dependency to codivd Cargo.toml if not present**

Check `crates/codivd/Cargo.toml` — uuid is likely already there (used for session IDs in daemon.rs). If not, add `uuid = { version = "1", features = ["v4"] }`.

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p codivd`
Expected: Compiles successfully.

- [ ] **Step 5: Commit**

```bash
git add crates/codivd/src/agent/shell_backend.rs crates/codivd/src/agent/mod.rs
git commit -m "Add ShellBackend enum for routing bash commands through client relay or daemon shell"
```

---

### Task 7: Modify build_tools to use ShellBackend

**Files:**
- Modify: `crates/codivd/src/agent/tools.rs:33-90`

- [ ] **Step 1: Change build_tools signature and bash tool routing**

Replace the current `build_tools` function in `crates/codivd/src/agent/tools.rs`. The new version takes a `ShellBackend` and `Arc<RwLock<String>>` instead of `cwd: String` and `env_vars: Vec<(String, String)>`.

The bash tool closure now calls `backend.execute(command, timeout_ms, &cwd_ref)` instead of `bash::execute(v, &cwd, &env_vars)`.

The glob and grep tools read cwd from `cwd_ref.read()` instead of capturing a static `Arc<String>`.

```rust
// crates/codivd/src/agent/tools.rs
use std::sync::{Arc, RwLock};

use aisdk::core::tools::{Tool, ToolExecute};
use serde_json::Value;

use codiv_tools::tools::{edit, glob, grep, read, write};
use codiv_tools::tools::bash::BashInput;

use super::permissions::PermissionContext;
use super::shell_backend::ShellBackend;

fn make_tool_with_permissions<T: schemars::JsonSchema>(
    name: &str,
    description: &str,
    execute: impl Fn(Value) -> Result<String, String> + Send + Sync + 'static,
    permission_ctx: Option<&Arc<PermissionContext>>,
) -> Tool {
    let boxed: Box<dyn Fn(Value) -> Result<String, String> + Send + Sync> = Box::new(execute);
    let final_execute = match permission_ctx {
        Some(ctx) => super::permissions::wrap_with_permissions(
            name.to_string(),
            boxed,
            Arc::clone(ctx),
        ),
        None => boxed,
    };
    Tool {
        name: name.to_string(),
        description: description.to_string(),
        input_schema: schemars::schema_for!(T),
        execute: ToolExecute::new(final_execute),
    }
}

pub fn build_tools(
    backend: ShellBackend,
    cwd_ref: Arc<RwLock<String>>,
    permission_ctx: Option<Arc<PermissionContext>>,
) -> Vec<Tool> {
    let pctx = &permission_ctx;

    vec![
        {
            let backend = backend.clone();
            let cwd_ref = Arc::clone(&cwd_ref);
            make_tool_with_permissions::<BashInput>(
                "bash",
                "Execute a bash command and return its output. Use for running shell commands, installing packages, running tests, etc.",
                move |v| {
                    let input: BashInput = serde_json::from_value(v)
                        .map_err(|e| format!("invalid bash input: {e}"))?;
                    backend.execute(&input.command, input.timeout_ms, &cwd_ref)
                },
                pctx.as_ref(),
            )
        },
        make_tool_with_permissions::<read::ReadInput>(
            "read",
            "Read the contents of a file. Returns numbered lines. Use offset and limit for large files.",
            read::execute,
            pctx.as_ref(),
        ),
        make_tool_with_permissions::<write::WriteInput>(
            "write",
            "Write content to a file. Creates parent directories if needed. Overwrites existing content.",
            write::execute,
            pctx.as_ref(),
        ),
        make_tool_with_permissions::<edit::EditInput>(
            "edit",
            "Replace a unique string in a file. The old_string must appear exactly once in the file.",
            edit::execute,
            pctx.as_ref(),
        ),
        {
            let cwd_ref = Arc::clone(&cwd_ref);
            make_tool_with_permissions::<glob::GlobInput>(
                "glob",
                "Find files matching a glob pattern. Returns sorted list of file paths.",
                move |v| {
                    let cwd = cwd_ref.read()
                        .map_err(|_| "cwd lock poisoned".to_string())?
                        .clone();
                    glob::execute(v, &cwd)
                },
                pctx.as_ref(),
            )
        },
        {
            let cwd_ref = Arc::clone(&cwd_ref);
            make_tool_with_permissions::<grep::GrepInput>(
                "grep",
                "Search file contents using regex. Returns matching lines with file paths and line numbers.",
                move |v| {
                    let cwd = cwd_ref.read()
                        .map_err(|_| "cwd lock poisoned".to_string())?
                        .clone();
                    grep::execute(v, &cwd)
                },
                pctx.as_ref(),
            )
        },
    ]
}
```

- [ ] **Step 2: Update agent.rs run_streaming to use new build_tools signature**

In `crates/codivd/src/agent/agent.rs`, the `run_streaming` method (line 246) currently calls:
```rust
let tools = super::tools::build_tools(self.cwd.clone(), self.env_vars.clone(), permission_ctx);
```

This needs to change. The agent needs a `ShellBackend` and `Arc<RwLock<String>>` passed in. Add these as fields to `Agent`:

In `crates/codivd/src/agent/agent.rs`, modify the `Agent` struct (lines 34-42):
- Add field: `pub shell_backend: Option<ShellBackend>`
- Add field: `pub cwd_ref: Arc<RwLock<String>>`

Update `Agent::new` to initialize `cwd_ref` from the `cwd` parameter:
```rust
cwd_ref: Arc::new(RwLock::new(cwd.clone())),
shell_backend: None, // Set by daemon before run_streaming
```

Update `run_streaming` (line 246-249):
```rust
let backend = self.shell_backend.clone()
    .expect("shell_backend must be set before run_streaming");
let tools = super::tools::build_tools(
    backend,
    Arc::clone(&self.cwd_ref),
    permission_ctx,
);
```

- [ ] **Step 3: Update daemon.rs to set shell_backend on agent before streaming**

In `crates/codivd/src/daemon.rs`, before the `tokio::spawn` that calls `agent.run_streaming()` (around line 296), set the shell backend:

```rust
// Create the shell backend for this agent
let client_tx_clone = client_tx.clone();
let pending_executions = session.pending_executions
    .get_or_insert_with(|| Arc::new(Mutex::new(HashMap::new())))
    .clone();
let shell_backend = ShellBackend::ClientRelay {
    client_tx: client_tx_clone,
    pending: pending_executions,
    handle: tokio::runtime::Handle::current(),
};
agent.shell_backend = Some(shell_backend);
```

Also add `pending_executions: Option<Arc<Mutex<HashMap<String, oneshot::Sender<RelayResult>>>>>` to `ClientSession` in `crates/codivd/src/session.rs`.

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p codivd`
Expected: Compiles. There may be unused import warnings for `bash` in tools.rs — remove the `bash` import from the `use codiv_tools::tools::{bash, edit, glob, grep, read, write};` line (keep `BashInput` import via `use codiv_tools::tools::bash::BashInput;`).

- [ ] **Step 5: Commit**

```bash
git add crates/codivd/src/agent/tools.rs crates/codivd/src/agent/agent.rs crates/codivd/src/daemon.rs crates/codivd/src/session.rs
git commit -m "Route bash tool through ShellBackend instead of one-shot subprocess"
```

---

## Chunk 4: Client-Side Relay and Daemon Dispatch

### Task 8: Handle ExecuteCommand on the client side

**Files:**
- Modify: `crates/codiv/src/ui/terminal/daemon.rs:169-544` (add match arm)
- Modify: `crates/codiv/src/ui/terminal/event_loop.rs` (execute through coprocess)

- [ ] **Step 1: Add ExecuteCommand match arm in daemon.rs handle_single_message**

In `crates/codiv/src/ui/terminal/daemon.rs`, in the `handle_single_message` function, add a match arm for `DaemonMessage::ExecuteCommand`. This should be near the other match arms (around line 169+).

The handler needs to:
1. Run the command through the bash coprocess
2. Capture the cwd after execution
3. Send `CommandExecutionResult` back to the daemon

Since `handle_single_message` doesn't have direct access to the coprocess, the approach is:
- Store the pending execution request on `TerminalState` as a new field `pending_ai_command: Option<PendingAiCommand>`
- The event loop detects this and runs it through the coprocess (same pattern as user commands)

Add to `handle_single_message`:
```rust
DaemonMessage::ExecuteCommand { command, execution_id, timeout_ms: _ } => {
    // Queue this for execution through the coprocess.
    // The event loop will pick it up and run it.
    state.pending_ai_command = Some(PendingAiCommand {
        command,
        execution_id,
    });
}
```

Add struct definition (in the file or in the terminal state module):
```rust
pub struct PendingAiCommand {
    pub command: String,
    pub execution_id: String,
}
```

- [ ] **Step 2: Handle PendingAiCommand in the event loop**

In `crates/codiv/src/ui/terminal/event_loop.rs`, after handling daemon messages, check for `pending_ai_command`:

```rust
// After daemon message handling, check for AI command requests
if state.pending_command.is_none() {
    if let Some(ai_cmd) = state.pending_ai_command.take() {
        // Start command through coprocess (same as user commands)
        if let Some(sentinel) = bash.start_command(&ai_cmd.command) {
            state.pending_command = Some(PendingCommand {
                command: ai_cmd.command,
                sentinel,
                accumulated: String::new(),
                needs_env_refresh: false,
                ai_execution_id: Some(ai_cmd.execution_id),
            });
        }
    }
}
```

Add `ai_execution_id: Option<String>` field to `PendingCommand` (default `None` for user commands).

- [ ] **Step 3: Send CommandExecutionResult when AI command completes**

In the command completion handler (around lines 309-375 of `event_loop.rs`), check if this was an AI-relayed command:

```rust
// After command completes and result is available:
if let Some(execution_id) = pending.ai_execution_id.take() {
    // This was an AI-relayed command — send result back to daemon
    let cwd = bash.capture_cwd().unwrap_or_default();
    if let Some(frame) = ipc_messages::build_command_execution_result(
        &execution_id,
        &result.output,
        result.exit_code,
        &cwd,
    ) {
        state.ipc_client.as_ref().map(|c| c.send(&frame));
    }
} else {
    // This was a user command — existing behavior (send CommandResult to daemon)
    // ... existing code ...
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p codiv`
Expected: Compiles successfully.

- [ ] **Step 5: Commit**

```bash
git add crates/codiv/src/ui/terminal/daemon.rs crates/codiv/src/ui/terminal/event_loop.rs
git commit -m "Handle ExecuteCommand from daemon by routing through client coprocess"
```

---

### Task 9: Handle CommandExecutionResult in daemon dispatch

**Files:**
- Modify: `crates/codivd/src/daemon.rs:198-652` (add match arm in dispatch)

- [ ] **Step 1: Add CommandExecutionResult handler**

In `crates/codivd/src/daemon.rs`, in the `dispatch` method, add a match arm for `ClientMessage::CommandExecutionResult`:

```rust
ClientMessage::CommandExecutionResult {
    execution_id,
    output,
    exit_code,
    cwd,
} => {
    tracing::info!("received command execution result: {}", execution_id);

    // Update session cwd
    if let Some(session) = self.sessions.get_mut(&client_id) {
        session.cwd = cwd.clone();
    }

    // Resolve the pending oneshot
    if let Some(session) = self.sessions.get(&client_id) {
        if let Some(ref pending) = session.pending_executions {
            ShellBackend::resolve_pending(
                pending,
                &execution_id,
                RelayResult { output, exit_code, cwd },
            );
        }
    }
}
```

- [ ] **Step 2: Add imports to daemon.rs**

Add necessary imports at the top of `daemon.rs`:
```rust
use crate::agent::shell_backend::{ShellBackend, RelayResult};
use std::sync::Mutex;
use std::collections::HashMap;
```

- [ ] **Step 3: Handle client disconnect — fail pending executions**

In the `cleanup_stale_sessions` method (around line 728), before removing the session, fail any pending executions:

```rust
for id in stale {
    info!("cleaning stale session {}", id);
    if let Some(session) = self.sessions.get(&id) {
        if let Some(ref pending) = session.pending_executions {
            ShellBackend::fail_all_pending(pending);
        }
    }
    self.sessions.remove(&id);
    self.ipc.disconnect(id);
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p codivd`
Expected: Compiles successfully.

- [ ] **Step 5: Run full project build**

Run: `cargo build`
Expected: Full project builds with no errors.

- [ ] **Step 6: Commit**

```bash
git add crates/codivd/src/daemon.rs
git commit -m "Handle CommandExecutionResult in daemon dispatch, resolve pending relay channels"
```

---

## Chunk 5: Integration Testing

### Task 10: End-to-end integration tests

**Files:**
- Test: `crates/codivd/src/daemon_shell.rs` (already has unit tests)
- Test: `crates/codiv-common/src/shell/pipe_io.rs` (already has unit tests)
- Verify: full cargo test suite

- [ ] **Step 1: Run all unit tests**

Run: `cargo test --workspace`
Expected: All tests pass. Pay special attention to:
- `codiv_common::shell::pipe_io::tests` — PipeIO sentinel protocol
- `codivd::daemon_shell::tests` — DaemonShell lifecycle
- `codiv_tools::tools::bash::tests` — Existing bash tool tests (should still pass since bash.rs is kept as utility)
- `codiv::shell::bash_coprocess::tests` — Existing coprocess tests (should still pass if refactor preserved behavior)

- [ ] **Step 2: Run aisdk tests**

Run: `cargo test -p aisdk`
Expected: All tests pass (aisdk is unmodified; blocking handled by block_in_place in codivd).

- [ ] **Step 3: Build release to catch any optimization-only issues**

Run: `cargo build --release`
Expected: Builds successfully.

- [ ] **Step 4: Manual smoke test**

If possible, run the codiv TUI and test:
1. Start codiv, connect to daemon
2. Ask the AI to run `cd /tmp && pwd` — verify it outputs `/tmp` (or `/private/tmp` on macOS)
3. Ask the AI to run `pwd` again — verify it still shows `/tmp` (state persists)
4. Ask the AI to run `export MY_VAR=test123 && echo $MY_VAR` — verify it outputs `test123`
5. Run `pwd` in the user's terminal — verify it also shows `/tmp` (shared state)

- [ ] **Step 5: Commit any test fixes**

If any tests needed adjustment:
```bash
git add -A
git commit -m "Fix tests for shared shell state integration"
```
