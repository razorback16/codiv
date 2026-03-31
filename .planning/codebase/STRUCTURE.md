# Codebase Structure

**Analysis Date:** 2026-03-31

## Directory Layout

```
codiv/                              # Workspace root
├── Cargo.toml                      # Workspace manifest (4 members)
├── Cargo.lock                      # Lockfile
├── Makefile                        # build/test/release targets
├── CLAUDE.md                       # Project reference doc
├── crates/
│   ├── codiv/                      # TUI client binary
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── main.rs             # Binary entry point, CLI parsing
│   │   │   ├── app.rs              # Startup orchestrator (bash + daemon + TUI)
│   │   │   ├── markdown.rs         # Streaming markdown renderer
│   │   │   ├── cli/                # Direct tool subcommands (no TUI)
│   │   │   │   ├── mod.rs          # Commands enum, dispatch()
│   │   │   │   └── tools.rs        # Clap arg structs + handler
│   │   │   ├── ipc/                # IPC client (Unix socket)
│   │   │   │   ├── mod.rs
│   │   │   │   ├── client.rs       # CodivdClient (sync read thread + channel)
│   │   │   │   ├── daemon_launcher.rs  # Socket path, launch helpers
│   │   │   │   └── messages.rs     # Re-exports + framing helpers
│   │   │   ├── shell/              # Bash PTY co-process
│   │   │   │   ├── mod.rs
│   │   │   │   ├── bash_coprocess.rs  # BashCoprocess (PTY, env capture, git info)
│   │   │   │   ├── command_index.rs   # Command history index
│   │   │   │   └── completion_engine.rs  # Tab-completion engine
│   │   │   └── ui/                 # Ratatui UI
│   │   │       ├── blocks.rs       # BlockRegistry: ToolBlock, PromptBlock, CmdResponseBlock
│   │   │       ├── color_downgrade.rs
│   │   │       ├── completion_popup.rs
│   │   │       ├── diff.rs         # Unified diff renderer
│   │   │       ├── input.rs        # InputLine widget
│   │   │       ├── theme.rs        # Theme (dark/light detection)
│   │   │       ├── tool_modal.rs   # Tool result modal overlay
│   │   │       └── terminal/       # Main TUI event loop
│   │   │           ├── mod.rs      # run() entry point, terminal setup/teardown
│   │   │           ├── animation.rs
│   │   │           ├── daemon.rs   # Daemon message handler
│   │   │           ├── event_loop.rs  # Main select! loop
│   │   │           ├── input.rs    # Keyboard input dispatch
│   │   │           ├── io.rs       # crossterm reader thread, tick channel, color query
│   │   │           ├── keys/       # Key binding handlers
│   │   │           ├── render.rs   # render_frame(), status bar
│   │   │           ├── state.rs    # TerminalState, PendingCommand, TokenUsage
│   │   │           └── utils.rs    # Scrollback helpers, notice push
│   │   └── tests/                  # Integration / E2E tests
│   │       └── screenshots/
│   ├── codivd/                     # Daemon binary
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── main.rs             # Binary entry point, daemonize, tokio runtime
│   │   │   ├── daemon.rs           # Daemon struct, async event loop
│   │   │   ├── daemon_shell.rs     # DaemonShell (local PTY fallback, not yet wired)
│   │   │   ├── session.rs          # ClientSession (per-connected-client state)
│   │   │   ├── store.rs            # SessionStore (rusqlite, sessions + events tables)
│   │   │   ├── agent/              # AI agent subsystem
│   │   │   │   ├── mod.rs
│   │   │   │   ├── agent.rs        # Agent struct, history, LLM call loop
│   │   │   │   ├── ast_classifier.rs  # AST-based tool classification
│   │   │   │   ├── config.rs       # AppConfig, ModelAssignment, hot-reload watcher
│   │   │   │   ├── error.rs        # Agent error types
│   │   │   │   ├── llm_evaluator.rs  # LLM-based permission evaluator
│   │   │   │   ├── permission_evaluator.rs  # Rule-based permission evaluation
│   │   │   │   ├── permissions.rs  # PermissionContext, wrap_with_permissions()
│   │   │   │   ├── risk_classifier.rs  # Risk level classification for tool calls
│   │   │   │   ├── shell_backend.rs  # ShellBackend::ClientRelay
│   │   │   │   └── tools.rs        # build_tools() — assembles aisdk Tool vec
│   │   │   └── ipc/                # IPC server (async Unix socket)
│   │   │       ├── mod.rs
│   │   │       └── server.rs       # IpcServer, client connection tasks
│   │   └── tests/                  # Daemon integration tests
│   ├── codiv-common/               # Shared types library
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── config.rs           # Path helpers (socket, pid, log, config dir)
│   │       ├── conversation.rs     # ConversationEvent enum, SessionInfo
│   │       ├── messages.rs         # ClientMessage, DaemonMessage, StreamChunk, RiskLevel, frame_message()
│   │       ├── permissions.rs      # PermissionMode, PermissionDecision
│   │       ├── tools.rs            # tool_names constants
│   │       ├── truncate.rs         # Output truncation helpers
│   │       ├── types.rs            # AgentRole enum
│   │       └── shell/              # Shell-related shared types
│   └── codiv-tools/                # Tool implementations library
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── agent_guide.rs      # --agent-guide text for tools
│           └── tools/
│               ├── mod.rs
│               ├── bash.rs         # BashInput struct, execute()
│               ├── edit.rs         # EditInput struct, execute()
│               ├── glob.rs         # GlobInput struct, execute()
│               ├── grep.rs         # GrepInput struct, execute()
│               ├── read.rs         # ReadInput struct, execute()
│               └── write.rs        # WriteInput struct, execute()
├── docs/                           # Design docs, plans, concepts
│   ├── concepts/
│   ├── design-docs/
│   └── plans/
│       └── archive/
├── references/                     # Vendored reference repos (aisdk, etc.)
│   └── aisdk/                      # Forked aisdk reference (not compiled directly)
├── .claude/                        # Claude skill guides
│   └── skills/
├── .planning/                      # GSD planning artifacts
│   └── codebase/
├── .github/workflows/              # CI
├── Formula/                        # Homebrew formula
├── install.sh                      # Install script
└── website/                        # Marketing site
```

