# IPC Protocol — Design Document

**Date**: 2026-02-26
**Author**: Subhagato
**Status**: Draft
**Refines**: Phase 1 (IPC Foundation), Phase 2 (Agent Messages), Phase 3+ (Multi-Agent Coordination)

---

## 1. Overview

The `codiv-common` crate (`crates/codiv-common/src/messages.rs`) defines the wire format using serde + bincode for communication between `codiv` (TUI client) and `codivd` (Rust daemon). This document defines the protocol semantics — when messages are sent, what responses are expected, how errors are handled, and how the protocol evolves over time.

This is the authoritative reference for IPC behavior. The schema defines *what* can be sent; this document defines *when* and *why*.

## 2. Problem

The Phase 1 IPC implementation works but the protocol semantics are implicit in the code. As the protocol grows to support agent messages, safety confirmation flows, streaming output, and multi-agent coordination, the implicit contracts become a liability:

- No documented message flow for new contributors to follow
- No error handling contract — what happens when a message is malformed or a session is unknown?
- No reconnection strategy — what happens when codiv loses its connection to codivd?
- No versioning strategy — how does the protocol evolve without breaking existing clients?

This design makes all of these explicit.

## 3. Transport Layer

### 3.1 Socket

Unix domain socket at:

```
/tmp/codivd-{uid}.sock
```

Where `{uid}` is the numeric user ID of the running user. One daemon per user, one socket per daemon.

No TLS — the socket is local-only, protected by filesystem permissions (user-only access). Same-user communication does not require encryption.

### 3.2 Framing

Every message on the wire is framed as:

```
┌──────────────────┬─────────────────────────┐
│ 4 bytes (u32 BE) │ N bytes (bincode)       │
│ payload length   │ payload                 │
└──────────────────┴─────────────────────────┘
```

- Length prefix: 4-byte unsigned integer, big-endian byte order
- Payload: bincode-encoded message as defined in `codiv-common/src/messages.rs`
- Maximum message size: 16 MB (configurable, but exceeding this indicates a bug)

### 3.3 Connection Lifecycle

```
codiv                          codivd
  |--- connect() -------------->|
  |--- EnvSnapshot ------------>|  (handshake: session_id, env, protocol_version)
  |<-- (implicit ack) ----------|  (connection accepted)
  |                             |
  |  [session active]           |
  |                             |
  |--- Heartbeat -------------->|  (every 5s)
  |--- Heartbeat -------------->|
  |  ...                        |
  |                             |
  |--- Shutdown --------------->|  (graceful disconnect)
  |<-- Shutdown ----------------|  (ack)
  |--- close() ---------------->|
```

## 4. Message Types

### 4.1 Phase 1 (Implemented)

#### EnvSnapshot
- **Direction**: codiv → codivd
- **When**: on initial connect and whenever the environment changes (e.g., user changes directory, shell env updated)
- **Contains**: session_id, env_vars (HashMap), PATH, cwd
- **Response**: none (fire-and-forget); daemon updates its internal session state
- **Future extension**: protocol_version field for handshake negotiation

#### Heartbeat
- **Direction**: codiv → codivd
- **Interval**: every 5 seconds
- **Contains**: session_id, timestamp (Unix millis)
- **Response**: none (daemon updates last-seen timestamp internally)
- **Purpose**: detect stale connections; daemon cleans up sessions with no heartbeat for 30 seconds

#### Shutdown
- **Direction**: bidirectional
- **When**: graceful disconnect initiated by either side
- **Contains**: session_id (optional — omit for daemon-wide shutdown)
- **Protocol**: sender sends Shutdown, receiver sends Shutdown as acknowledgment, then both close the connection

#### Error
- **Direction**: bidirectional
- **When**: an error occurs that the other side needs to know about
- **Contains**: error_code (u16), description (string)
- **Behavior**: informational — the receiver logs the error; connection may or may not be closed depending on severity

### 4.2 Phase 2 (Planned)

#### AgentRequest
- **Direction**: codiv → codivd
- **When**: user submits natural language input for agent processing
- **Contains**: session_id, prompt, request_id, context (SessionContext with cwd, recent_commands, env_vars)
- **Response**: one or more AgentStreamChunk messages followed by exactly one AgentComplete
- **SessionContext**: provides the daemon with the client's current working directory, a list of recent CommandRecord entries (command, output, exit_code, timestamp), and relevant environment variables
- **CommandRecord**: each entry captures a previously executed command with its output, exit code, and timestamp

