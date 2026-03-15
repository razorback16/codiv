---
title: TUI Client
description: Terminal rendering, scroll model, markdown display, and interactive passthrough.
section: Design Docs
order: 6
---

## Overview

The Codiv TUI is built with [ratatui](https://ratatui.rs/) and uses a linear scroll-down flow model. Everything — shell output, AI responses, task headers — flows downward like a terminal, but with rich rendering.

## Linear Scroll-Down Flow

The TUI presents a single scrollable pane where content appears chronologically:

```
┌──────────────────────────────────────┐
│ $ ls -la                             │  ← Shell command
│ total 48                             │  ← Shell output
│ drwxr-xr-x  12 user staff  384 ...  │
│                                      │
│ ━━━ Task: Refactor auth module ━━━━  │  ← Task header
│ ├─ [✓] Read auth module              │  ← DAG tree
│ ├─ [→] Edit auth code                │
│ └─ [ ] Run tests                     │
│                                      │
│ I'll start by reading the current    │  ← Agent block
│ auth module to understand the        │
│ structure...                         │
│                                      │
│ ```rust                              │  ← Syntax highlighted
│ pub fn authenticate(token: &str)     │    code block
│ ```                                  │
└──────────────────────────────────────┘
```

## Rendering

### Markdown

Agent responses are rendered as markdown using streamdown-rs:

- Headings with proper hierarchy
- Code blocks with syntax highlighting
- Bold, italic, inline code
- Lists and blockquotes
- Tables
- Links

### Syntax Highlighting

Code blocks include language-aware syntax highlighting. The color scheme adapts to your terminal's color scheme — detected automatically at startup.

### Color Scheme by Role

Different content types use distinct visual styles:

| Element | Style |
|---------|-------|
| Shell command | Bold, prompt prefix |
| Shell output | Default terminal colors |
| Agent text | Rendered markdown |
| Tool calls | Muted, monospace |
| Errors | Red accent |
| Task headers | Bold, bordered |

## Interactive Passthrough

When you launch a full-screen program (vim, ssh, python REPL, top), the TUI enters passthrough mode:

1. Detects the program is interactive (from a known list + heuristics)
2. Hands full terminal control to the child process
3. Suspends TUI rendering
4. Resumes TUI when the program exits

This means `vim file.txt` works exactly as you would expect — full-screen, with all keybindings.

## Status Symbols

The TUI uses symbols to indicate work item and task status:

| Symbol | Meaning |
|--------|---------|
| `[ ]` | Pending |
| `[→]` | Running |
| `[✓]` | Completed |
| `[✗]` | Failed |
| `[⊘]` | Cancelled |
| `[⧖]` | Blocked |

These appear in DAG tree views and task summaries, giving you a quick visual overview of progress.
