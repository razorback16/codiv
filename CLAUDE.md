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

**Version:** 0.1.4 | **License:** GPL-3.0 | **Updated:** 2025-03-18
