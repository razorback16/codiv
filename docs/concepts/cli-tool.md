# Unifying Concepts: Everything is a CLI Tool

## "CLI tools are MCP in this shell-agent"

Yes, exactly right. MCP exists because LLMs need a standard way to discover and call external capabilities. But Slate Agent is a shell. CLI tools already have a universal interface: stdin → stdout, exit codes, flags, `--help`. **The shell is the protocol.** You don't need JSON-RPC 2.0 over stdio when you have... stdio. MCP becomes an unnecessary indirection layer — the daemon can discover CLI tools the same way a shell does (PATH, which, help text), and the agent can call them the same way a human does.

## "Invoking a skill means the LLM gets power to use some CLI tools better"

This reframes skills beautifully. A skill isn't a separate concept — it's a **tool enhancement**. It's the difference between:

- The agent knowing `git commit` exists (raw tool)
- The agent knowing how to use `git commit` well — check status first, write good messages, stage selectively, etc. (skill = tool + expertise prompt)

So a skill is just **metadata + prompt** attached to a tool that makes the agent smarter about using it. Not a separate primitive.

## "Everything is a CLI tool"

This collapses the entire 7-concept taxonomy into one:

| Old concept | New concept |
| --- | --- |
| Built-in tool | CLI tool (compiled into daemon) |
| MCP server | CLI tool (discovered from SLATE_TOOLS_PATH) |
| Skill | CLI tool + expertise prompt |
| Command | CLI tool alias (slash shortcut) |
| Plugin | Package of CLI tools (npm-style) |
| Agent/subagent | CLI tool that happens to call an LLM |

## Binary tools vs prompt tools

A tool can be implemented in two ways — both present the same interface to the agent:

- **Binary tool**: Provides its own executable in `bin/`. The binary handles `--help` and `--agent-guide` directly. Bundled docs live in `docs/`.
- **Prompt tool**: Has no binary — the daemon acts as the runtime. The tool is defined by `tool.toml` (with a `[skill]` section) + `guide.md` (the expertise prompt). The daemon synthesizes `--help` and `--agent-guide` from these files.

Prompt tools can bundle three types of resources:

| Directory | Purpose | Loading |
| --- | --- | --- |
| `scripts/` | Executable helpers (Python, Bash, etc.) | Executed at runtime via Bash, not loaded into context |
| `references/` | Documentation for agent context | Loaded on-demand (Tier 3), maps to `--agent-guide <topic>` |
| `assets/` | Output resources (templates, images) | Never loaded into context, used directly by path |

This means Claude Code skills (SKILL.md + bundled resources) can be repackaged as Slate tools with minimal transformation: frontmatter → `tool.toml`, body → `guide.md`, bundled `scripts/`/`references/`/`assets/` stay unchanged.

## Hooks as event-triggered tools

This fits cleanly. A hook is just a tool binding:

```toml
[[hooks]]
event = "pre-commit"    # trigger
tool = "lint-staged"     # which CLI tool
args = ["--config", ".lintstagedrc"]
```

No new concept needed. It's "run this tool when this event happens." The tool itself is the same thing as any other tool — a CLI program. The only new idea is the event binding, which is configuration, not a new primitive.