## Directory Purposes

**`crates/codiv/src/`:**
- Purpose: All TUI client code
- Contains: Entry point, startup, UI rendering, shell PTY, IPC client, direct CLI dispatch
- Key files: `main.rs`, `app.rs`, `ui/terminal/event_loop.rs`, `ui/terminal/state.rs`

**`crates/codiv/src/ui/terminal/`:**
- Purpose: Core TUI rendering and event loop
- Contains: The `event_loop()` function that drives all user interaction
- Key files: `event_loop.rs`, `render.rs`, `state.rs`, `daemon.rs` (message handler)

**`crates/codiv/src/shell/`:**
- Purpose: PTY-based bash co-process management
- Contains: `BashCoprocess` (PTY lifecycle, env snapshots, git info, resize), completion engine
- Key files: `bash_coprocess.rs`

**`crates/codivd/src/agent/`:**
- Purpose: All AI-related logic including LLM calls, tool execution, permission gating
- Contains: `Agent`, `PermissionContext`, tool builder, risk/permission evaluators, config loader
- Key files: `agent.rs`, `tools.rs`, `permissions.rs`, `config.rs`

**`crates/codivd/src/ipc/`:**
- Purpose: Async Unix socket server for daemon-side IPC
- Contains: `IpcServer` that multiplexes multiple client connections via tokio tasks
- Key files: `server.rs`

**`crates/codiv-common/src/`:**
- Purpose: Shared contract between client and daemon — must not break compatibility
- Contains: All IPC message types, conversation event types, permission types, path helpers
- Key files: `messages.rs` (IPC protocol), `conversation.rs` (event model)

**`crates/codiv-tools/src/tools/`:**
- Purpose: Concrete tool implementations usable both by the agent and direct CLI
- Contains: One file per tool, each with an input struct (derives `JsonSchema`) and `execute()` fn
- Key files: `bash.rs`, `read.rs`, `write.rs`, `edit.rs`, `glob.rs`, `grep.rs`

## Key File Locations

**Entry Points:**
- `crates/codiv/src/main.rs`: TUI client binary entry; routes to TUI or direct CLI
- `crates/codivd/src/main.rs`: Daemon binary entry; daemonizes, starts tokio runtime
- `crates/codiv/src/app.rs`: Client startup orchestrator (bash + daemon + TUI)
- `crates/codivd/src/daemon.rs`: Daemon async event loop (`Daemon::run()`)

**IPC Protocol:**
- `crates/codiv-common/src/messages.rs`: All IPC message types and framing functions

**Conversation Model:**
- `crates/codiv-common/src/conversation.rs`: `ConversationEvent` enum used for history + persistence

**Configuration:**
- `crates/codiv-common/src/config.rs`: Runtime path helpers (socket, pid, log, config dir)
- `crates/codivd/src/agent/config.rs`: `AppConfig` with hot-reload watcher

**Persistence:**
- `crates/codivd/src/store.rs`: `SessionStore` (SQLite at `~/.codiv/sessions.db`)
- `crates/codivd/src/session.rs`: `ClientSession` (per-client in-memory state)

