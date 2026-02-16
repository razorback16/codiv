# Unified Tool Model — Design Document

**Date**: 2026-02-16
**Author**: Subhagato
**Status**: Approved
**Replaces**: FR-011 (Skill System), FR-012 (Plugin System), FR-013 (MCP Client)

---

## 1. Problem

The original PRD defines seven separate extensibility concepts: Skills, Commands, Plugins, MCP Servers, Hooks, Agent definitions, and SKILL.md files. These overlap in unclear ways, create confusion about when to use what, and were largely copied from Claude Code's patterns without questioning whether they're appropriate for a terminal-native agent.

A terminal is already a tool execution environment. An AI agent that lives in a terminal shouldn't need a separate protocol (MCP) to call external tools when it already has the shell.

## 2. Design Philosophy

**Unix pipes**: one primitive, infinite composability. Extensions are just programs.

- **The one primitive is Tool** — a CLI executable with a minimal convention
- CLI tools already have a universal interface: stdin/stdout, exit codes, flags, `--help`
- The shell is the protocol — MCP is an unnecessary indirection when you have stdio
- The LLM is the adapter layer — it reads `--help` and figures out how to call things
- Skills are not a separate concept — they're expertise prompts attached to tools
- Hooks are not a separate concept — they're event bindings to tools
- Slash commands are not a separate concept — they're aliases to tool invocations

## 3. The Tool Interface

Every tool in Slate Agent — built-in or external — exposes the same interface:

```
<tool> <command> [args] [flags]
```

### Required convention

| Flag | Purpose |
|------|---------|
| `--help` | Human and agent-readable description of all commands, args, I/O |

### Optional conventions

| Flag | Purpose |
|------|---------|
| `--agent-guide` | Returns a strategy/expertise prompt teaching the agent how to use this tool well |
| `--agent-guide <topic>` | Returns deep documentation on a specific topic |
| `--json-in` | Accept JSON on stdin instead of positional args |
| `--json-out` | Structured JSON output instead of text |

### Examples

```bash
# External tool
git-commit --help              # "Creates a git commit. Args: --message, --amend..."
git-commit --agent-guide       # "Before committing, always run git status first..."
git-commit --agent-guide workflows  # Deep doc on commit workflows
git-commit run --message "fix auth bug" --json-out
# -> {"commit_hash": "abc123", "files_changed": 3}

# Built-in tool (same interface, runs in-process)
slate-read --help
slate-read run --path ./src/main.cpp --offset 10 --limit 50
```

### Agent's view

The agent sees a flat list of tools, each with a name, a help string, and optionally an agent guide. It doesn't know or care whether a tool is built-in, a CLI binary, or a bridged MCP server.

## 4. Discovery & SLATE_TOOLS_PATH

Tools are discovered via `SLATE_TOOLS_PATH` — a colon-separated list of directories, just like `PATH`.

```bash
SLATE_TOOLS_PATH=.slate-agent/tools:~/.slate-agent/tools:/usr/local/share/slate-agent/tools
```

### Discovery order (first match wins)

1. **Project-level**: `.slate-agent/tools/` — project-specific tools
2. **User-level**: `~/.slate-agent/tools/` — personal tools
3. **System-level**: `/usr/local/share/slate-agent/tools/` — system-wide installs

### Built-in tools

Read, Write, Edit, Glob, Grep, Bash are always available regardless of `SLATE_TOOLS_PATH`. They are compiled into the daemon but expose the same `--help` / `--agent-guide` interface.

### Startup behavior

1. Scan `SLATE_TOOLS_PATH` directories
2. Run `<tool> --help` for each discovered executable (cached, only re-runs on mtime change)
3. Build tool index: name -> help text -> optional agent-guide -> location
4. Agent receives the tool index as available tools

### Search

```bash
slate tools search "database migration"  # grep over cached --help and --agent-guide text
slate tools search --tag "database"       # filter by tags (from registry metadata)
slate tools list                          # all installed tools
slate tools info <tool>                   # full --help + --agent-guide output
```

Search is grep over the cached help text. No vector DB, no fancy indexing.

## 5. Progressive Loading

With 100+ tools installed, dumping all help text and agent guides into the agent's context wastes thousands of tokens. Progressive loading keeps the context window lean.

### Four tiers

```
Tier 0: Always in context     -> tool name + one-line description
Tier 1: Loaded on relevance   -> full --help output (args, flags, I/O)
Tier 2: Loaded on invocation  -> --agent-guide (strategy + doc index)
Tier 3: Loaded on demand      -> --agent-guide <topic> (deep docs)
```

