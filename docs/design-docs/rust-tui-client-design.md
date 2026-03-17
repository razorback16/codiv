# Rust TUI Client — Design Document

**Date**: 2026-02-23
**Author**: Subhagato
**Status**: Draft

---

## 1. Overview

Replace the FTXUI-based C++ terminal client (`codiv`) with a Rust binary built on **ratatui + crossterm**. The daemon (`codivd`) has been rewritten to Rust (see [Rust Daemon Design](../plans/2026-02-26-rust-daemon-aisdk-design.md)). The two processes communicate over a Unix domain socket using serde+bincode IPC.

```
codiv (Rust binary)
  - ratatui + crossterm terminal UI
  - PTY management for shell passthrough
  - Markdown/syntax highlighting renderer
  - Multi-agent inline block rendering
  - serde+bincode IPC client
          ↕ Unix socket + serde+bincode
codivd (Rust daemon)
  - Agent orchestration, memory, scheduler
  - Worker bash sessions, tool execution
  - LLM access (aisdk.rs)
```

Both client and daemon are Rust, sharing types via the `codiv-common` crate. IPC uses serde+bincode over Unix socket.

---

## 2. Motivation

### Why not FTXUI

1. **Complexity overhead** — building a custom TUI in FTXUI is too much work for the core value prop (the AI agent, not the terminal).
2. **Limited rendering** — FTXUI can't easily handle rich markdown, syntax highlighting, or streaming code blocks.
3. **Raw terminal conflict** — FTXUI's rendering model conflicts with raw VT100 passthrough. The abandoned `StreamView` attempt confirmed this.
4. **Ecosystem** — FTXUI has far fewer widgets, examples, and community support than ratatui.

### Why Rust

1. **ratatui ecosystem** — extensive community support, widgets, examples. Zellij proves the stack handles all requirements at scale.
2. **syntect** — production-grade syntax highlighting (same engine as Sublime Text). No equivalent in C++ TUI space.
3. **comrak** — full CommonMark + GFM rendering. Robust markdown-to-terminal pipeline.
4. **PTY handling** — well-supported via `portable-pty` or raw `nix` crate. Zellij demonstrates this at scale.
5. **Memory safety** — the UI layer handles user input, network data, PTY output — all untrusted. Rust prevents entire classes of bugs.
6. **Thin client** — the Rust binary is just UI + IPC. The language boundary is a Unix socket, which is about as clean as it gets.

---

## 3. UI Model: Linear Scroll-Down Flow

The key design decision: **no multi-pane split layout**. Everything flows linearly downward like a normal terminal. Agent work expands inline as structured blocks below the user's prompt.

### Flow

```
$ ls -la                               ← Command mode: user types, output below
total 48
drwxr-xr-x  12 user staff  384 Feb 23 .
-rw-r--r--   1 user staff 1200 Feb 23 auth.ts

> refactor auth module to use JWT       ← AI mode: natural language goes to agent

── Task: refactor auth module to use JWT ──────────────
│
├─ WI#1 Plan decomposition      ✓ TeamLead
├─ WI#2 Implement JWT auth      ● Engineer #1 (cyan)
│  └─ WI#5 Write auth tests     ○ blocked by WI#2
├─ WI#3 Update README           ✓ Engineer #2 (green)
├─ WI#4 Security audit          ◐ Reviewer (yellow)
│
● running  ◐ in review  ○ pending  ✓ done

┌ Engineer #1 ── ◆ cyan ── WI#2 ─────────────────────┐
│ $ npm install jsonwebtoken                           │
│ added 12 packages in 2.1s                            │
│ $ edit src/auth.ts [+15 -3]                         │
│ ████████████░░░░░░░░ 60%                             │
└──────────────────────────────────────────────────────┘

┌ Engineer #2 ── ◆ green ── WI#3 ────────────────────┐
│ ✓ README.md updated (collapsed — Enter to expand)    │
└──────────────────────────────────────────────────────┘

┌ Reviewer ── ◆ yellow ── WI#4 ──────────────────────┐
│ Reviewing auth.ts changes...                         │
│ ⚠ Missing error handling on token expiry (line 42)   │
└──────────────────────────────────────────────────────┘

> _                                     ← back to AI mode prompt when done
```

