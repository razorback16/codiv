# Slate Agent

A terminal-native AI coding assistant built in Rust. Slate replaces your shell with one that runs commands instantly and handles complex AI tasks in the same interface.

## What It Does

- **Command fast-pass** — recognized shell commands execute with near-zero overhead (<10ms target), no AI round-trip
- **AI routing** — unknown commands and natural language automatically route to an AI agent with tool calling
- **Multi-model** — supports Anthropic, OpenAI, and Google providers via aisdk, with plans for per-role model assignment
- **Native tools as subcommands** — built-in tools (read, write, edit, glob, grep, bash) are accessible both to the agent in-process and to users from the CLI

## Architecture

```
slate (TUI client)          slated (daemon)
┌────────────────┐          ┌──────────────────────┐
│ ratatui + pty  │◄─IPC────►│ tokio + aisdk        │
│ command index  │ bincode  │ agent + tool loop    │
│ tab completion │  over    │ session management   │
│ markdown render│  unix    │ worker bash sessions │
└────────────────┘ socket   └──────────────────────┘
                                      │
                            ┌─────────┴─────────┐
                            │   slate-tools      │
                            │ (shared lib crate) │
                            └───────────────────┘
```

| Crate | Purpose |
|-------|---------|
| `slate` | TUI client — terminal rendering, bash co-process, input classification, tab completion |
| `slated` | Async daemon — AI agent, session management, LLM streaming, tool execution |
| `slate-tools` | Shared library — tool implementations (bash, read, write, edit, glob, grep) and agent guides |
| `slate-common` | Shared types — IPC messages, config, utilities |

## Getting Started

### Prerequisites

- Rust toolchain (stable)
- An API key for at least one LLM provider (Anthropic, OpenAI, or Google)

### Build

```bash
cargo build --workspace
```

### Install

```bash
cargo build --workspace --release
ln -sf "$(pwd)/target/release/slate" /usr/local/bin/slate
```

### Run

```bash
# Launch the TUI (default — replaces your shell)
slate

# Use tools directly from the CLI
slate read --path ./src/main.rs
slate grep --pattern "fn main" --path ./src
slate bash --command "cargo test"
slate glob --pattern "**/*.rs"

# JSON mode for programmatic use
echo '{"file_path":"./Cargo.toml"}' | slate read --json-in --json-out

# Tool metadata
slate read --help
slate read --agent-guide
```

### Debug Mode

```bash
# Logs to /tmp/slate-debug.log
slate --debug
slate --debug=trace
```

## CLI Tool Reference

Each built-in tool is available as a `slate` subcommand with these common flags:

| Flag | Purpose |
|------|---------|
| `--json-in` | Read JSON input from stdin |
| `--json-out` | Output result as JSON (`{"result": ..., "error": ...}`) |
| `--agent-guide` | Print agent usage guide |
| `--help` | Print help |

### Tools

| Command | Description |
|---------|-------------|
| `slate read --path <file> [--offset N] [--limit N]` | Read file contents with numbered lines |
| `slate write --path <file> --content <text>` | Write content to a file |
| `slate edit --path <file> --old-string <old> --new-string <new>` | Find and replace a unique string |
| `slate glob --pattern <glob> [--path <dir>]` | Find files matching a pattern |
| `slate grep --pattern <regex> [--path <dir>] [--include <glob>]` | Search file contents with regex |
| `slate bash --command <cmd> [--timeout <ms>]` | Execute a shell command |

## Roadmap

### Phase 1: Terminal Foundation — Complete

Working terminal client with daemon IPC, command fast-pass, and zero-overhead shell experience.

1. Ratatui TUI with persistent bash co-process (portable-pty)
2. Command index: PATH scanning + bash/zsh builtins in O(1) hash map
3. Input classification: Execute, Interactive, AiQuery, NotFound, Clear, Exit
4. 3-tier tab completion: programmable bash-completion, command, file
5. Interactive passthrough for full-screen programs (vim, ssh, python REPL)
6. Serde+bincode IPC over Unix domain socket with heartbeat
7. Env snapshot protocol and worker bash sessions

### Phase 2: Single-Agent AI Loop — In Progress

Natural language routes to an AI agent that reasons and uses tools.

1. [x] aisdk integration with streaming LLM access (Anthropic, OpenAI, Google)
2. [x] Unified session timeline (shell commands + queries + responses)
3. [x] Real-time CommandResult IPC
4. [x] Streamdown markdown rendering with syntax highlighting
5. [x] Terminal colorscheme detection
6. [x] Native tools extracted to shared crate with CLI subcommands
7. [x] Tool calling loop (agent reasons, calls tools, continues)
8. [ ] Risk classification + confirmation prompts for destructive commands
9. [ ] Env snapshot refresh on `cd` and `source`
10. [ ] TOML config for API keys and model selection

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

- `SLATE_TOOLS_PATH` discovery and progressive loading (Tier 0-3)
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
