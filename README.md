# Codiv Agent

A terminal-native AI coding assistant built in Rust. Codiv replaces your shell with one that runs commands instantly and handles complex AI tasks in the same interface.

## What It Does

- **Dual-mode interface** — AI mode for natural language queries, Command mode for direct shell execution, toggled with Tab
- **AI-first** — starts in AI mode by default; the agent reasons, calls tools, and streams responses
- **Multi-model** — supports Anthropic, OpenAI, and Google providers via aisdk, with plans for per-role model assignment
- **Easy authentication** — `codiv login` wizard for API keys and OAuth with automatic token refresh
- **Native tools as subcommands** — built-in tools (read, write, edit, glob, grep, bash) are accessible both to the agent in-process and to users from the CLI

## Architecture

```
codiv (TUI client)          codivd (daemon)
┌────────────────┐          ┌──────────────────────┐
│ ratatui + pty  │◄─IPC────►│ tokio + aisdk        │
│ command index  │ bincode  │ agent + tool loop    │
│ tab completion │  over    │ session management   │
│ markdown render│  unix    │ worker bash sessions │
│                │ socket   │ token refresh        │
└────────────────┘          └──────────────────────┘
                                      │
                            ┌─────────┴─────────┐
                            │   codiv-tools      │
                            │ (shared lib crate) │
                            └───────────────────┘
```

| Crate | Purpose |
|-------|---------|
| `codiv` | TUI client — terminal rendering, bash co-process, dual-mode input, tab completion |
| `codivd` | Async daemon — AI agent, session management, LLM streaming, tool execution |
| `codiv-tools` | Shared library — tool implementations (bash, read, write, edit, glob, grep) and agent guides |
| `codiv-common` | Shared types — IPC messages, auth types, config, utilities |

## Install

### Quick Install (macOS / Linux)

```bash
curl -fsSL https://codiv.ai/install.sh | bash
```

This downloads the latest release, installs `codiv` and `codivd` to `~/.local/bin/`, and starts the daemon as a system service (launchd on macOS, systemd on Linux).

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

This installs both `codiv` and `codivd` and optionally runs the daemon via brew services.

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

- An API key for at least one LLM provider (Anthropic, OpenAI, or Google)

### Run

```bash
# Launch the TUI (default — replaces your shell)
codiv

# Use tools directly from the CLI
codiv read --path ./src/main.rs
codiv grep --pattern "fn main" --path ./src
codiv bash --command "cargo test"
codiv glob --pattern "**/*.rs"

# Authenticate with AI providers
codiv login
codiv login anthropic

# Migrate API keys from environment variables to config
codiv migrate-env

# JSON mode for programmatic use
echo '{"file_path":"./Cargo.toml"}' | codiv read --json-in --json-out

# Tool metadata
codiv read --help
codiv read --agent-guide
```

### Debug Mode

```bash
# Logs to /tmp/codiv-debug.log
codiv --debug
codiv --debug=trace
```

## CLI Tool Reference

Each built-in tool is available as a `codiv` subcommand with these common flags:

| Flag | Purpose |
|------|---------|
| `--json-in` | Read JSON input from stdin |
| `--json-out` | Output result as JSON (`{"result": ..., "error": ...}`) |
| `--agent-guide` | Print agent usage guide |
| `--help` | Print help |

### Tools

| Command | Description |
|---------|-------------|
| `codiv read --path <file> [--offset N] [--limit N]` | Read file contents with numbered lines |
| `codiv write --path <file> --content <text>` | Write content to a file |
| `codiv edit --path <file> --old-string <old> --new-string <new>` | Find and replace a unique string |
| `codiv glob --pattern <glob> [--path <dir>]` | Find files matching a pattern |
| `codiv grep --pattern <regex> [--path <dir>] [--include <glob>]` | Search file contents with regex |
| `codiv bash --command <cmd> [--timeout <ms>]` | Execute a shell command |

## Roadmap

### Phase 1: Terminal Foundation — Complete

Working terminal client with daemon IPC, dual-mode input, and zero-overhead shell experience.

1. Ratatui TUI with persistent bash co-process (portable-pty)
2. Command index: PATH scanning + bash/zsh builtins in O(1) hash map
3. Dual-mode input: AI mode (default) and Command mode with Tab toggle
4. 3-tier tab completion: programmable bash-completion, command, file
5. Interactive passthrough for full-screen programs (vim, ssh, python REPL)
6. Serde+bincode IPC over Unix domain socket with heartbeat
7. Env snapshot protocol and worker bash sessions

### Phase 2: Single-Agent AI Loop — Complete

AI mode input goes to an agent that reasons and uses tools.

1. [x] aisdk integration with streaming LLM access (Anthropic, OpenAI, Google)
2. [x] Unified session timeline (shell commands + queries + responses)
3. [x] Real-time CommandResult IPC
4. [x] Streamdown markdown rendering with syntax highlighting
5. [x] Terminal colorscheme detection
6. [x] Native tools extracted to shared crate with CLI subcommands
7. [x] Tool calling loop (agent reasons, calls tools, continues)
8. [x] Risk classification + confirmation prompts for destructive commands
9. [x] Env snapshot refresh on `cd` and `source`
10. [x] TOML config for API keys and model selection (with hot-reload)
11. [x] OAuth authentication with interactive login wizard and automatic token refresh

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

### Phase 5: Memory + Project Context

Persistent bounded memory across sessions with project auto-switching.

- SQLite episodic store with configurable retention
- Semantic markdown files (user.md, project.md, topics/)
- Narrator agent for memory consolidation
- Project fingerprint detection and auto-context switching

### Phase 6: Tool System

Unified tool registry — binary tools, MCP bridge, prompt tools, hooks, aliases.

- `CODIV_TOOLS_PATH` discovery and progressive loading (Tier 0-3)
- MCP bridge (CLI ↔ MCP protocol translation)
- Prompt tool runtime with skill import
- Hooks and aliases in config

### Phase 7: Advanced Safety + Audit

Production-grade safety controls and compliance.

- Full audit trail (JSON lines with timestamps, agent roles, model IDs)
- Privacy settings (redact env vars, exclude output from LLM context)
- OS-level sandboxing roadmap (bubblewrap on Linux, seatbelt on macOS)
- Error handling: daemon crash recovery, LLM timeout backoff, IPC reconnection

## Development

```bash
make build    # cargo build --workspace
make test     # cargo test --workspace
make release  # cargo build --workspace --release
make clean    # cargo clean
```

## License

This project is licensed under the [GNU General Public License v3.0](LICENSE).
