# Architecture

**Analysis Date:** 2026-03-31

## Pattern Overview

**Overall:** Two-process client-daemon architecture with IPC over Unix domain sockets

**Key Characteristics:**
- `codiv` (TUI client) and `codivd` (daemon) are separate binaries communicating over a Unix socket
- The TUI client stays responsive because all AI/LLM work runs in the daemon's async tokio runtime
- The daemon holds all AI state (conversation history, agent, sessions); the client is stateless for AI purposes
- Shell commands execute in a PTY co-process owned by the TUI client, with the daemon relaying execution requests back to the client when the agent needs to run bash
- IPC uses serde + bincode with 4-byte big-endian length-prefixed framing

## Layers

**TUI Client (`codiv`):**
- Purpose: User-facing terminal interface, shell management, IPC relay
- Location: `crates/codiv/src/`
- Contains: `app.rs` (startup orchestrator), `ui/` (ratatui rendering), `shell/` (PTY/bash coprocess), `ipc/` (IPC client), `cli/` (direct tool dispatch subcommands)
- Depends on: `codiv-common` (shared types and IPC framing), `codiv-tools` (for direct CLI invocation)
- Used by: end users directly

**Daemon (`codivd`):**
- Purpose: AI agent loop, session persistence, permission gating
- Location: `crates/codivd/src/`
- Contains: `daemon.rs` (event loop), `agent/` (LLM agent, tools, permissions), `ipc/` (Unix socket server), `session.rs` (per-client state), `store.rs` (SQLite persistence)
- Depends on: `codiv-common`, `codiv-tools`, `aisdk` (LLM abstraction)
- Used by: `codiv` TUI via IPC

**Shared Library (`codiv-common`):**
- Purpose: Shared types, IPC message definitions, config paths
- Location: `crates/codiv-common/src/`
- Contains: `messages.rs` (IPC enums `ClientMessage`/`DaemonMessage`/`StreamChunk`), `conversation.rs` (`ConversationEvent` enum), `types.rs` (`AgentRole`), `permissions.rs` (`PermissionMode`/`PermissionDecision`), `config.rs` (path helpers)
- Depends on: serde, bincode
- Used by: both `codiv` and `codivd`

**Tools Library (`codiv-tools`):**
- Purpose: Shared implementations of all agent tools (bash, read, write, edit, glob, grep)
- Location: `crates/codiv-tools/src/`
- Contains: `tools/bash.rs`, `tools/read.rs`, `tools/write.rs`, `tools/edit.rs`, `tools/glob.rs`, `tools/grep.rs`, `agent_guide.rs`
- Depends on: schemars (JSON schema for tool inputs)
- Used by: `codivd` (agent tool execution), `codiv` (direct CLI subcommand dispatch)

## Data Flow

**User Prompt (AI Mode):**

1. User types a prompt in `codiv` TUI, presses Enter
2. `crates/codiv/src/ui/terminal/input.rs` dispatches to agent mode
3. `CodivdClient` (`crates/codiv/src/ipc/client.rs`) sends `ClientMessage::AgentRequest` over Unix socket
4. `IpcServer` (`crates/codivd/src/ipc/server.rs`) receives and routes to `Daemon` event loop (`crates/codivd/src/daemon.rs`)
5. `Daemon` retrieves or creates a `ClientSession` (`crates/codivd/src/session.rs`) and spawns `Agent::run()` (`crates/codivd/src/agent/agent.rs`)
6. `Agent` calls aisdk LLM provider, streaming `DaemonMessage::AgentStreamChunk` back to the TUI
7. If the LLM calls a tool, `PermissionContext` evaluates risk (`crates/codivd/src/agent/permissions.rs`); may send `DaemonMessage::ConfirmationRequest` to the TUI for user approval
8. For bash tools: daemon sends `DaemonMessage::ExecuteCommand` to the TUI; TUI runs it in the PTY co-process (`crates/codiv/src/shell/bash_coprocess.rs`) and returns `ClientMessage::CommandExecutionResult`
9. Agent appends all events to `Vec<ConversationEvent>` history and `SessionStore` (`crates/codivd/src/store.rs`) SQLite DB
10. `DaemonMessage::AgentComplete` signals end of response

**Direct Shell Command:**

1. User types a command (non-AI mode)
2. `BashCoprocess` (`crates/codiv/src/shell/bash_coprocess.rs`) executes it via PTY
3. Output is rendered via `vt100::Parser` in the TUI
4. On completion, `ClientMessage::CommandResult` is sent to daemon to update conversation context

**Shell Backend (Bash Tool via Agent):**

- The `ShellBackend::ClientRelay` variant in `crates/codivd/src/agent/shell_backend.rs` implements a request-response relay: daemon sends `ExecuteCommand` to the client; client executes in PTY; client sends `CommandExecutionResult` back; daemon unblocks the waiting agent task

**State Management:**
- `TerminalState` (`crates/codiv/src/ui/terminal/state.rs`) is the single mutable state object for the TUI, holding pending commands, streaming buffers, block registry, token usage, and UI flags
- `ClientSession` (`crates/codivd/src/session.rs`) is the daemon-side per-client state, holding the `Agent`, permission context, and SQLite session ID
- `Agent` struct (`crates/codivd/src/agent/agent.rs`) holds conversation history as `Vec<ConversationEvent>` plus model config and shell backend reference

## Key Abstractions

