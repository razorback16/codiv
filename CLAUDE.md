# CLAUDE.md — Codiv Agent Project Reference

## Purpose

This file documents foundational aspects of Codiv that remain stable as the codebase evolves. If something in the project surprises you, alert the developer and add it to "Common Pitfalls" below.

---

## Project Overview

**Codiv Agent** is a terminal-native AI coding assistant built in Rust. It replaces your shell with an interface that switches between instant command execution and AI task orchestration.

**Core Philosophy:**
- Zero-latency shell commands (no AI round-trip)
- Multi-model support (Anthropic, OpenAI, Google, OpenAI-compatible)
- Terminal-native (not a VS Code extension)
- Unix-only (macOS/Linux)

---

## Architecture

### Two-Process Design

```
codiv (TUI)                 codivd (daemon)
┌─────────────┐            ┌──────────────────┐
│ ratatui     │◄─IPC────►  │ tokio + aisdk    │
│ bash PTY    │ bincode    │ agent loop       │
│ completion  │ Unix sock  │ tool execution   │
└─────────────┘            └──────────────────┘
                                    │
                           ┌────────┴────────┐
                           │  codiv-tools    │
                           │  (shared lib)   │
                           └─────────────────┘
```

**Why this split?** Terminal stays responsive during long AI operations.

### Workspace

```
crates/
├── codiv/           # TUI client (ratatui, crossterm, portable-pty)
├── codivd/          # Daemon (tokio, aisdk, rusqlite)
├── codiv-tools/     # Tools (bash, read, write, edit, glob, grep)
└── codiv-common/    # Shared types, IPC, config
```

---

## Key Concepts

### IPC Protocol
- Unix domain socket at `/tmp/codivd-{uid}.sock`
- Serde + bincode serialization
- Length-prefixed framing

### Bash Co-Process
- Persistent bash session via PTY
- Shared by orchestrator agent
- Zero-latency command execution
- Full interactive program support (vim, ssh, etc.)

### Environment Snapshots
- Daemon syncs shell state (PWD, env vars) from client
- Triggered on `cd`, `source`, and before bash tool execution

### Tool Interface
All tools (built-in and external) are CLI executables with:
- `--json-in` / `--json-out` for structured I/O
- `--agent-guide` for LLM usage instructions
- `--help` for human help

### Configuration
- Location: `~/.codiv/config.toml`
- Hot-reloads without daemon restart
- Per-role model assignment, API keys, permissions

---

## Development

```bash
make build    # cargo build --workspace
make test     # cargo test --workspace
make release  # cargo build --workspace --release

codiv --debug        # Logs to /tmp/codiv-debug.log
codiv --debug=trace  # Verbose logging
```

---

## Common Pitfalls

### 1. Config File Path
Config is at `~/.codiv/config.toml` (not in `~/.config/`)

### 2. Environment Variable Precedence
Environment variables override config file:
- `ANTHROPIC_API_KEY`
- `OPENAI_API_KEY`
- `GOOGLE_API_KEY`

### 3. Permission System Context
- Agent tool calls → permission checks apply
- Direct CLI (`codiv <tool>`) → bypasses permission checks

### 4. Tool Schema Updates
Adding fields to tool input structs requires:
- `#[derive(JsonSchema)]` on the struct
- Proper `#[serde]` and `#[schemars]` attributes

### 5. Daemon Socket Cleanup
Multiple instances can conflict. Manual cleanup:
```bash
rm /tmp/codivd-*.sock
```

### 6. Test Session Isolation
Tests interacting with daemon need unique sessions or cleanup to prevent state bleed.

### 7. IPC Compatibility
Changing message types in `codiv-common` breaks client-daemon communication.

---

## External Dependencies

**Forked:**
- `aisdk` (git: razorback16/aisdk) — unified LLM interface
- `streamdown-rs` (git: razorback16/streamdown-rs) — markdown streaming parser

**Core:**
- `ratatui`, `crossterm` — terminal UI
- `tokio` — async runtime
- `portable-pty` — PTY management
- `rusqlite` — session storage
- `schemars` — JSON Schema for tools

---

## Quick Reference

