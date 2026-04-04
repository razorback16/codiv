---
title: Client & Daemon
description: Detailed look at the TUI client, async daemon, and how they communicate.
section: Architecture
order: 2
---

## Client (codiv)

The client is a terminal application built with [ratatui](https://ratatui.rs/). It handles:

### Terminal Rendering

- Full TUI with scrollable output panes
- Markdown rendering via streamdown-rs with syntax highlighting
- Terminal colorscheme detection for proper theming

### Bash Co-Process

A persistent bash child process runs alongside the TUI. When you type a known command, it is sent directly to this co-process for instant execution — no daemon round-trip needed.

The co-process also handles AI-relayed commands via the shell lease protocol. When the orchestrator agent needs to run a bash command, it first acquires a lease on the client's shell. The daemon sends an `AcquireShellLease` message, the client responds with `ShellLeaseAcquired`, and the daemon can then send `ExecuteLeasedCommand` messages through the leased shell. When the agent is done, it sends `ReleaseShellLease` to free the shell for other use. If multiple agents request the shell concurrently, leases are queued and granted in order. Leases have timeouts to prevent indefinite blocking. This means the AI agent shares shell state (environment variables, working directory, etc.) with the user while maintaining safe concurrent access.

### Command Index

At startup, the client builds a hash map of all known commands:

- Executables from every directory on `PATH`
- Bash and zsh builtins

This allows O(1) lookup to classify input as a known command or an AI query.

### Tab Completion

Three tiers of tab completion:

1. **Programmable** — bash-completion scripts for commands that support them
2. **Command** — completes command names from the index
3. **File** — filesystem path completion

## Daemon (codivd)

The daemon is a Tokio async application that handles all AI and tool operations:

### Agent Sessions

Each AI query creates a session with context from the terminal timeline. The agent reasons, calls tools, and streams results back to the client.

### LLM Streaming

Responses stream token-by-token from the LLM provider via the aisdk library. Supports Anthropic, OpenAI, and Google providers with a unified interface.

### Worker Bash Sessions

The orchestrator agent does not use a separate worker session — its bash commands route through the client's co-process via the shell lease protocol (see above), so shell state is shared with the user. Independent agents (engineer, reviewer, etc. in multi-agent mode) get their own daemon-side `DaemonShell` instances — pipe-based `bash -i` processes with fresh state, inheriting only the initial working directory from the parent.

### Background Token Refresh

The daemon runs a background token refresh cycle that checks OAuth tokens every 30 seconds. When a token is nearing expiration, the daemon automatically refreshes it using the stored refresh token. This ensures that long-running agent sessions never fail mid-stream due to an expired OAuth credential. The refresh cycle covers all configured OAuth providers (Claude Code, Codex, etc.).

## IPC Communication

The client and daemon communicate over a Unix domain socket using:

- **Serialization** — serde + bincode (compact binary format)
- **Framing** — 4-byte big-endian length prefix followed by the payload
- **Lifecycle** — the client connects at startup, maintains a heartbeat, and reconnects if the connection drops

### Connection Lifecycle

```mermaid
sequenceDiagram
    participant C as Client
    participant D as Daemon
    C->>D: Connect
    D->>C: Accept
    C->>D: EnvSnapshot
    loop Periodic
        C->>D: Heartbeat
        D->>C: Heartbeat
    end
    C->>D: AgentRequest
    D-->>C: StreamChunk (repeated)
    Note right of D: Agent needs bash
    D->>C: AcquireShellLease
    C->>D: ShellLeaseAcquired
    D->>C: ExecuteLeasedCommand
    Note left of C: Runs in co-process
    C->>D: CommandCompleted
    D->>C: ReleaseShellLease
    D-->>C: StreamChunk (continued)
    D->>C: AgentComplete
```

The heartbeat ensures both sides detect connection loss promptly. If the daemon restarts, the client reconnects and re-sends the environment snapshot.
