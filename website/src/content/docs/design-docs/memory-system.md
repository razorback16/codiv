---
title: Memory System
description: Two-tier persistent memory with episodic storage and semantic knowledge.
section: Design Docs
order: 3
---

## Overview

Codiv's memory system gives the AI agent persistent context across sessions. It uses a two-tier architecture combining structured event storage with human-readable knowledge files.

## Two-Tier Architecture

### Tier 1: Episodic Memory (SQLite)

A local SQLite database stores discrete events:

- Commands run and their outputs
- AI queries and responses
- Tool calls and results
- Errors and resolutions

Each event is timestamped, tagged with a session ID, and linked to a project. The database supports configurable retention — older episodes are pruned automatically.

### Tier 2: Semantic Memory (Markdown)

Long-lived knowledge is stored as markdown files:

```
~/.codiv/memory/
├── user.md          # User preferences and patterns
├── projects/
│   ├── my-app.md    # Project-specific knowledge
│   └── my-lib.md
└── topics/
    ├── rust-patterns.md
    ├── testing.md
    └── git-workflow.md
```

These files are readable and editable by both the agent and the user.

## Storage Layout

```
~/.codiv/
├── config.toml
├── memory/
│   ├── episodic.db       # SQLite database
│   ├── user.md           # User-level semantic memory
│   ├── projects/         # Per-project knowledge
│   └── topics/           # Cross-project topic files
└── ...
```

## Episode Schema

Each episodic record contains:

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID | Unique identifier |
| `timestamp` | DateTime | When the event occurred |
| `session_id` | UUID | Which session this belongs to |
| `project_id` | String | Project fingerprint |
| `event_type` | Enum | Command, Query, Response, ToolCall, Error |
| `content` | Text | The event payload |
| `tokens_used` | Integer | Token count if applicable |

## Narrator Consolidation

A background process (the Narrator agent in Phase 4) periodically reviews episodic memory and consolidates patterns into semantic memory:

1. **Scan** recent episodes for recurring patterns
2. **Extract** insights (e.g., "user prefers spaces over tabs", "this project uses pytest")
3. **Update** the relevant semantic markdown file
4. **Prune** episodes that have been consolidated

## Topic Lifecycle

Semantic topic files follow a lifecycle:

- **Create** — when the narrator identifies a new recurring theme
- **Merge** — when two topics overlap significantly
- **Split** — when a topic grows too broad
- **Retire** — when a topic is no longer relevant (archived, not deleted)

## Memory Tools

The agent has access to memory-specific tools:

- **recall** — search episodic memory for relevant past events
- **remember** — store a new insight in semantic memory
- **forget** — remove outdated information

## Project Auto-Switching

Codiv detects which project you are working in using filesystem fingerprinting:

- Checks for marker files (Cargo.toml, package.json, .git)
- Computes a project fingerprint from the directory structure
- Automatically loads the matching project memory context

When you `cd` into a different project, Codiv detects the change and switches memory context without any manual configuration.