### How it works

**Session start**: Agent receives Tier 0 — a compact list of all available tools with one-line descriptions extracted from the first line of `--help`. 50 tools = ~50 lines of context.

**Agent considers using a tool**: Daemon injects Tier 1 — the full `--help` output for that tool. Agent now knows args, flags, I/O format. Triggered when the agent mentions the tool or the task seems relevant (daemon uses keyword matching against tool descriptions).

**Agent invokes a tool**: Daemon injects Tier 2 — the `--agent-guide` output. Agent now has the expertise prompt teaching it strategy, not just syntax. Loaded once per session per tool. The guide includes a doc index listing available topics.

**Agent needs deep knowledge**: Agent calls `--agent-guide <topic>` to load Tier 3 — specific documentation on a narrow subject. Only the relevant doc is loaded, never all of them.

### Example flow

```
Session start:
  [read: "Read file contents"]
  [write: "Write file to disk"]
  [grep: "Search file contents"]
  [git-commit: "Create git commits with best practices"]
  [db-migrate: "Run database migrations"]
  ... 50 more one-liners

User: "commit my changes"
  -> Daemon detects relevance, loads Tier 1 for git-commit
  -> Agent sees full --help (args, flags, examples)

Agent decides to call git-commit:
  -> Daemon loads Tier 2 (--agent-guide)
  -> Agent now knows: check status first, stage selectively, write good messages...
  -> Agent sees doc index: workflows, advanced, troubleshooting

Agent encounters pre-commit hook failure:
  -> Agent calls: git-commit --agent-guide troubleshooting
  -> Tier 3: specific guidance on hook failures loaded
```

## 6. MCP Bridge

The MCP bridge makes existing MCP servers appear as regular CLI tools. The user never sees MCP — they just see tools.

### How it works

1. User installs: `slate install mcp:@modelcontextprotocol/server-filesystem`
2. Registry creates a thin wrapper executable in `~/.slate-agent/tools/` that translates:
   - `--help` -> MCP `tools/list`
   - `<command> [args]` -> MCP `tools/call`
   - `--json-out` -> passes MCP's JSON response through
   - `--agent-guide` -> returns MCP tool descriptions as an agent guide
3. Daemon discovers it like any other tool — no special handling

### State management

MCP servers are stateful (persistent connections, session state, auth tokens). The daemon manages MCP server subprocesses as children:

- **Spawned lazily** on first tool call
- **Stay alive** for the session duration
- **Health-checked** via MCP `ping`
- **Shut down** with the daemon

The CLI wrapper in `SLATE_TOOLS_PATH` is a thin IPC call back to the daemon, which holds the actual MCP connection. State is preserved naturally because the daemon is the long-lived process.

### Configuration

```toml
# ~/.slate-agent/config.toml

[[tools.mcp]]
name = "filesystem"
package = "@modelcontextprotocol/server-filesystem"
args = ["/Users/subhagato/Development"]
transport = "stdio"

[[tools.mcp]]
name = "github"
package = "@modelcontextprotocol/server-github"
transport = "http"
url = "https://api.github.com/mcp"
env = { GITHUB_TOKEN = "${GITHUB_TOKEN}" }
```

### Key point

The MCP bridge is an implementation detail of the registry. A developer writing a native Slate tool never thinks about MCP. A developer wanting to use an MCP server just runs `slate install mcp:<package>` and gets a tool.

## 7. Hooks as Event-Bound Tools

Hooks are tool invocations triggered by lifecycle events. Not a separate concept.

### Configuration

```toml
# ~/.slate-agent/config.toml or .slate-agent/config.toml

[[hooks]]
event = "pre-commit"
tool = "lint-staged"
args = ["--config", ".lintstagedrc"]

[[hooks]]
event = "session-start"
tool = "project-context"
args = ["--load"]

[[hooks]]
event = "post-tool-use"
match = "write"
tool = "auto-format"
args = ["--path", "$ARTIFACT_PATH"]
```

### Available events

| Event | When it fires |
|-------|--------------|
| `session-start` | Daemon begins a new session |
| `session-end` | Session closes |
| `pre-tool-use` | Before any tool executes (can abort) |
| `post-tool-use` | After any tool executes |
| `pre-commit` | Before git commit |
| `post-task` | After a Work Item completes |
| `pre-compact` | Before memory compaction |