**`ClientMessage` / `DaemonMessage` (IPC Protocol):**
- Purpose: Typed bidirectional IPC between TUI and daemon
- Location: `crates/codiv-common/src/messages.rs`
- Pattern: Serde enums serialized with bincode, framed with 4-byte BE length prefix via `frame_message()` / `parse_frame_header()`

**`ConversationEvent`:**
- Purpose: Unified event type for both in-memory agent history and SQLite persistence/replay
- Location: `crates/codiv-common/src/conversation.rs`
- Pattern: Enum with variants for `UserPrompt`, `AssistantText`, `ToolCall`, `ToolResult`, `ShellCommand`, `Summary`, `TokenUsage`, `Error`

**`Agent`:**
- Purpose: Wraps aisdk LLM client, maintains conversation history, streams responses
- Location: `crates/codivd/src/agent/agent.rs`
- Pattern: Struct with `history: Vec<ConversationEvent>`, `model_config`, `provider_config`, `shell_backend`

**`PermissionContext`:**
- Purpose: Per-session permission gating for tool calls
- Location: `crates/codivd/src/agent/permissions.rs`
- Pattern: Shared `Arc<PermissionContext>` with `RwLock<PermissionMode>` (Auto/Manual/Bypass) and risk classification pipeline (`risk_classifier.rs` → `permission_evaluator.rs` → optional `llm_evaluator.rs`)

**`BlockRegistry` / `Block`:**
- Purpose: TUI rendering abstraction for conversation turns (tool calls, prompts, responses)
- Location: `crates/codiv/src/ui/blocks.rs`
- Pattern: Registry of `ToolBlock`, `PromptBlock`, `CmdResponseBlock` each tracking scrollback index, height, and rendered ANSI lines

**`BashCoprocess`:**
- Purpose: Persistent bash session via PTY providing zero-latency shell execution
- Location: `crates/codiv/src/shell/bash_coprocess.rs`
- Pattern: `portable-pty`-based PTY, shared by both direct user commands and AI-relayed commands

**`SessionStore`:**
- Purpose: SQLite-backed persistence of sessions and conversation events
- Location: `crates/codivd/src/store.rs`
- Pattern: Synchronous `rusqlite` `Connection` on the daemon main task; `sessions` and `events` tables with WAL journal mode

**`ShellBackend`:**
- Purpose: Abstract bash execution for the agent (currently `ClientRelay` only)
- Location: `crates/codivd/src/agent/shell_backend.rs`
- Pattern: Enum allowing future backends; `ClientRelay` routes via IPC to the TUI's PTY

## Entry Points

**`codiv` TUI:**
- Location: `crates/codiv/src/main.rs`
- Triggers: User runs `codiv` (no subcommand)
- Responsibilities: Parse CLI args, init logging, set SIGTERM handler, call `app::run()`

**`codiv` Direct Tool CLI:**
- Location: `crates/codiv/src/main.rs` → `crates/codiv/src/cli/mod.rs`
- Triggers: User runs `codiv read <file>`, `codiv bash <cmd>`, etc.
- Responsibilities: Dispatch to `codiv-tools` implementations directly, bypassing permission checks

**`codivd` Daemon:**
- Location: `crates/codivd/src/main.rs` → `async_main()`
- Triggers: Service manager (launchd/systemd) or direct invocation
- Responsibilities: Fork/daemonize, write PID file, build tokio runtime, create and run `Daemon`

**`Daemon::run()` Event Loop:**
- Location: `crates/codivd/src/daemon.rs`
- Triggers: Called from `async_main()`
- Responsibilities: `tokio::select!` on new IPC connections, client messages, agent completion, name updates, config changes, cleanup interval, SIGTERM

**`event_loop()` TUI Loop:**
- Location: `crates/codiv/src/ui/terminal/event_loop.rs`
- Triggers: Called from `ui::terminal::run()`
- Responsibilities: Poll crossterm events (keyboard, mouse, resize), daemon messages, tick channel; dispatch to input/render handlers; manage daemon reconnection

## Error Handling

**Strategy:** `Result<T, E>` propagation with `Box<dyn std::error::Error>` at top-level boundaries; `tracing` macros for daemon logging; `log` crate for TUI debug logging

**Patterns:**
- IPC send failures return `bool` (silent on disconnect) allowing graceful degradation
- Agent errors surface as `DaemonMessage::Error` sent to TUI, displayed inline
- Daemon startup failures use `process::exit(1)` after logging
- `SessionStore` migration errors are fatal (daemon exits)
- Tool execution errors return `Result<String, String>` — error string becomes tool result content for the LLM

## Cross-Cutting Concerns

**Logging:**
- TUI (`codiv`): `env_logger` writing to `/tmp/codiv-debug.log`, activated with `--debug` flag
- Daemon (`codivd`): `tracing_subscriber` writing to `~/.codiv/codivd.log`, level controlled by `RUST_LOG` env var

**Validation:**
- Tool inputs are validated via `serde_json::from_value()` at call sites; invalid inputs return error strings to the LLM

**Authentication:**
- API keys sourced from env vars (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GOOGLE_API_KEY`) or `~/.codiv/config.toml`; env vars take precedence

**Config Hot-Reload:**
- `AppConfig` uses a file watcher that sends on `config_change_tx`; daemon reloads on next event loop iteration without restart

**Session Compaction:**
- When `input_tokens` exceeds threshold, `needs_compaction` is set on `ClientSession`; next idle cycle triggers `CompactRequest` which runs a summarization LLM call and replaces old history with a `ConversationEvent::Summary`

---

*Architecture analysis: 2026-03-31*