### Key Behaviors

1. **Everything scrolls down** — user input, shell output, and agent blocks flow in one linear stream, like a terminal.
2. **Shell commands** render output directly below the prompt, exactly like a normal terminal.
3. **Agent tasks** expand inline as structured blocks:
   - Task header with the goal.
   - DAG tree showing all work items, status, assigned agent — live-updating in place.
   - Agent output blocks — color-bordered boxes, one per active agent, streaming their work.
   - Completed agent blocks auto-collapse to a single summary line (expandable with Enter).
4. **Auto-scroll** — viewport always follows new content at the bottom. User can scroll up to read history.
5. **Interactive commands** (vim, ssh, htop) — ratatui suspends entirely, terminal goes raw, child process owns the screen. On exit, ratatui resumes and the session continues.

---

## 4. Rendering Components

### Markdown Rendering

Agent output is streamed as markdown. The rendering pipeline: `comrak` parses CommonMark + GFM, then maps to styled terminal output:

- Syntax-highlighted code blocks (via `syntect`).
- Bold, italic, lists, headers mapped to ANSI styles.
- Inline diffs with red/green coloring.

### DAG Tree Widget

Unicode box-drawing characters. Updates in-place (cursor repositioning within the block) as statuses change:

| Symbol | Status |
|--------|--------|
| `●` | Running |
| `◐` | In review |
| `○` | Pending |
| `✓` | Done |

### Agent Output Blocks

Color-bordered boxes using Unicode box-drawing:

- Each agent gets a unique color from palette.
- Stream content line-by-line as it arrives from daemon.
- Auto-collapse to single summary line when work item completes.
- Expandable with Enter after completion.

### Color Scheme

| Role | Color |
|------|-------|
| Engineer | Cyan (cycle cyan variants for multiple) |
| Reviewer | Yellow |
| Security | Red |
| Researcher | Magenta |
| Perf | Blue |
| TeamLead | White/bold |
| User commands | Default terminal color, no border |

### Progress Indicators

Block progress bars (`████░░░░`), spinners (`⠋⠙⠹`), percentage for long-running operations.

### PTY Passthrough

For interactive commands: ratatui suspends, terminal goes raw, child process owns screen. On exit, ratatui resumes.

---

## 5. IPC Protocol

Unix domain socket with 4-byte length prefix + bincode payload. Both client and daemon use the `codiv-common` crate for shared IPC types (serde + bincode serialization).

---

## 6. Crate Dependencies

| Crate | Purpose |
|-------|---------|
| `ratatui` | Terminal UI framework |
| `crossterm` | Cross-platform terminal manipulation |
| `syntect` | Syntax highlighting for code blocks |
| `comrak` | CommonMark markdown parsing |
| `codiv-common` | Shared IPC types (serde + bincode) |
| `serde` | Serialization framework |
| `bincode` | Binary serialization format |
| `tokio` | Async runtime for IPC + PTY I/O |
| `nix` | Unix system calls (PTY, signals) |
| `portable-pty` | PTY management |

---

## 7. Change Summary

### Changes (Rust client replaces FTXUI client)

| Component | Before | After |
|-----------|--------|-------|
| Terminal UI framework | FTXUI | ratatui + crossterm |
| Language for `codiv` binary | C++ | Rust |
| Markdown rendering | cmark-gfm + tree-sitter | comrak + syntect |
| Build system (codiv only) | CMake | Cargo |

### Unchanged

- `codivd` daemon — rewritten to Rust (see daemon design doc).
- serde+bincode IPC protocol — shared via `codiv-common` crate.
- Unix domain socket transport.
- Persistent bash co-process model. The coprocess now also handles AI-relayed commands via the command relay protocol. When the daemon sends an `ExecuteCommand` IPC message, the client queues it as a `PendingAiExecution` and runs it through the same coprocess. AI command output is invisible in the terminal — the VT parser's `process` step is skipped for relayed executions, so the user's scrollback is not polluted. The result (output, exit code, updated cwd) is sent back to the daemon via `CommandExecutionResult`.
- Dual-mode input logic.
- Env snapshot protocol.
- All agent/memory/scheduler logic in daemon.
