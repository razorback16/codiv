---
title: Commands
description: How Codiv classifies and routes your input to the right handler.
section: Usage
order: 1
---

## Command Fast-Pass

When you type a command in Codiv, it first checks whether the command is a recognized shell command. If it is, the command executes immediately with near-zero overhead (target under 10ms) — no AI round-trip required.

This lookup uses an O(1) hash map built from:

- All executables found on your `PATH`
- Shell builtins for bash and zsh

## Input Classification

Every line of input is classified into one of six categories:

| Classification | Description | Example |
|---|---|---|
| **Execute** | Known command — runs immediately | `ls -la`, `git status` |
| **Interactive** | Full-screen program — passthrough mode | `vim`, `ssh`, `python` |
| **AiQuery** | Natural language or unknown command — routes to AI | `"find large files"` |
| **NotFound** | Looks like a command but not found | `gti status` (typo) |
| **Clear** | Clear the screen | `clear`, `cls` |
| **Exit** | Exit Codiv | `exit`, `quit` |

## How Commands Are Routed

```
User Input
    │
    ▼
┌─────────────────┐
│  Command Index   │──── known? ──── yes ──▶ Execute immediately
│  (hash lookup)   │                         via bash co-process
└─────────────────┘
    │ no
    ▼
┌─────────────────┐
│  Interactive     │──── match? ──── yes ──▶ PTY passthrough
│  Detection       │
└─────────────────┘
    │ no
    ▼
┌─────────────────┐
│  AI Agent        │──── route to daemon
│  (via IPC)       │     for LLM processing
└─────────────────┘
```

## Interactive Passthrough

Programs that take over the full terminal (like `vim`, `ssh`, `python`, `top`) are detected and handed off to the PTY in passthrough mode. Codiv steps aside and lets the program control the terminal directly until it exits.

## Command Not Found

When input looks like a command but is not in the index, Codiv reports it as not found — similar to a regular shell. You can still ask the AI about it by rephrasing as a natural language query.