#### AgentStreamChunk
- **Direction**: codivd → codiv
- **When**: the agent produces output (text, tool activity, thinking)
- **Contains**: session_id, work_item_id, chunk_type, data
- **Chunk types**:
  - `text`: markdown text from the agent's response
  - `tool_start`: agent is about to invoke a tool (includes tool name and arguments)
  - `tool_end`: tool execution completed (includes result summary)
  - `thinking`: agent's reasoning trace (displayed in a collapsible section)
- **Delivery**: streaming, ordered within a single work item

#### AgentComplete
- **Direction**: codivd → codiv
- **When**: the agent has finished processing the request
- **Contains**: session_id, summary (string), artifact_ids (list of created/modified files)
- **Guarantees**: exactly one AgentComplete per AgentRequest; no further AgentStreamChunk after this

#### ConfirmationRequest
- **Direction**: codivd → codiv
- **When**: the agent attempts a high or critical risk action (see Safety & Audit Design)
- **Contains**: session_id, work_item_id, request_id, command, risk_level, description
- **Response**: exactly one Confirmation with matching request_id
- **Timeout**: if no response within 60 seconds, daemon treats it as rejected

#### Confirmation
- **Direction**: codiv → codivd
- **When**: user responds to a safety confirmation prompt
- **Contains**: session_id, request_id, approved (bool), add_to_allowlist (bool), add_to_denylist (bool)
- **Constraint**: request_id must match a pending ConfirmationRequest

#### ExecuteCommand
- **Direction**: codivd → codiv
- **When**: the Orchestrator's Bash tool needs to run a command through the client's shared PTY coprocess (command relay for shared shell state)
- **Contains**: execution_id (unique correlation ID), command (string), timeout_ms (u64)
- **Response**: exactly one CommandExecutionResult with matching execution_id
- **Purpose**: enables the AI to execute commands in the same shell environment as the user, so state changes (cd, export, etc.) are immediately visible to both sides

#### CommandExecutionResult
- **Direction**: codiv → codivd
- **When**: the client has finished executing a relayed command through its coprocess
- **Contains**: execution_id, output (string), exit_code (i32), cwd (string — the working directory after execution)
- **Constraint**: execution_id must match a pending ExecuteCommand
- **Purpose**: returns the command output and updated cwd to the daemon, which resolves the waiting tool closure via a oneshot channel

### 4.3 Phase 3+ (Planned)

#### WorkItemUpdate
- **Direction**: codivd → codiv
- **When**: a Work Item changes state in the execution DAG
- **Contains**: session_id, work_item_id, state (pending/running/completed/failed/blocked), assigned_role (string), model_id (string), progress_pct (u8)
- **Purpose**: enables codiv to render a live DAG visualization of multi-agent task execution

#### TaskTreeUpdate
- **Direction**: codivd → codiv
- **When**: the full task DAG is created or restructured
- **Contains**: session_id, work_items (array of { id, goal, state, role, dependencies })
- **Purpose**: provides codiv with the complete DAG structure for initial rendering or after a restructure

## 5. Message Flow Diagrams

### 5.1 Command Execution

**Note**: Direct command execution is handled locally by `codiv`'s persistent bash co-process. The daemon is not involved in command execution — it only handles agent requests.

### 5.2 Agent Task (Phase 2)

```
codiv                          codivd
  |--- AgentRequest ----------->|  (prompt, SessionContext: cwd, recent_commands, env_vars)
  |<-- AgentStreamChunk --------|  (text: "I'll create the file...")
  |<-- AgentStreamChunk --------|  (tool_start: "Bash: echo hello > hello.txt")
  |<-- AgentStreamChunk --------|  (tool_end: "exit code 0")
  |<-- AgentStreamChunk --------|  (text: "File created successfully.")
  |<-- AgentComplete -----------|
```

### 5.3 Safety Confirmation (Phase 2)

```
codiv                          codivd
  |<-- AgentStreamChunk --------|  (tool_start: "Bash: rm -rf ./build/")
  |<-- ConfirmationRequest -----|  (risk: high)
  |--- Confirmation ----------->|  (approved: true)
  |<-- AgentStreamChunk --------|  (tool_end: "exit code 0")
```

If the user rejects:

```
codiv                          codivd
  |<-- ConfirmationRequest -----|  (risk: high)
  |--- Confirmation ----------->|  (approved: false)
  |<-- AgentStreamChunk --------|  (text: "Action rejected. Finding alternative...")
```

### 5.4 Multi-Agent Task (Phase 3+)

```
codiv                          codivd
  |--- AgentRequest ----------->|
  |<-- TaskTreeUpdate ----------|  (3 Work Items created)
  |<-- WorkItemUpdate ----------|  (WI-1: running, Engineer)
  |<-- WorkItemUpdate ----------|  (WI-2: running, Engineer)
  |<-- AgentStreamChunk --------|  (WI-1 output streaming)
  |<-- AgentStreamChunk --------|  (WI-2 output streaming)
  |<-- WorkItemUpdate ----------|  (WI-1: completed)
  |<-- WorkItemUpdate ----------|  (WI-3: running, Reviewer)
  |<-- WorkItemUpdate ----------|  (WI-2: completed)
  |<-- WorkItemUpdate ----------|  (WI-3: completed)
  |<-- AgentComplete -----------|
```

### 5.5 Heartbeat and Timeout

```
codiv                          codivd
  |--- Heartbeat -------------->|  (t=0s)
  |--- Heartbeat -------------->|  (t=5s)
  |--- Heartbeat -------------->|  (t=10s)
  |  [network issue]            |
  |                             |  (t=40s: no heartbeat for 30s)
  |                             |  daemon cleans up session
  |                             |
  |--- Heartbeat -------------->|  (t=45s: reconnect attempt)
  |--- EnvSnapshot ------------>|  (re-handshake)
```

### 5.6 Command Relay (Shared Shell)

```
codiv                          codivd
  |                             |  [LLM tool call: Bash("cd /tmp && ls")]
  |<-- ExecuteCommand ----------|  (execution_id, command, timeout_ms)
  |  [runs through coprocess]   |
  |--- CommandExecutionResult ->|  (execution_id, output, exit_code, cwd="/tmp")
  |                             |  [daemon resolves oneshot, returns result to LLM]
```

The command relay enables the Orchestrator agent to share shell state with the user. When the daemon's Bash tool uses `ShellBackend::ClientRelay`, it sends an `ExecuteCommand` IPC message to the client, which queues it as a `PendingAiExecution` and runs it through the same PTY coprocess used for user commands. The client sends back a `CommandExecutionResult` with the output, exit code, and updated cwd. AI command output is not displayed in the terminal (the PTY parser's `process` step is skipped for relayed commands).

Independent agents (Engineers, Reviewers) use `ShellBackend::Local` with a `DaemonShell` — a pipe-based `bash -i` process on the daemon side — and do not participate in the command relay.

## 6. Error Handling

### 6.1 Error Codes

| Code | Name                  | Description                                        |
|------|-----------------------|----------------------------------------------------|
| 0    | SUCCESS               | No error (used in acknowledgments)                 |
| 1    | UNKNOWN_ERROR         | Unclassified error                                 |
| 2    | SESSION_NOT_FOUND     | The session_id does not match any active session   |
| 3    | INVALID_MESSAGE       | bincode payload could not be deserialized           |
| 4    | OVERLOADED            | Daemon is at capacity, cannot accept new work      |
| 5    | TIMEOUT               | Operation timed out                                |
| 6    | BUDGET_EXCEEDED       | Token or cost budget exhausted for this session    |
| 7    | CONFIRMATION_DENIED   | A required safety confirmation was rejected        |

### 6.2 Error Behaviors

**Unknown message type**: Log a warning and ignore the message. This is the forward-compatibility rule — new message types added in future protocol versions are silently ignored by older clients/daemons.

**Malformed payload**: Send an Error message with code INVALID_MESSAGE. Close the connection. A malformed payload indicates a serious bug or version mismatch; continuing is unsafe.

**Session not found**: Send an Error message with code SESSION_NOT_FOUND. The client should re-send an EnvSnapshot to establish a new session.

**Daemon overloaded**: Send an Error message with code OVERLOADED. Codiv displays a "daemon busy" status message. The client may retry after a backoff.

**Budget exceeded**: Send an Error message with code BUDGET_EXCEEDED. The agent stops execution. Codiv displays the budget status and asks the user whether to increase the budget or abort.

**Confirmation denied**: Sent as part of normal agent flow (not an error in the connection sense). The agent receives this and must find an alternative approach.

## 7. Reconnection

