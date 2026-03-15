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
│ > refactor auth module               │  ← AI mode query
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
│                                      │
│ $ ls -la                             │  ← Shell command
│ total 48                             │  ← Shell output
│ drwxr-xr-x  12 user staff  384 ...  │
└──────────────────────────────────────┘
```

## Dual-Mode Input

The TUI uses an explicit dual-mode input system controlled by the `InputMode` enum, which has two variants: `Ai` and `Command`.

### Modes

- **AI mode** is the default on startup. The gutter displays `>` in cyan, and all input is sent to the AI agent for processing.
- **Command mode** switches the gutter to `$` in white, and input is sent directly to the bash co-process for execution.

### Switching modes

Pressing **Tab on an empty input line** toggles between AI mode and Command mode. The gutter symbol and color update immediately to reflect the active mode, giving you a clear visual indicator of where your input will go.

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
| AI mode gutter | `>` in cyan |
| Command mode gutter | `$` in white |

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
