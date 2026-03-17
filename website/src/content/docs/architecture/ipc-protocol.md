---
title: IPC Protocol
description: Wire format, message types, and communication flows between client and daemon.
section: Architecture
order: 3
---

## Wire Format

All IPC messages use a simple length-prefixed binary format:

```
┌──────────────┬──────────────────────────┐
│ 4 bytes (BE) │ bincode payload          │
│ length       │ (serde-serialized)       │
└──────────────┴──────────────────────────┘
```

- **Length** — 4-byte big-endian unsigned integer indicating the payload size in bytes
- **Payload** — bincode-serialized Rust struct

This keeps parsing simple and avoids delimiter-based framing issues with binary data.

## Message Types

| Message | Direction | Purpose |
|---------|-----------|---------|
| `EnvSnapshot` | Client to Daemon | Sends current environment variables, working directory, terminal size |
| `Heartbeat` | Bidirectional | Keep-alive ping/pong |
| `AgentRequest` | Client to Daemon | User query with session context |
| `AgentStreamChunk` | Daemon to Client | Streaming token from LLM response |
| `AgentComplete` | Daemon to Client | Agent finished processing |
| `ConfirmationRequest` | Daemon to Client | Ask user to approve a risky command |
| `ConfirmationResponse` | Client to Daemon | User's approval or denial |
| `CommandResult` | Daemon to Client | Result of a tool execution |
| `ExecuteCommand` | Daemon to Client | Route AI bash command through client's co-process |
| `CommandExecutionResult` | Client to Daemon | Relay result with output, exit code, and updated cwd |

## Agent Task Flow

A typical AI query follows this sequence:

```mermaid
sequenceDiagram
    participant C as Client
    participant D as Daemon
    C->>D: AgentRequest
    Note right of D: LLM call
    D-->>C: AgentStreamChunk (reasoning text)
    D-->>C: AgentStreamChunk
    Note right of D: Tool call (e.g., grep)
    D-->>C: AgentStreamChunk (tool result summary)
    Note right of D: LLM call (with tool result)
    D-->>C: AgentStreamChunk (final response)
    D->>C: AgentComplete
```

## Safety Confirmation Flow

When the agent wants to run a command classified as risky:

```mermaid
sequenceDiagram
    participant C as Client
    participant D as Daemon
    Note right of D: Agent wants to run\n"rm -rf target/"
    D->>C: ConfirmationRequest (command, risk level, explanation)
    Note left of C: User sees prompt in TUI
    C->>D: ConfirmationResponse (allow / deny / always-allow)
    Note right of D: Executes or skips based on response
    D-->>C: AgentStreamChunk (continues)
```

## Command Relay Flow

When the orchestrator agent runs a bash command, it is relayed through the client's co-process so that shell state is shared with the user:

```mermaid
sequenceDiagram
    participant A as Agent (Daemon)
    participant D as Daemon IPC
    participant C as Client IPC
    participant S as Bash Co-Process

    A->>D: bash tool call
    D->>C: ExecuteCommand (command, cwd)
    C->>S: Write command to co-process
    S-->>C: Output + exit code
    C->>D: CommandExecutionResult (output, exit_code, new_cwd)
    D->>A: Tool result
```

Independent agents (engineer, reviewer) bypass this relay and execute commands in their own daemon-side `DaemonShell` processes.

## Versioning

The IPC protocol is versioned implicitly through the shared `codiv-common` crate. Since both client and daemon are built from the same workspace, they always use matching message definitions. The `EnvSnapshot` message includes a protocol version field for future compatibility when independent versioning is needed.
