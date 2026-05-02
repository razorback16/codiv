# Codiv Agent

A terminal-native AI coding assistant built in Rust. Codiv replaces your shell with one that runs commands instantly and handles complex AI tasks in the same interface.

**Version:** 0.1.7 | **Platform:** macOS / Linux | **License:** [GPL-3.0](LICENSE)

## What It Does

- **Dual-mode interface** — AI mode for natural language queries, Command mode for direct shell execution, toggled with Tab
- **Zero-latency commands** — recognized shell commands execute with no AI round-trip, indistinguishable from a regular terminal
- **AI agent with tool calling** — the agent reasons, calls tools (bash, read, write, edit, glob, grep), and streams markdown responses
- **AST-based safety** — shell commands are parsed into an AST and classified into four risk levels (Low/Medium/High/Critical) before execution, with confirmation prompts and an optional LLM evaluator fallback
- **Multi-provider auth** — `codiv login` wizard supports API keys (Anthropic, OpenAI) and OAuth with PKCE (Claude Code, Codex), with automatic background token refresh
- **Session persistence** — conversations are stored in SQLite and can be resumed, replayed, or compacted when context grows too large
- **Live config reload** — `~/.codiv/config.toml` is watched with `notify`; changes take effect without restarting the daemon
- **Shell lease protocol** — the daemon acquires a lease on the client's PTY before running commands, with per-command timeouts, queuing, and cancellation support
- **Native tools as subcommands** — all six built-in tools are accessible both to the agent and to users directly from the CLI

## Architecture

Codiv is split into two processes connected over a Unix domain socket. The TUI client stays responsive while the daemon handles all AI operations asynchronously.

```
codiv (TUI client)          codivd (daemon)
┌────────────────┐          ┌──────────────────────┐
│ ratatui + pty  │◄─IPC────►│ tokio + aisdk        │
│ command index  │ bincode  │ agent + tool loop    │
│ tab completion │  over    │ session persistence  │
│ markdown render│  unix    │ permission gating    │
│ shell lease    │ socket   │ token refresh        │
└────────────────┘          └──────────────────────┘
                                      │
                            ┌─────────┴─────────┐
                            │   codiv-tools      │
                            │ (shared lib crate) │
                            └───────────────────┘
```

| Crate | Purpose |
|-------|---------|
| `codiv` | TUI client — ratatui rendering, bash co-process, dual-mode input, tab completion, shell lease client |
| `codivd` | Async daemon — AI agent loop, session management, LLM streaming, tool execution, permission system, relay manager |
| `codiv-tools` | Shared library — tool implementations (bash, read, write, edit, glob, grep) with JSON I/O and agent guides |
| `codiv-common` | Shared types — IPC messages, conversation events, auth (OAuth + API key), config paths |

## Install

### Quick Install (macOS / Linux)

```bash
curl -fsSL https://codiv.ai/install.sh | bash
```

Downloads the latest release, installs `codiv` and `codivd` to `~/.local/bin/`, and registers the daemon as a system service (launchd on macOS, systemd on Linux).

To install a specific version:

```bash
curl -fsSL https://codiv.ai/install.sh | bash -s -- v0.1.7
```

### Homebrew (macOS / Linux)

```bash
brew tap razorback16/codiv https://github.com/razorback16/codiv.git
brew install codiv
brew services start codiv
```

### Cargo Install (from Git)

```bash
cargo install --git https://github.com/razorback16/codiv.git codiv
cargo install --git https://github.com/razorback16/codiv.git codivd
```