**Permission System:**
- `crates/codiv-common/src/permissions.rs`: `PermissionMode` enum (Auto/Manual/Bypass)
- `crates/codivd/src/agent/permissions.rs`: `PermissionContext`, `wrap_with_permissions()`
- `crates/codivd/src/agent/risk_classifier.rs`: Tool call risk classification
- `crates/codivd/src/agent/permission_evaluator.rs`: Rule-based decision
- `crates/codivd/src/agent/llm_evaluator.rs`: LLM-based decision (Auto mode, High risk)

**UI Rendering:**
- `crates/codiv/src/ui/blocks.rs`: `BlockRegistry` — the display model for conversation turns
- `crates/codiv/src/ui/terminal/render.rs`: `render_frame()` — the ratatui render function
- `crates/codiv/src/ui/terminal/state.rs`: `TerminalState` — all mutable TUI state

**Tools:**
- `crates/codiv-tools/src/tools/`: One file per tool (bash, read, write, edit, glob, grep)
- `crates/codivd/src/agent/tools.rs`: `build_tools()` — wires tools into the aisdk Tool vec

## Naming Conventions

**Files:**
- `snake_case.rs` for all Rust source files
- One public struct/enum per file where practical (e.g., `client.rs` → `CodivdClient`, `store.rs` → `SessionStore`)
- `mod.rs` for directory modules

**Directories:**
- `snake_case` for all directory names
- Feature-grouped (e.g., `agent/`, `ipc/`, `shell/`, `tools/`)

**Types:**
- `PascalCase` for structs, enums, traits
- `SCREAMING_SNAKE_CASE` for constants (e.g., `MAX_SCROLLBACK`, `MAX_HISTORY_EVENTS`)

**Functions:**
- `snake_case` for all functions and methods
- `build_*` prefix for factory functions (e.g., `build_tools()`)
- `cmd_*` prefix for CLI subcommand handlers in `codivd/main.rs`

## Where to Add New Code

**New agent tool:**
1. Add implementation struct + `execute()` to a new file `crates/codiv-tools/src/tools/<name>.rs`
2. Export from `crates/codiv-tools/src/tools/mod.rs`
3. Add aisdk `Tool` entry in `crates/codivd/src/agent/tools.rs` `build_tools()`
4. Add clap subcommand in `crates/codiv/src/cli/mod.rs` and `crates/codiv/src/cli/tools.rs`
5. Derive `JsonSchema` on the input struct and add `#[serde]` attributes

**New IPC message:**
1. Add variant to `ClientMessage` or `DaemonMessage` in `crates/codiv-common/src/messages.rs`
2. Handle in `crates/codivd/src/daemon.rs` (for `ClientMessage`) or `crates/codiv/src/ui/terminal/daemon.rs` (for `DaemonMessage`)
3. Note: changing `codiv-common` message types breaks client-daemon IPC compatibility

**New TUI UI state:**
1. Add field to `TerminalState` in `crates/codiv/src/ui/terminal/state.rs`
2. Handle in `event_loop.rs` and render in `render.rs`

**New conversation event type:**
1. Add variant to `ConversationEvent` in `crates/codiv-common/src/conversation.rs`
2. Handle in `crates/codivd/src/agent/agent.rs` (history building)
3. Handle in `crates/codivd/src/store.rs` (serialization to SQLite)
4. Handle in `crates/codivd/src/daemon.rs` (session replay)

**New configuration option:**
1. Add field to `AppConfig` or sub-struct in `crates/codivd/src/agent/config.rs`
2. Add default and TOML deserialization; config file is at `~/.codiv/config.toml`

**Utility / helper:**
- Shared utilities with no external deps: `crates/codiv-common/src/`
- TUI-only helpers: `crates/codiv/src/ui/terminal/utils.rs`
- Daemon-only helpers: add to the relevant module in `crates/codivd/src/`

## Special Directories

**`references/aisdk/`:**
- Purpose: Read-only reference copy of the forked aisdk crate for exploring API
- Generated: No (git submodule / manual copy)
- Committed: Yes (used for developer reference only, not compiled via workspace)

**`target/`:**
- Purpose: Cargo build artifacts
- Generated: Yes
- Committed: No

**`.planning/codebase/`:**
- Purpose: GSD codebase map documents for AI planning and execution
- Generated: Yes (by `/gsd:map-codebase`)
- Committed: Yes

**`.claude/skills/`:**
- Purpose: Claude skill guides for aisdk, ratatui, termwright, streamdown-rs
- Generated: No
- Committed: Yes

**`docs/`:**
- Purpose: Design docs, implementation plans, concept notes
- Generated: No
- Committed: Yes

---

*Structure analysis: 2026-03-31*
