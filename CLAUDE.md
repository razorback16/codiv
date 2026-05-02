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
│ login/auth  │            │ token refresh    │
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
├── codiv/           # TUI client (ratatui, crossterm, portable-pty, dialoguer)
├── codivd/          # Daemon (tokio, aisdk, rusqlite)
├── codiv-tools/     # Tools (bash, read, write, edit, glob, grep)
└── codiv-common/    # Shared types, IPC, config, auth
```

---

## Key Concepts

### IPC Protocol
- Unix domain socket at `/tmp/codivd-{uid}.sock`
- Serde + bincode serialization
- Length-prefixed framing
- **Warning:** Do not use `#[serde(skip_serializing_if)]` on fields going through `frame_message()` — bincode is non-self-describing and it will silently corrupt messages

### Shell Lease Protocol
Agent bash execution uses a lease-based state machine (replaced the v0.1.6 request-response relay):
1. Daemon sends `AcquireShellLease` → client grants or queues
2. `ExecuteLeasedCommand` runs in PTY with per-command timeout
3. `ReleaseShellLease` frees the PTY for the next lease
- Supports cancellation (`UserAbort`, `ExecutionTimeout`, `SessionStale`)
- Managed by `RelayManager` (`crates/codivd/src/agent/relay_manager.rs`)

### Bash Co-Process
- Persistent bash session via PTY
- Shared by orchestrator agent via shell lease protocol
- Zero-latency command execution
- Full interactive program support (vim, ssh, etc.)
- History expansion disabled (`HISTFILE=/dev/null`)

### Authentication
- `codiv login [provider]` — interactive wizard for API key entry or OAuth code flows
- `codiv migrate-env` — migrates environment variable API keys to config.toml
- 4 built-in providers: Anthropic, OpenAI, Claude Code (OAuth), Codex (OAuth)
- Provider registry in `codiv-common/src/auth/provider_registry.rs`
- OAuth supports PKCE, localhost callback (Codex) and manual paste (Anthropic) flows
- Credentials stored in `~/.codiv/config.toml` under `[providers.{id}]` and `[auth.tokens.{id}]`
- Background token refresh task in daemon (checks every 30s, refreshes 1 min before expiry)
- API keys use safe display truncation (never logged in full)

### Conversation Compaction
- When `input_tokens` exceeds threshold, `needs_compaction` is set on `ClientSession`
- Next idle cycle triggers a summarization LLM call
- Old history is replaced with a `ConversationEvent::Summary` event
- Manual compaction available via `/compact` command

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
- Auth tokens stored at `[auth.tokens.{provider_id}]`

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
Environment variables override config file. Use `codiv migrate-env` to move them into config.toml:
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

### 8. Bincode Serialization
Do not use `#[serde(skip_serializing_if)]` on any field serialized through `frame_message()`. Bincode is non-self-describing — skipped fields silently corrupt the message.

### 9. OAuth Token Storage
OAuth tokens are stored in `[auth.tokens.{provider_id}]` in config.toml. The access token is also written to `[providers.{provider_id}] api_key` for daemon compatibility. Refreshing happens automatically in the daemon background task.

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

**Auth:**
- `dialoguer` — interactive CLI prompts (login wizard)
- `oauth2` — PKCE code challenge/verifier
- `reqwest` — HTTP client for OAuth token exchange
- `chrono` — token expiry tracking

---

## Quick Reference

| Command | Purpose |
|---------|---------|
| `codiv` | Launch TUI |
| `codiv <tool>` | Run tool directly |
| `codiv login [provider]` | Auth wizard (API key or OAuth) |
| `codiv migrate-env` | Migrate env vars to config.toml |
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

