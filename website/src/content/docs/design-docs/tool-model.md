---
title: Tool Model
description: How Codiv discovers, loads, and executes tools — from built-ins to MCP bridges.
section: Design Docs
order: 5
---

## One Primitive: Tool as CLI Executable

In Codiv, every tool is a CLI executable. Whether built-in or third-party, tools share the same interface:

- Accept parameters as CLI flags
- Support `--json-in` for structured input from stdin
- Support `--json-out` for structured output
- Provide `--agent-guide` for LLM-readable usage instructions
- Provide `--help` for human-readable help

This uniform interface means new tools can be written in any language.

## Progressive Loading (4 Tiers)

Tools load progressively to minimize startup time:

| Tier | When Loaded | What |
|------|-------------|------|
| **Tier 0** | Always available | Built-in tools (bash, read, write, edit, glob, grep) |
| **Tier 1** | On startup | Tools on `CODIV_TOOLS_PATH` — discovered by scanning the path |
| **Tier 2** | On demand | MCP-bridged tools — loaded when first referenced |
| **Tier 3** | On demand | Prompt tools (skills) — loaded from definition files |

## Binary Tools vs Prompt Tools

### Binary Tools

Standard executables that run as child processes. They receive input, perform work, and return output. All built-in tools are binary tools.

### Prompt Tools

Defined in configuration files rather than executables. A prompt tool is essentially a system prompt template with parameters — the agent fills in the parameters and the prompt is injected into the LLM context.

Prompt tools are useful for domain-specific workflows that do not require code execution.

## MCP Bridge

Codiv can bridge to [Model Context Protocol (MCP)](https://modelcontextprotocol.io/) servers:

```
Agent ──► Codiv Tool Interface ──► MCP Bridge ──► MCP Server
                                        │
                                   CLI ↔ MCP
                                   translation
```

The bridge translates between Codiv's CLI tool interface and MCP's JSON-RPC protocol. This lets agents use any MCP-compatible tool server without modification.

## Hooks and Aliases

### Hooks

Run commands before or after tool execution:

```toml
[tools.hooks]
before_bash = "echo 'Running command...'"
after_write = "cargo fmt -- {path}"
```

### Aliases

Create shortcuts for common tool invocations:

```toml
[tools.aliases]
test = "bash --command 'cargo test --workspace'"
fmt = "bash --command 'cargo fmt --all'"
```

## Skill Conversion

Codiv can import skills from Claude Code's format (CLAUDE.md-style definitions) and convert them to Codiv prompt tools. This provides a migration path for teams already using Claude Code workflows.