Requires the Rust toolchain — install via [rustup](https://rustup.rs/).

### Build from Source

```bash
git clone https://github.com/razorback16/codiv.git
cd codiv
cargo build --workspace --release
ln -sf "$(pwd)/target/release/codiv" ~/.local/bin/codiv
ln -sf "$(pwd)/target/release/codivd" ~/.local/bin/codivd
```

### Prerequisites

- macOS or Linux (Unix-only)
- An API key or OAuth credentials for at least one LLM provider (Anthropic, OpenAI, or Google)

## Usage

```bash
# Launch the TUI
codiv

# Authenticate with AI providers
codiv login                # interactive wizard — pick a provider
codiv login anthropic      # API key entry
codiv login claude_code    # OAuth (Anthropic)
codiv login codex          # OAuth (OpenAI)

# Migrate API keys from environment variables to config
codiv migrate-env

# Use tools directly from the CLI (bypasses permission checks)
codiv read --path ./src/main.rs
codiv grep --pattern "fn main" --path ./src
codiv bash --command "cargo test"
codiv glob --pattern "**/*.rs"
codiv write --path ./out.txt --content "hello"
codiv edit --path ./src/main.rs --old-string "old" --new-string "new"

# JSON mode for programmatic use
echo '{"file_path":"./Cargo.toml"}' | codiv read --json-in --json-out

# Debug logging
codiv --debug              # logs to /tmp/codiv-debug.log
codiv --debug=trace        # verbose
```

## CLI Tool Reference

Each built-in tool is available as a `codiv` subcommand with these common flags:

| Flag | Purpose |
|------|---------|
| `--json-in` | Read JSON input from stdin |
| `--json-out` | Output result as JSON (`{"result": ..., "error": ...}`) |
| `--agent-guide` | Print agent usage guide (for LLM consumption) |
| `--help` | Print help |

| Command | Description |
|---------|-------------|
| `codiv read --path <file> [--offset N] [--limit N]` | Read file contents with numbered lines |
| `codiv write --path <file> --content <text>` | Write content to a file |
| `codiv edit --path <file> --old-string <old> --new-string <new>` | Find and replace a unique string |
| `codiv glob --pattern <glob> [--path <dir>]` | Find files matching a pattern |
| `codiv grep --pattern <regex> [--path <dir>] [--include <glob>]` | Search file contents with regex |
| `codiv bash --command <cmd> [--timeout <ms>]` | Execute a shell command |

## Configuration

Config lives at `~/.codiv/config.toml` and hot-reloads without restarting the daemon.

| Setting | Description |
|---------|-------------|
| `[providers.<id>]` | API key and base URL per provider |
| `[models.roles.<role>]` | Model assignment per agent role (engineer, reviewer, etc.) |
| `[compaction]` | Context compaction threshold and behavior |
| `[permissions]` | Default permission mode, allow/deny lists |
| `[auth.tokens.<id>]` | OAuth access/refresh tokens (managed by `codiv login`) |

Environment variables (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GOOGLE_API_KEY`) override config file values. Use `codiv migrate-env` to move them into config.toml.

## Current Status (v0.1.7)

Phases 1 and 2 are complete. The current release is a fully functional single-agent AI terminal.

### Phase 1: Terminal Foundation — Complete

- Ratatui TUI with persistent bash co-process (portable-pty)
- Command index: PATH scanning + bash/zsh builtins in O(1) hash map
- Dual-mode input: AI mode (default) and Command mode with Tab toggle
- 3-tier tab completion: programmable bash-completion, command, file
- Interactive passthrough for full-screen programs (vim, ssh, python REPL)
- Serde + bincode IPC over Unix domain socket
- Environment snapshot protocol (syncs PWD and env vars on `cd` and `source`)

### Phase 2: Single-Agent AI Loop — Complete

- aisdk integration with streaming LLM access (Anthropic, OpenAI, Google, OpenAI-compatible)
- Unified session timeline (shell commands + AI queries + tool calls + responses)
- Streamdown markdown rendering with syntax highlighting and terminal colorscheme detection
- Six native tools extracted to shared crate with CLI subcommands
- Tool calling loop (agent reasons → calls tools → continues)
- AST-based risk classification (brush_parser) with four risk levels and confirmation prompts
- Shell lease protocol for daemon-to-client bash execution with timeouts and cancellation
- Session persistence in SQLite with replay and auto-compaction
- TOML config with hot-reload via filesystem watcher
- OAuth authentication (PKCE) for Anthropic and Codex, API key auth for Anthropic/OpenAI/Google
- Background token refresh in daemon (checks every 30s, refreshes 1 min before expiry)

## Roadmap

### Phase 3: Work Item DAG + Scheduler

Complex tasks decompose into a dependency graph of Work Items executed concurrently.

- Work Item schema with state machine (pending → running → completed/failed)
- DAG construction from agent planning
- Tokio-based concurrent execution with dependency tracking
- Artifact storage and token/cost budget enforcement

### Phase 4: Multi-Agent Roles + Multi-Model

Specialized agent roles with different models assigned per role.

- Roles: Orchestrator, TeamLead, Engineer, Reviewer, Narrator
- Model catalog with per-role assignment (frontier for planning, cheap for memory curation)
- TeamLead dynamic model selection per Work Item
- Reviewer gating on outputs
- Single-writer ownership for concurrent file access

The config schema already supports per-role model assignment (`[models.roles]`), but the runtime currently uses a single Engineer agent per session.

### Phase 5: Memory + Project Context

Persistent bounded memory across sessions with project auto-switching.

- SQLite episodic store with configurable retention
- Semantic markdown files (user.md, project.md, topics/)
- Narrator agent for memory consolidation
- Project fingerprint detection and auto-context switching

### Phase 6: Tool System

Unified tool registry — binary tools, MCP bridge, prompt tools, hooks, aliases.

- `CODIV_TOOLS_PATH` discovery and progressive loading (Tier 0–3)
- MCP bridge (CLI ↔ MCP protocol translation)
- Prompt tool runtime with skill import
- Hooks and aliases in config

### Phase 7: Advanced Safety + Audit

Production-grade safety controls and compliance.

- Full audit trail (JSON lines with timestamps, agent roles, model IDs)
- Privacy settings (redact env vars, exclude output from LLM context)
- OS-level sandboxing (bubblewrap on Linux, seatbelt on macOS)
- Daemon crash recovery, LLM timeout backoff, IPC reconnection

## Development

```bash
make build    # cargo build --workspace
make test     # cargo test --workspace
make release  # cargo build --workspace --release
make debug    # kill daemon, restart with debug logging, launch TUI in debug mode
make clean    # cargo clean
```

| Path | Purpose |
|------|---------|
| `~/.codiv/config.toml` | User configuration |
| `~/.codiv/sessions.db` | Session database |
| `~/.codiv/codivd.pid` | Daemon PID file |
| `~/.codiv/codivd.log` | Daemon log file |
| `/tmp/codiv-debug.log` | Client debug log (`--debug`) |
| `/tmp/codivd-{uid}.sock` | IPC socket |

## License

This project is licensed under the [GNU General Public License v3.0](LICENSE).