### Match field

Optional filter. Without it, the hook fires on every occurrence of that event. With it, only fires when the event matches (e.g., specific tool name, specific file pattern).

### Variable substitution

Event context is passed as environment variables so hook args can reference them:

- `$TOOL_NAME` — the tool that triggered the event
- `$ARTIFACT_PATH` — path to the artifact produced
- `$EXIT_CODE` — exit code of the tool
- `$WORK_ITEM_ID` — the Work Item that triggered `post-task`

## 8. Slash Commands as Aliases

Slash commands are aliases to tool invocations. Not a separate concept.

### Configuration

```toml
# ~/.slate-agent/config.toml

[[aliases]]
name = "commit"
tool = "git-commit"
args = ["run"]

[[aliases]]
name = "plan"
tool = "work-item"
args = ["plan", "--interactive"]

[[aliases]]
name = "tasks"
tool = "work-item"
args = ["list"]

[[aliases]]
name = "search"
tool = "slate-tools"
args = ["search"]
```

### Usage

```
/commit                    -> git-commit run
/commit --amend            -> git-commit run --amend
/plan                      -> work-item plan --interactive
/tasks                     -> work-item list
```

Built-in aliases ship with the daemon (`/help`, `/history`, etc.) but can be overridden in config. Override order follows the same chain as `SLATE_TOOLS_PATH`: project > user > system.

## 9. Registry & Distribution

Tools are distributed via an npm/brew-style registry.

### CLI commands

```bash
slate install <tool>              # install from registry
slate install mcp:<package>       # install MCP server as a tool (bridge)
slate install ./path/to/tool      # install from local path
slate remove <tool>               # uninstall
slate list                        # show installed tools
slate update [tool]               # update one or all
slate search "query"              # search registry (remote)
slate tools search "query"        # search installed tools (local)
```

### Tool package structure

```
my-tool/
├── tool.toml          # Manifest
├── bin/               # Executable(s)
│   └── my-tool
├── docs/              # Optional bundled documentation
│   ├── workflows.md   # Exposed via --agent-guide workflows
│   ├── advanced.md    # Exposed via --agent-guide advanced
│   └── troubleshooting.md
└── README.md          # Optional
```

### Manifest (tool.toml)

```toml
[tool]
name = "git-commit"
version = "1.0.0"
author = "Subhagato"
description = "Create git commits with best practices"
tags = ["git", "vcs", "commit"]

[install]
bin = "bin/git-commit"
```

That's it. One binary, one manifest. The binary handles `--help` and optionally `--agent-guide`. Documentation files in `docs/` are served by the binary's `--agent-guide <topic>` subcommand.

## 10. What Was Eliminated

| Old concept (PRD v1.1) | Replaced by |
|------------------------|-------------|
| Skills (FR-011) | `--agent-guide` flag on any tool |
| SKILL.md format | `--agent-guide` + bundled docs/ |
| Commands (FR-012) | Slash aliases in config |
| Plugin bundle with 6 component types (FR-012) | Tool package: 1 binary + tool.toml |
| MCP Client as separate system (FR-013) | MCP bridge (daemon-managed, exposed as tools) |
| Hooks system (FR-012) | Event bindings in config |
| Agent definitions (FR-012) | A tool that calls an LLM is just a tool |
| plugin.toml with components map | tool.toml with single binary |
| Built-in skills hardcoded separately | Built-in tools with same interface |

**Seven extension concepts collapsed into one primitive (Tool) plus configuration (aliases, hooks, registry).**

## 11. Impact on PRD

This design replaces FR-011, FR-012, and FR-013 with a single unified FR:

**FR-011: Unified Tool System**

- One primitive: Tool (CLI executable with `--help`, optional `--agent-guide`, optional `--json-in`/`--json-out`)
- Built-in tools compiled into daemon with same interface
- Discovery via `SLATE_TOOLS_PATH` (project > user > system)
- Progressive loading: Tier 0 (name) -> Tier 1 (help) -> Tier 2 (agent-guide) -> Tier 3 (deep docs)
- MCP bridge: `slate install mcp:<pkg>` wraps MCP servers as tools, daemon manages state
- Hooks: event + tool bindings in config
- Aliases: slash commands mapped to tool invocations in config
- Registry: `slate install/remove/update/search` with npm/brew-style distribution
- Search: grep over cached help text

Phase 6 in the implementation plan should be updated to reflect this unified model instead of the separate Plugin/MCP/Skill phases.