**Version:** 0.1.7 | **License:** GPL-3.0 | **Updated:** 2026-05-02


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
- `dialoguer` 0.12 — Interactive CLI prompts with Select/Password widgets (`crates/codiv/`)
- `oauth2` 5.0 — PKCE code challenge/verifier generation (`crates/codiv-common/`)
- `open` 5.3 — Open URLs in default browser for OAuth flows (`crates/codiv/`)
- `chrono` 0.4 — DateTime for OAuth token expiry tracking (`crates/codiv-common/`)
- `anyhow` 1 — Error handling in auth flows and CLI commands (`crates/codiv/`, `crates/codiv-common/`)
## Configuration
- Config file: `~/.codiv/config.toml` (auto-created from embedded default on first run)
- Environment variable overrides: `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GOOGLE_API_KEY`
- Per-provider and per-role API keys configurable in TOML
- OAuth tokens stored at `[auth.tokens.{provider_id}]` with access/refresh/expiry
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
- Sparse use: only `type DynError = Box<dyn std::error::Error + Send + Sync>` found in `crates/codivd/src/agent/models.rs`
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
- `Result<T, DynError>` (where `DynError = Box<dyn Error + Send + Sync>`) — async agent/IPC pipeline in `streaming.rs`
- `std::io::Result<T>` — process/socket/PTY operations
- `?` operator throughout async code
- `map_err(|e| format!("context: {e}"))` for converting typed errors to `String` errors at tool boundaries
- `anyhow` used in auth flows (`codiv-common/src/auth/`) and CLI login/migrate commands
- No `thiserror` — either raw `Box<dyn Error>`, `anyhow::Result`, or explicit `Result<T, SomeError>`
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
- Contains: `app.rs` (startup orchestrator), `ui/` (ratatui rendering, `terminal/daemon/` submodule with streaming.rs, session.rs, lease.rs, helpers.rs), `shell/` (PTY/bash coprocess), `ipc/` (IPC client), `cli/` (direct tool dispatch subcommands)
- Depends on: `codiv-common` (shared types and IPC framing), `codiv-tools` (for direct CLI invocation)
- Used by: end users directly
- Purpose: AI agent loop, session persistence, permission gating
- Location: `crates/codivd/src/`
- Contains: `daemon.rs` (event loop), `agent/` (agent.rs, models.rs, providers.rs, streaming.rs, config.rs, permissions.rs, risk_classifier.rs, ast/, relay_manager.rs, tools.rs, error.rs, llm_evaluator.rs, permission_evaluator.rs), `handlers/` (agent, compaction, permission, session, shell), `ipc/` (Unix socket server), `session.rs` (per-client state), `store.rs` (SQLite persistence)
- Depends on: `codiv-common`, `codiv-tools`, `aisdk` (LLM abstraction)
- Used by: `codiv` TUI via IPC
- Purpose: Shared types, IPC message definitions, config paths
- Location: `crates/codiv-common/src/`
- Contains: `messages.rs` (IPC enums `ClientMessage`/`DaemonMessage`/`StreamChunk`), `conversation.rs` (`ConversationEvent` enum), `types.rs` (`AgentRole`), `permissions.rs` (`PermissionMode`/`PermissionDecision`), `config.rs` (path helpers), `auth/` (types, flows, storage, provider_registry)
- Depends on: serde, bincode, oauth2, reqwest, chrono, toml_edit
- Used by: both `codiv` and `codivd`
- Purpose: Shared implementations of all agent tools (bash, read, write, edit, glob, grep)
- Location: `crates/codiv-tools/src/`
- Contains: `tools/bash.rs`, `tools/read.rs`, `tools/write.rs`, `tools/edit.rs`, `tools/glob.rs`, `tools/grep.rs`, `agent_guide.rs`
- Depends on: schemars (JSON schema for tool inputs)
- Used by: `codivd` (agent tool execution), `codiv` (direct CLI subcommand dispatch)
## Data Flow
- `RelayManager` in `crates/codivd/src/agent/relay_manager.rs` implements a lease-based shell protocol: daemon acquires a shell lease → sends command with timeout → awaits started ACK → awaits completion → releases lease. Supports queuing, timeout, and cancellation
- `TerminalState` (`crates/codiv/src/ui/terminal/state.rs`) is the single mutable state object for the TUI, holding pending commands, streaming buffers, block registry, token usage, UI flags, and `ShellRelayState` (shell lease management with active/pending leases)
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
- Pattern: Shared `Arc<PermissionContext>` with `RwLock<PermissionMode>` (Auto/Manual/Bypass) and risk classification pipeline (`risk_classifier.rs` → `ast/classify.rs` → `permission_evaluator.rs` → optional `llm_evaluator.rs`)
- Purpose: TUI rendering abstraction for conversation turns (tool calls, prompts, responses)
- Location: `crates/codiv/src/ui/blocks.rs` (registry/lifecycle) + `crates/codiv/src/ui/tool_presenters.rs` (visual presentation)
- Pattern: BlockRegistry manages lifecycle; tool_presenters contains extracted per-tool rendering functions (`present_edit()`, `present_read()`, `present_bash()`, etc.)
- Purpose: Persistent bash session via PTY providing zero-latency shell execution
- Location: `crates/codiv/src/shell/bash_coprocess.rs`
- Pattern: `portable-pty`-based PTY, shared by both direct user commands and AI-relayed commands
- Purpose: SQLite-backed persistence of sessions and conversation events
- Location: `crates/codivd/src/store.rs`
- Pattern: Synchronous `rusqlite` `Connection` on the daemon main task; `sessions` and `events` tables with WAL journal mode
- Purpose: Lease-based shell execution management for the agent
- Location: `crates/codivd/src/agent/relay_manager.rs`
- Pattern: State machine with `LeasePhase`/`CommandPhase` tracking; manages lease acquisition, command execution with timeouts, cancellation, and release via IPC to the TUI's PTY
## Entry Points
- Location: `crates/codiv/src/main.rs`
- Triggers: User runs `codiv` (no subcommand)
- Responsibilities: Parse CLI args, init logging, set SIGTERM handler, call `app::run()`
- Location: `crates/codiv/src/main.rs` → `crates/codiv/src/cli/mod.rs`
- Triggers: User runs `codiv read <file>`, `codiv bash <cmd>`, `codiv login`, `codiv migrate-env`, etc.
- Responsibilities: Dispatch to `codiv-tools` implementations directly (bypasses permission checks), or run auth subcommands (`login.rs`, `migrate.rs`)
- Location: `crates/codivd/src/main.rs` → `async_main()`
- Triggers: Service manager (launchd/systemd) or direct invocation
- Responsibilities: Fork/daemonize, write PID file, build tokio runtime, create and run `Daemon`
- Location: `crates/codivd/src/daemon.rs`
- Triggers: Called from `async_main()`
- Responsibilities: `tokio::select!` on new IPC connections, client messages, agent completion, name updates, config changes, cleanup interval, SIGTERM. Dispatches to focused handler modules in `handlers/` (agent, compaction, permission, session, shell). Spawns background OAuth token refresh task
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