### 7.1 Detection

Codiv detects a lost connection when:

- The TCP write fails (socket closed)
- No daemon response to heartbeats for 30 seconds
- An explicit connection reset

### 7.2 Strategy

Three reconnection attempts with exponential backoff:

| Attempt | Delay  |
|---------|--------|
| 1       | 1s     |
| 2       | 2s     |
| 3       | 4s     |

On each attempt:
1. Connect to the Unix domain socket
2. Re-send EnvSnapshot with the existing session_id
3. If the daemon recognizes the session: resume (pending output is replayed)
4. If the daemon has restarted: session is lost; notify the user and start a fresh session

### 7.3 User Experience During Reconnection

- Codiv shows a "Reconnecting..." status indicator
- User input is buffered during reconnection attempts
- If all 3 attempts fail: Codiv shows "Connection lost. Daemon may have stopped." with instructions to restart codivd
- Buffered input is discarded on total failure (user is notified)

## 8. Versioning

### 8.1 Protocol Version

A protocol_version field is included in the EnvSnapshot message (the initial handshake). This is an integer that increments on breaking changes.

```
protocol_version = 1   # Phase 1
protocol_version = 2   # Phase 2 (agent messages, confirmation)
protocol_version = 3   # Phase 3 (multi-agent, DAG updates)
```

### 8.2 Compatibility Rules

**Additive changes (non-breaking)**:
- New message types: old clients ignore unknown types (Section 6.2)
- New fields in existing messages: serde supports adding fields with `#[serde(default)]`; old clients read only the fields they know

**Breaking changes**:
- Removing a message type
- Changing the semantics of an existing field
- Changing the framing format

Breaking changes increment the protocol_version. The daemon rejects connections from clients with an incompatible version, returning an Error with a clear message indicating the version mismatch.

### 8.3 serde + bincode Schema Evolution

serde + bincode support schema evolution with these rules:

- New fields are added to structs with `#[serde(default)]` so older payloads missing the field deserialize successfully with a default value
- New enum variants can be appended to message enums; older clients that encounter an unknown variant treat it as an unrecognized message (see Section 6.2)
- Existing fields are never removed or retyped in a non-breaking release; deprecated fields are kept with `#[serde(default)]` and ignored at runtime
- For complex migrations, the `ClientMessage` and `DaemonMessage` enums can carry versioned variants (e.g., `AgentRequestV2`) while retaining the original variant for backward compatibility

This aligns naturally with the additive compatibility model.

## 9. Performance Considerations

### 9.1 Streaming Chunk Size

Streaming chunks (AgentStreamChunk) should be at least 64 bytes to avoid excessive IPC overhead from per-message framing and syscall costs.

**Batching strategy**: buffer output for 10ms or until 64 bytes accumulate, whichever comes first. This balances latency (user sees output within 10ms) against throughput (avoids sending one-byte messages for slow-dripping output).

### 9.2 bincode Serialization

bincode serialization is allocation-light and does not require builder reuse patterns. Each `bincode::serialize` call writes directly into a `Vec<u8>` with minimal overhead. No thread-local builder or reset logic is needed.

### 9.3 Socket Buffer Size

The default OS socket buffer size (64KB on most systems) is sufficient for terminal output. No custom tuning is needed unless profiling shows IPC as a bottleneck.

### 9.4 Message Ordering

All messages on a single socket connection are ordered (Unix domain sockets are stream-oriented). No sequence numbers are needed for ordering within a connection. However, if multiple agents produce interleaved output, each AgentStreamChunk carries a work_item_id so codiv can demultiplex.

## 10. Open Questions

1. **Message acknowledgment**: Should we add request-response IDs (correlation IDs) to all messages for reliability? Currently only ConfirmationRequest/Confirmation use a request_id. Adding correlation IDs to AgentRequest/AgentComplete would enable tracking in-flight requests and detecting lost responses.

2. **Separate streaming channel**: Should streaming output (AgentStreamChunk) use a separate socket or channel from control messages (Heartbeat, Shutdown, Error)? This would prevent a flood of output from blocking control messages, at the cost of managing two connections.

3. **Cross-version graceful handling**: How should codiv handle connecting to a daemon running a different (but not explicitly incompatible) protocol version? For example, protocol_version=2 client connecting to protocol_version=3 daemon — the daemon supports features the client does not know about. Current strategy is for the daemon to downgrade its message set, but this needs concrete specification.
