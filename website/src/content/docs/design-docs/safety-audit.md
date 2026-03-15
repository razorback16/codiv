---
title: Safety & Audit
description: Risk classification, confirmation flows, and audit logging for command execution.
section: Design Docs
order: 4
---

## Overview

Codiv executes real shell commands on your system. The safety system ensures destructive or dangerous operations require explicit user approval before running.

## 4-Tier Risk Classification

Every command the agent wants to execute is classified into one of four risk tiers:

| Tier | Level | Action | Examples |
|------|-------|--------|----------|
| 0 | **Low** | Auto-execute | `ls`, `cat`, `echo`, `pwd`, `grep` |
| 1 | **Medium** | Confirm once | `rm file.txt`, `git commit`, `cargo build` |
| 2 | **High** | Confirm with warning | `rm -rf directory/`, `git push --force`, `chmod 777` |
| 3 | **Critical** | Confirm with strong warning | `rm -rf /`, `mkfs`, `dd if=/dev/zero` |

## Pattern-Based Classification

The risk engine uses pattern matching to classify commands:

- **Exact match** — `rm -rf /` is always critical
- **Prefix match** — commands starting with `rm -rf` are high risk
- **Flag detection** — `--force`, `--hard`, `--no-verify` elevate risk
- **Target analysis** — writing to system paths (`/etc`, `/usr`) elevates risk
- **Pipe analysis** — piping to `sh` or `bash` elevates risk

## Confirmation Flow

When a command requires confirmation, the TUI shows:

```
┌─────────────────────────────────────────────┐
│  Agent wants to run:                        │
│  $ rm -rf target/                           │
│                                             │
│  Risk level: MEDIUM                         │
│                                             │
│  [y] Allow  [n] Deny  [a] Always allow      │
└─────────────────────────────────────────────┘
```

- **Allow (y)** — execute this one time
- **Deny (n)** — skip this command, agent continues with the denial
- **Always allow (a)** — add to the permanent allowlist

## Allowlist / Denylist

Users configure trusted and blocked commands in `~/.codiv/config.toml`:

```toml
[safety]
allowlist = ["ls", "cat", "echo", "cargo test", "cargo build"]
denylist = ["rm -rf /", "mkfs"]
```

- **Allowlist** — commands that always execute without confirmation
- **Denylist** — commands that are always blocked, even if the agent requests them

The allowlist/denylist is checked before the pattern-based classifier.

## Audit Logging

Every command execution is logged for accountability:

```json
{
  "timestamp": "2025-01-15T10:30:00Z",
  "session_id": "abc-123",
  "command": "rm -rf target/",
  "risk_tier": 1,
  "action": "allowed",
  "source": "agent",
  "agent_role": "engineer"
}
```

Audit logs are written as JSON lines to `~/.codiv/audit.jsonl`.

## Roadmap: Phase 7

Future safety enhancements planned for Phase 7:

- **OS-level sandboxing** — bubblewrap on Linux, seatbelt on macOS to restrict filesystem and network access
- **Privacy controls** — redact environment variables and sensitive output from LLM context
- **Compliance mode** — configurable policies for enterprise environments
- **Crash recovery** — daemon crash recovery with in-progress task resumption
