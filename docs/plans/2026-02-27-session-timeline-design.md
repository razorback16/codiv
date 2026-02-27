# Unified Session Timeline Design

**Date**: 2026-02-27
**Status**: Implemented

## Problem

The daemon creates a new Agent per request, losing all conversation history. The aisdk's native multi-turn `Message` API is unused — instead, history is stuffed into a single prompt string with "User: / Assistant:" prefixes. Shell command context (cwd, recent commands, env vars) is sent but ignored.

## Goal

Build a unified session timeline where shell commands, user AI queries, and AI responses exist in a single conversation. The AI sees what the user has been doing in their terminal, enabling context-aware responses.

## Approach: Shell Commands as Message Pairs

Shell commands are represented as assistant/user message pairs in the aisdk message history, inspired by synthetic tool call patterns used by Claude Code and other agent frameworks. Rather than using actual `ToolCallInfo`/`ToolResultInfo` objects (which require the assistant to have explicitly requested the tool call), shell commands are embedded as structured text messages that convey the same context.

### Data Model

```rust
pub enum SessionEvent {
    ShellCommand {
        command: String,
        output: String,       // truncated stdout+stderr
        exit_code: i32,
        cwd: String,
    },
    UserQuery(String),
    AssistantResponse(String),
}
```

The `Agent` struct holds `pub history: Vec<SessionEvent>` instead of `Vec<ConversationMessage>`.

### Message Conversion

When building the aisdk request, `SessionEvent`s convert to `Message` objects via `Message::builder()`:

| Event | aisdk Message |
|-------|---------------|
| `ShellCommand` | `builder.assistant("Running command: {cmd} (in {cwd})")` + `builder.user("[Terminal output for \`{cmd}\`]:\n{output}")` |
| `UserQuery` | `builder.user(text)` |
| `AssistantResponse` | `builder.assistant(text)` |

Non-zero exit codes append `[exit code: N]` to the output. The system prompt tells the AI it is embedded in a terminal and can see recent command history in the conversation.

### Client → Daemon Communication

The client sends shell command results to the daemon in real-time via a new `ClientMessage::CommandResult` variant, rather than batching them with AI requests. This keeps the daemon's timeline always up-to-date.

```rust
ClientMessage::CommandResult {
    command: String,
    output: String,
    exit_code: i32,
    cwd: String,
}
```

### Output Truncation

A reusable utility in `slate-common` truncates long command output:
- Keep first N lines (default 20) and last N lines (default 20)
- If output fits within head + tail, return unchanged
- Otherwise insert `... (X lines omitted) ...` in the middle
- Applied when storing `CommandResult` in the session timeline
- Designed for reuse with future agent bash tool results

### Bounded History

The timeline is capped at 100 most recent events (`MAX_HISTORY_EVENTS`). Older events are dropped from the front.

### Concurrency

The existing take/return oneshot pattern continues — if the agent is currently streaming a response and a new request arrives, a fresh agent is created (acceptable for now since concurrent AI queries from a single terminal are unlikely).

## Files to Modify

| File | Change |
|------|--------|
| `crates/slate-common/src/messages.rs` | Add `ClientMessage::CommandResult` variant |
| `crates/slate-common/src/truncate.rs` | New: reusable output truncation utility |
| `crates/slate-common/src/lib.rs` | Export `truncate` module |
| `crates/slated/src/agent/agent.rs` | `SessionEvent` enum, `build_messages()` with synthetic tool calls, use aisdk `Messages` API |
| `crates/slated/src/agent/config.rs` | Accept `Messages` instead of `&str` prompt; use `.messages()` builder |
| `crates/slated/src/daemon.rs` | Handle `CommandResult` in dispatch, append to session agent timeline |
| `crates/slate/src/ui/terminal.rs` | Send `CommandResult` after shell commands complete |