| Command | Purpose |
|---------|---------|
| `codiv` | Launch TUI |
| `codiv <tool>` | Run tool directly |
| `codiv --debug` | Debug logging |
| `codivd` | Start daemon |
| `pkill codivd` | Stop daemon |

| File | Purpose |
|------|---------|
| `~/.codiv/config.toml` | Config |
| `~/.codiv/codivd.pid` | Daemon PID |
| `~/.codiv/sessions.db` | Session DB |
| `~/.codiv/codivd.log` | Daemon logs |
| `/tmp/codiv-debug.log` | Debug logs (--debug mode) |

---

**Version:** 0.1.6 | **License:** GPL-3.0 | **Updated:** 2026-03-20

<!-- GSD:project-start source:PROJECT.md -->
## Project

**Codiv Auth — AI Provider Login System**

A unified authentication system for Codiv that adds a `codiv login` CLI command with an interactive wizard for configuring AI provider credentials. Ported and adapted from the forgecode project's auth infrastructure, it supports API key entry and OAuth code flows for Anthropic (Claude Code), OpenAI (Codex), and other providers. Credentials are stored in `~/.codiv/config.toml` alongside existing configuration.

**Core Value:** Users can authenticate with any supported AI provider through a single `codiv login` command — no manual config file editing, no hunting for environment variable names, no expired tokens breaking sessions.

### Constraints

- **Config format**: TOML (not JSON like forgecode) — must merge auth into existing config.toml
- **Architecture**: Token refresh must be in codivd daemon, not client
- **UX**: CLI subcommand (`codiv login`), not TUI inline
- **Dependencies**: Minimize new dependencies — prefer reusing what's already in the workspace
- **Security**: API keys and tokens must not be logged or exposed in debug output
<!-- GSD:project-end -->

<!-- GSD:stack-start source:codebase/STACK.md -->
## Technology Stack

