---
title: AI Mode
description: "How AI mode works: querying the agent, tool calling, and session context."
section: Usage
order: 2
---

## How It Works

AI mode is the default mode in Codiv. When the gutter shows `>` in cyan, everything you type is sent to the daemon's AI agent over IPC.

The agent then:

1. Receives your query with the current session context
2. Reasons about what to do
3. Calls tools as needed (bash, read, write, edit, grep, glob)
4. Streams the response back to your terminal in real time

AI mode is active on startup. Press Tab on empty input to switch to Command mode, and Tab again to return.

## Streaming Responses

AI responses stream token-by-token to the TUI as they are generated. You see the agent's thinking and output appear progressively, rendered as markdown with syntax highlighting.

## Tool Calling Loop

The agent operates in a loop:

```
Query ──▶ LLM Reasoning ──▶ Tool Call ──▶ Tool Result ──▶ LLM Reasoning ──▶ ...
                                                              │
                                                              ▼
                                                        Final Response
```

Each iteration, the LLM decides whether to call another tool or produce a final response. This allows multi-step tasks: the agent might grep for a pattern, read the matching file, edit it, and then run tests — all from a single query.

## Session Timeline

Codiv maintains a unified timeline that includes:

- Shell commands you have run and their output
- AI queries and responses
- Tool calls made by the agent

This context flows into the LLM prompt, so the agent is aware of what you have been doing. If you just ran `cargo test` and some tests failed, the agent knows about it when you ask "fix the failing test."

## Safety Confirmations

When the agent wants to run a potentially dangerous command (like `rm`, `git push --force`, or writing to system files), Codiv pauses and asks for confirmation:

```
Agent wants to run: rm -rf target/
Risk level: MEDIUM

[y] Allow  [n] Deny  [a] Always allow this command
```

You can allow the command once, deny it, or add it to your permanent allowlist. See the [Safety & Audit](/slate-agent/docs/design-docs/safety-audit/) design doc for details on risk classification.
