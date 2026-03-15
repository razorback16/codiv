# Codiv Agent — One-Pager

**Date**: 2026-02-26 | **Author**: Subhagato | **Status**: Draft

---

### Problem

- Developers constantly context-switch between their terminal and AI coding tools — breaking flow and wasting time.
- Existing AI-in-terminal tools (Claude Code, Codex CLI, Aider) are single-model, flat-architecture systems that process every task with the same model regardless of complexity.
- No tool today lets you type a shell command and have it execute instantly (no AI round-trip) while also handling complex AI tasks in the same interface.

### Target User

Terminal-centric developers — senior backend engineers, full-stack developers, and DevOps practitioners who live in the terminal and want AI to meet them there, not pull them into a separate app or editor.

### Core Solution

A terminal that **is** your shell. Codiv Agent replaces your terminal emulator with one that:

- **Runs commands instantly** — recognized shell commands execute with near-zero overhead, no different from a regular terminal.
- **Handles AI tasks natively** — natural-language requests are automatically decomposed into a dependency graph of work items, assigned to specialized agent roles (planning, coding, review, research), and executed in parallel.
- **Assigns the right model to each job** — a configurable model catalog lets the system match task complexity to model capability and cost, rather than routing everything through a single LLM.

### Why Now / Differentiation

| Capability | Claude Code / Codex CLI | Aider / Cursor | Codiv Agent |
|---|---|---|---|
| Instant command execution (no AI overhead) | No | No | Yes |
| Multi-agent task decomposition | No | No | Yes |
| Multi-model assignment per task | No | No | Yes |
| Native terminal experience | Partial (runs inside a terminal) | No (editor-based) | Yes (is the terminal) |

The building blocks have converged: fast frontier models make real-time agent orchestration viable, multi-provider APIs enable model selection by task, and Rust/C++ give the performance budget to keep command execution imperceptible.

### Success Metrics

- **< 10 ms** overhead on recognized command execution (indistinguishable from a raw shell)
- **< 500 ms** first-token latency on AI responses
- **4+ parallel workers** executing agent tasks concurrently
- Users report **not needing to leave the terminal** for AI-assisted coding workflows

### MVP Scope

TODO:

### Architecture (one sentence)

Rust terminal client for rendering and user interaction, C++ singleton daemon for agent orchestration and worker management, connected via binary IPC over a Unix domain socket.

### Monetization Hypothesis

Open-source core with a future paid tier for team features (shared project memory, centralized model routing, usage dashboards). The terminal itself is always free.