## Languages
- Rust (edition 2021) - All application code across all four crates
- TOML - Configuration files (`~/.codiv/config.toml`, `crates/codivd/config.default.toml`)
- Bash - Install script (`install.sh`), Makefile build targets
## Runtime
- Unix-only (macOS / Linux) — `nix` crate usage, Unix domain sockets, PTY management
- No browser, no WASM, no cross-platform Windows target
- Cargo 1.94.0
- Lockfile: `Cargo.lock` present (committed)
## Workspace Layout
| Crate | Path | Role |
|---|---|---|
| `codiv` | `crates/codiv/` | TUI client binary |
| `codivd` | `crates/codivd/` | Daemon binary |
| `codiv-tools` | `crates/codiv-tools/` | Tool implementations (shared lib) |
| `codiv-common` | `crates/codiv-common/` | Shared types, IPC, config (shared lib) |
## Frameworks
- `ratatui` 0.30 — TUI rendering framework (`crates/codiv/`)
- `crossterm` 0.28 — Cross-platform terminal control (`crates/codiv/`)
- `tui-term` 0.3 — Terminal emulator widget for ratatui (`crates/codiv/`)
- `vt100` 0.16 — VT100 terminal emulator state (`crates/codiv/`)
- `tokio` 1 (features: `full`) — Async runtime in daemon (`crates/codivd/`)
- `tokio` 1 (features: `rt-multi-thread`, `macros`) — Used in codiv dev-dependencies
- `aisdk` 0.5 (forked: `github.com/razorback16/aisdk`, branch `main`) — Unified LLM interface, features: `anthropic`, `openai`, `google`, `openaicompatible`, `vllm` (`crates/codivd/`)
- `streamdown-parser` (forked: `github.com/razorback16/streamdown-rs`) — Streaming markdown parser (`crates/codiv/`)
- `streamdown-render` (forked: `github.com/razorback16/streamdown-rs`) — Markdown renderer (`crates/codiv/`)
- `brush-parser` 0.3 — Bash AST parsing for risk classification (`crates/codivd/`)
- `termwright` 0.2 — Terminal UI testing framework (`crates/codiv/` dev-dependencies)
- Standard Rust `#[test]` and `#[tokio::test]` for unit/integration tests
## Key Dependencies
- `serde` 1 + `serde_json` 1 — JSON and struct serialization throughout all crates
- `bincode` 1 — Binary serialization for IPC framing (all crates)
- `crossbeam-channel` 0.5 — Multi-producer multi-consumer channels (`codiv`, `codiv-common`, `codiv-tools`)
- `portable-pty` 0.9 — PTY (pseudo-terminal) management (`codiv`, `codiv-common`)
- `nix` 0.29 — Unix syscalls: signals, process, terminal, filesystem (`codiv`, `codivd`)
- `libc` 0.2 — Low-level C bindings (`codiv`, `codiv-common`)
- `rusqlite` 0.31 (feature: `bundled`) — Embedded SQLite for session storage (`crates/codivd/`)
- `toml` 0.8 — Config file parsing (`crates/codivd/`)
- `toml_edit` 0.22 — Format-preserving config editing (`crates/codivd/`)
- `notify` 7 — Filesystem watcher for hot-reload of config (`crates/codivd/`)
- `reqwest` 0.12 (feature: `json`) — HTTP client used within `aisdk` for LLM API calls (`crates/codivd/`)
- `clap` 4 (feature: `derive`) — CLI argument parsing (`crates/codiv/`)
- `uuid` 1 (feature: `v4`) — Session ID generation (`crates/codivd/`)
- `schemars` 1 — JSON Schema generation for tool definitions (`crates/codivd/`, `crates/codiv-tools/`)
- `futures` 0.3 — Stream combinators (`crates/codivd/`)
- `tracing` 0.1 + `tracing-subscriber` 0.3 — Structured logging in daemon (`crates/codivd/`)
- `log` 0.4 + `env_logger` 0.11 — Logging in TUI client (`crates/codiv/`)
- `dirs` 6 — Platform home directory resolution (`crates/codiv/`)
- `rand` 0.8 — Random number generation (`codiv`, `codiv-common`)
- `similar` 2 — Diff computation for edit display (`crates/codiv/`)
- `human_format` 1 — Human-readable number formatting (`crates/codiv/`)
- `glob` 0.3 — File glob matching (`crates/codiv-tools/`)
- `grep-regex` 0.1 + `grep-searcher` 0.1 — Ripgrep-based file search (`crates/codiv-tools/`)
- `ignore` 0.4 — `.gitignore`-aware directory traversal (`crates/codiv-tools/`)
## Configuration
- Config file: `~/.codiv/config.toml` (auto-created from embedded default on first run)
- Environment variable overrides: `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GOOGLE_API_KEY`
- Per-provider and per-role API keys configurable in TOML
- Config hot-reloads at runtime via `notify` filesystem watcher (no daemon restart needed)
- `Makefile` with targets: `build`, `test`, `release`, `debug`, `clean`
- `make build` → `cargo build --workspace`
- `make release` → `cargo build --workspace --release`
- `make test` → `cargo test --workspace`
- `~/.codiv/config.toml` — User configuration
- `~/.codiv/sessions.db` — SQLite session database
- `~/.codiv/codivd.pid` — Daemon PID file
- `~/.codiv/codivd.log` — Daemon log file
- `/tmp/codiv-debug.log` — Debug log (only with `--debug` flag)
- `/tmp/codivd-{uid}.sock` — Unix domain socket for IPC
## Platform Requirements
- Rust 1.94.0+ (edition 2021)
- macOS or Linux (Unix-only)
- `cargo` / `make`
- macOS (tested; LaunchAgent plist in `install.sh`)
- Linux (supported; systemd unit in `install.sh`)
- Self-contained binary distribution via GitHub Releases (`razorback16/codiv`)
<!-- GSD:stack-end -->

<!-- GSD:conventions-start source:CONVENTIONS.md -->
## Conventions

## Naming Patterns
- `snake_case.rs` everywhere: `bash_coprocess.rs`, `permission_evaluator.rs`, `ast_classifier.rs`
- Modules mirror directory names: `crates/codiv/src/ui/terminal/render.rs` → `crate::ui::terminal::render`
- No barrel-file aliasing — each module is individually imported
- `PascalCase`: `BashCoprocess`, `InputLine`, `ClientMessage`, `DaemonMessage`, `ApiErrorKind`
- Enum variants are `PascalCase`: `RiskLevel::High`, `PermissionDecision::LlmEvaluate`
- `snake_case`: `classify_error`, `generate_sentinel`, `truncate_output`, `frame_message`
- Boolean predicates prefixed with `is_`: `is_retryable()`, `is_readonly_bash()`, `is_char_boundary()`
- Constructor convention: `fn new(...)` for primary constructors, `fn open(...)` for resource openers (`SessionStore::open`)
- Builder-style helpers: `fn spawn(...)` for processes, `fn build_tools(...)` for factory functions
- `SCREAMING_SNAKE_CASE`: `MAX_TOOL_OUTPUT_BYTES`, `MAX_HISTORY_EVENTS`, `PASTE_COLLAPSE_THRESHOLD`, `INITIAL_BACKOFF_MS`
- Module-level constants declared immediately before the code that uses them
- Sparse use: only `type DynError = Box<dyn std::error::Error + Send + Sync>` found in `crates/codivd/src/agent/config.rs`
## Code Style
- Standard `rustfmt` (no custom config detected)
- Edition 2021 across all crates
- `#[allow(dead_code)]` used to suppress warnings for fields intentionally unused (e.g., `blocks.rs`, `state.rs`) — prefer this over removing fields that serve documentation purposes
- `#[allow(clippy::too_many_arguments)]` used in `event_loop.rs` and `input_keys.rs` for large event-handler functions
- `#[allow(clippy::module_inception)]` in `crates/codivd/src/agent/mod.rs` where module and inner file share a name
## Import Organization
- None — fully qualified paths only
## Section Separators
## Error Handling
- `Result<T, String>` — used in tool execute functions (`read::execute`, `bash::execute`) where the error is a human-readable message
- `Result<T, rusqlite::Error>` — typed errors for database operations in `store.rs`
- `Result<T, DynError>` (where `DynError = Box<dyn Error + Send + Sync>`) — async agent/IPC pipeline in `config.rs`
- `std::io::Result<T>` — process/socket/PTY operations
- `?` operator throughout async code
- `map_err(|e| format!("context: {e}"))` for converting typed errors to `String` errors at tool boundaries
- No `anyhow` or `thiserror` — either raw `Box<dyn Error>` or explicit `Result<T, SomeError>`
- `ApiErrorKind` enum in `crates/codivd/src/agent/error.rs` provides structured classification of API errors (Server, RateLimited, ContextOverflow, AuthError, Other) with `is_retryable()` helper
- `tracing::error!(...)` / `tracing::warn!(...)` for soft failures in the daemon
- `log::debug!()` / `log::info!()` in the TUI client (uses `log` crate, not `tracing`)
- `.ok()` for explicitly discarding errors where failure is acceptable (e.g., cleanup operations)
## Logging
- **Daemon (`codivd`):** `tracing` crate — `tracing::info!`, `tracing::debug!`, `tracing::warn!`, `tracing::error!`
- **TUI client (`codiv`):** `log` crate — `log::debug!`, `log::info!`
- `debug`: Per-command execution details, PTY/completion events, stream details
- `info`: Major lifecycle events (spawn, connect, session create, config reload)
- `warn`: Recoverable failures (timeout, channel full, LLM evaluator fallback)
- `error`: Failed DB writes, stream failures, spawn failures
## Comments
- Used within function bodies to explain non-obvious logic: protocol details, edge cases, why something is done a certain way
- Section separators (`// ---`) used inside large `impl` blocks to group related methods
## Function Design
## Module Design
- `pub` for cross-crate API surface (in `codiv-common`)
- `pub(crate)` for intra-crate sharing between modules (common in `codiv/src/ui/terminal/`)
- `pub(super)` for constants and helpers shared between parent and child module only
- Private by default for implementation details
- No barrel `lib.rs` re-exports in `codiv` or `codivd` (binary crates)
- `codiv-common` and `codiv-tools` use `lib.rs` with explicit `pub mod` declarations
- `mod.rs` files in subdirectories declare and optionally re-export their children
<!-- GSD:conventions-end -->

<!-- GSD:architecture-start source:ARCHITECTURE.md -->
## Architecture

## Pattern Overview
- `codiv` (TUI client) and `codivd` (daemon) are separate binaries communicating over a Unix socket
- The TUI client stays responsive because all AI/LLM work runs in the daemon's async tokio runtime
- The daemon holds all AI state (conversation history, agent, sessions); the client is stateless for AI purposes
- Shell commands execute in a PTY co-process owned by the TUI client, with the daemon relaying execution requests back to the client when the agent needs to run bash
- IPC uses serde + bincode with 4-byte big-endian length-prefixed framing
## Layers
- Purpose: User-facing terminal interface, shell management, IPC relay
- Location: `crates/codiv/src/`
- Contains: `app.rs` (startup orchestrator), `ui/` (ratatui rendering), `shell/` (PTY/bash coprocess), `ipc/` (IPC client), `cli/` (direct tool dispatch subcommands)
- Depends on: `codiv-common` (shared types and IPC framing), `codiv-tools` (for direct CLI invocation)
- Used by: end users directly
- Purpose: AI agent loop, session persistence, permission gating
- Location: `crates/codivd/src/`
- Contains: `daemon.rs` (event loop), `agent/` (LLM agent, tools, permissions), `ipc/` (Unix socket server), `session.rs` (per-client state), `store.rs` (SQLite persistence)
- Depends on: `codiv-common`, `codiv-tools`, `aisdk` (LLM abstraction)
- Used by: `codiv` TUI via IPC
- Purpose: Shared types, IPC message definitions, config paths
- Location: `crates/codiv-common/src/`
- Contains: `messages.rs` (IPC enums `ClientMessage`/`DaemonMessage`/`StreamChunk`), `conversation.rs` (`ConversationEvent` enum), `types.rs` (`AgentRole`), `permissions.rs` (`PermissionMode`/`PermissionDecision`), `config.rs` (path helpers)
- Depends on: serde, bincode
- Used by: both `codiv` and `codivd`
- Purpose: Shared implementations of all agent tools (bash, read, write, edit, glob, grep)
- Location: `crates/codiv-tools/src/`
- Contains: `tools/bash.rs`, `tools/read.rs`, `tools/write.rs`, `tools/edit.rs`, `tools/glob.rs`, `tools/grep.rs`, `agent_guide.rs`
- Depends on: schemars (JSON schema for tool inputs)
- Used by: `codivd` (agent tool execution), `codiv` (direct CLI subcommand dispatch)
## Data Flow
- The `ShellBackend::ClientRelay` variant in `crates/codivd/src/agent/shell_backend.rs` implements a request-response relay: daemon sends `ExecuteCommand` to the client; client executes in PTY; client sends `CommandExecutionResult` back; daemon unblocks the waiting agent task
- `TerminalState` (`crates/codiv/src/ui/terminal/state.rs`) is the single mutable state object for the TUI, holding pending commands, streaming buffers, block registry, token usage, and UI flags
- `ClientSession` (`crates/codivd/src/session.rs`) is the daemon-side per-client state, holding the `Agent`, permission context, and SQLite session ID
- `Agent` struct (`crates/codivd/src/agent/agent.rs`) holds conversation history as `Vec<ConversationEvent>` plus model config and shell backend reference
## Key Abstractions
- Purpose: Typed bidirectional IPC between TUI and daemon
- Location: `crates/codiv-common/src/messages.rs`
- Pattern: Serde enums serialized with bincode, framed with 4-byte BE length prefix via `frame_message()` / `parse_frame_header()`
- Purpose: Unified event type for both in-memory agent history and SQLite persistence/replay
- Location: `crates/codiv-common/src/conversation.rs`
- Pattern: Enum with variants for `UserPrompt`, `AssistantText`, `ToolCall`, `ToolResult`, `ShellCommand`, `Summary`, `TokenUsage`, `Error`
- Purpose: Wraps aisdk LLM client, maintains conversation history, streams responses
- Location: `crates/codivd/src/agent/agent.rs`
- Pattern: Struct with `history: Vec<ConversationEvent>`, `model_config`, `provider_config`, `shell_backend`
- Purpose: Per-session permission gating for tool calls
- Location: `crates/codivd/src/agent/permissions.rs`
- Pattern: Shared `Arc<PermissionContext>` with `RwLock<PermissionMode>` (Auto/Manual/Bypass) and risk classification pipeline (`risk_classifier.rs` → `permission_evaluator.rs` → optional `llm_evaluator.rs`)
- Purpose: TUI rendering abstraction for conversation turns (tool calls, prompts, responses)
- Location: `crates/codiv/src/ui/blocks.rs`
- Pattern: Registry of `ToolBlock`, `PromptBlock`, `CmdResponseBlock` each tracking scrollback index, height, and rendered ANSI lines
- Purpose: Persistent bash session via PTY providing zero-latency shell execution
- Location: `crates/codiv/src/shell/bash_coprocess.rs`
- Pattern: `portable-pty`-based PTY, shared by both direct user commands and AI-relayed commands
- Purpose: SQLite-backed persistence of sessions and conversation events
- Location: `crates/codivd/src/store.rs`
- Pattern: Synchronous `rusqlite` `Connection` on the daemon main task; `sessions` and `events` tables with WAL journal mode
- Purpose: Abstract bash execution for the agent (currently `ClientRelay` only)
- Location: `crates/codivd/src/agent/shell_backend.rs`
- Pattern: Enum allowing future backends; `ClientRelay` routes via IPC to the TUI's PTY
## Entry Points
- Location: `crates/codiv/src/main.rs`
- Triggers: User runs `codiv` (no subcommand)
- Responsibilities: Parse CLI args, init logging, set SIGTERM handler, call `app::run()`
- Location: `crates/codiv/src/main.rs` → `crates/codiv/src/cli/mod.rs`
- Triggers: User runs `codiv read <file>`, `codiv bash <cmd>`, etc.
- Responsibilities: Dispatch to `codiv-tools` implementations directly, bypassing permission checks
- Location: `crates/codivd/src/main.rs` → `async_main()`
- Triggers: Service manager (launchd/systemd) or direct invocation
- Responsibilities: Fork/daemonize, write PID file, build tokio runtime, create and run `Daemon`
- Location: `crates/codivd/src/daemon.rs`
- Triggers: Called from `async_main()`
- Responsibilities: `tokio::select!` on new IPC connections, client messages, agent completion, name updates, config changes, cleanup interval, SIGTERM
- Location: `crates/codiv/src/ui/terminal/event_loop.rs`
- Triggers: Called from `ui::terminal::run()`
- Responsibilities: Poll crossterm events (keyboard, mouse, resize), daemon messages, tick channel; dispatch to input/render handlers; manage daemon reconnection
## Error Handling
- IPC send failures return `bool` (silent on disconnect) allowing graceful degradation
- Agent errors surface as `DaemonMessage::Error` sent to TUI, displayed inline
- Daemon startup failures use `process::exit(1)` after logging
- `SessionStore` migration errors are fatal (daemon exits)
- Tool execution errors return `Result<String, String>` — error string becomes tool result content for the LLM
## Cross-Cutting Concerns
- TUI (`codiv`): `env_logger` writing to `/tmp/codiv-debug.log`, activated with `--debug` flag
- Daemon (`codivd`): `tracing_subscriber` writing to `~/.codiv/codivd.log`, level controlled by `RUST_LOG` env var
- Tool inputs are validated via `serde_json::from_value()` at call sites; invalid inputs return error strings to the LLM
- API keys sourced from env vars (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GOOGLE_API_KEY`) or `~/.codiv/config.toml`; env vars take precedence
- `AppConfig` uses a file watcher that sends on `config_change_tx`; daemon reloads on next event loop iteration without restart
- When `input_tokens` exceeds threshold, `needs_compaction` is set on `ClientSession`; next idle cycle triggers `CompactRequest` which runs a summarization LLM call and replaces old history with a `ConversationEvent::Summary`
<!-- GSD:architecture-end -->

<!-- GSD:workflow-start source:GSD defaults -->
## GSD Workflow Enforcement

Before using Edit, Write, or other file-changing tools, start work through a GSD command so planning artifacts and execution context stay in sync.

Use these entry points:
- `/gsd:quick` for small fixes, doc updates, and ad-hoc tasks
- `/gsd:debug` for investigation and bug fixing
- `/gsd:execute-phase` for planned phase work

Do not make direct repo edits outside a GSD workflow unless the user explicitly asks to bypass it.
<!-- GSD:workflow-end -->

<!-- GSD:profile-start -->
## Developer Profile

> Profile not yet configured. Run `/gsd:profile-user` to generate your developer profile.
> This section is managed by `generate-claude-profile` -- do not edit manually.
<!-- GSD:profile-end -->
