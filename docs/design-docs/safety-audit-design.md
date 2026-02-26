# Safety & Audit System — Design Document

**Date**: 2026-02-26
**Author**: Subhagato
**Status**: Draft
**Refines**: FR-009 (Safety & Guardrails), Phase 2 (Basic Safety), Phase 7 (Advanced Safety & Compliance)

---

## 1. Overview

This document specifies the safety and audit architecture for Slate Agent. It covers risk classification, user confirmation flows, allowlist/denylist configuration, audit log schema, and sandboxing strategy.

Phase 2 delivers the basic safety layer: pattern-based risk classification, confirmation prompts for dangerous operations, configurable allow/deny lists, and tool-call audit logging. Phase 7 extends this with OS-level and container-based sandboxing, full audit trails, privacy controls, and compliance alignment.

## 2. Problem

The PRD defines safety requirements (FR-009) but leaves several questions unanswered:

- What is the exact risk classification schema? Which operations map to which risk levels?
- How do confirmation flows render in the TUI and communicate with the daemon over IPC?
- What is the file format and resolution order for allowlists and denylists?
- What schema do audit log entries follow? What action types are recorded?
- How does OS-level sandboxing integrate with the daemon's worker processes?

This design provides concrete answers to each.

## 3. Risk Classification Schema

### 3.1 Risk Levels

Four levels, from least to most restrictive:

| Level      | Description                                           | Requires Confirmation |
|------------|-------------------------------------------------------|-----------------------|
| `low`      | Read-only operations with no side effects              | No                    |
| `medium`   | Local writes that are reversible                       | No                    |
| `high`     | Destructive or externally-visible operations           | Yes                   |
| `critical` | Irreversible, high-blast-radius operations             | Yes (no "always" option) |

### 3.2 Built-in Tool Classification

Tools have a base risk level independent of their arguments:

- **Low**: Read, Glob, Grep — pure reads, no side effects
- **Medium**: Write, Edit — local file modifications, reversible via git
- **Pattern-matched**: Bash — classified by matching the command string against the pattern table below

### 3.3 Pattern-Based Classification

Bash commands are classified by matching against regex patterns in descending severity order. The first match wins.

```toml
[safety.risk_classification]
# Pattern-based classification (regex on command string)
critical = [
  "rm\\s+-rf",
  "git\\s+push\\s+--force",
  "git\\s+reset\\s+--hard",
  "DROP\\s+(TABLE|DATABASE)",
  "TRUNCATE",
  "sudo\\s+",
  "su\\s+-",
]
high = [
  "rm\\s+",
  "git\\s+push",
  "git\\s+reset",
  "git\\s+clean",
  "chmod\\s+777",
  "curl\\s+.*-X\\s+(POST|PUT|DELETE)",
]
# Commands not matching any pattern default to:
#   medium — if the tool is a write tool (Write, Edit)
#   low    — if the tool is a read tool or unclassified Bash command
```

Classification examples:

| Command                          | Matched Pattern       | Risk Level |
|----------------------------------|-----------------------|------------|
| `cat README.md`                  | (no match)            | low        |
| `git status`                     | (no match)            | low        |
| `npm install`                    | (no match)            | low        |
| `rm old-file.txt`               | `rm\s+`               | high       |
| `git push origin main`          | `git\s+push`          | high       |
| `rm -rf /`                      | `rm\s+-rf`            | critical   |
| `git push --force origin main`  | `git\s+push\s+--force`| critical   |
| `sudo apt install`              | `sudo\s+`             | critical   |

## 4. Confirmation Flow UX

### 4.1 High-Risk Confirmation

When a high-risk action is detected:

1. The agent output block displays the proposed action with a risk level badge.
2. A confirmation prompt appears in slate's input area:
   ```
   ⚠ HIGH RISK: rm -rf ./build/ — Allow? [y/N/always/never]
   ```
3. User responses:
   - **`y`** (yes): execute this one time
   - **`n`** or **Enter** (no): reject; the agent is informed and must find an alternative
   - **`always`**: add to session allowlist (current session only)
   - **`never`**: add to session denylist (current session only)
4. Timeout: 60 seconds with no input defaults to reject.

### 4.2 Critical-Risk Confirmation

Critical-risk actions follow the same flow with stronger visual treatment and no "always" option:

```
🔴 CRITICAL: git push --force origin main — This cannot be undone. Allow? [y/N]
```

- RED coloring in the TUI
- Only `y` or `n`/Enter — no `always` option (every critical action requires explicit approval)
- Same 60-second timeout defaulting to reject

### 4.3 IPC Flow

The confirmation flow spans the IPC boundary between slate and slated:

```
slate                          slated
  |                              |
  |<-- AgentStreamChunk ---------|  (tool_start: shows proposed command)
  |<-- ConfirmationRequest ------|  (risk_level, command, description)
  |                              |
  |  [user sees prompt, decides] |
  |                              |
  |--- ConfirmationResponse ---->|  (approved, add_to_allowlist, add_to_denylist)
  |                              |
  |  (if approved)               |
  |<-- AgentStreamChunk ---------|  (tool_end: execution result)
  |                              |
  |  (if rejected)               |
  |<-- AgentStreamChunk ---------|  (text: "Action rejected, finding alternative...")
```

## 5. Allowlist / Denylist Format

### 5.1 Persistent Configuration

User-level configuration in `~/.slate-agent/config.toml`:

```toml
[safety]
allowlist = [
  "git push origin main",
  "rm -rf ./build/",
  "npm publish",
]
denylist = [
  "rm -rf /",
  "git push --force origin main",
  "sudo rm",
]
```

### 5.2 Project-Level Overrides

Project-level configuration in `.slate-agent/config.toml` at the project root. Same format as user-level. Project entries extend (not replace) user-level entries.

### 5.3 Session-Level Lists

Built dynamically from `always` and `never` responses during a session. Cleared when the session ends. Not persisted to disk.

### 5.4 Resolution Order

When evaluating a command:

1. Check **denylist** (project-level, then user-level, then session-level). If any match: **deny**.
2. Check **allowlist** (project-level, then user-level, then session-level). If any match: **allow without confirmation**.
3. If no list matches: apply risk classification and prompt if high/critical.

Denylist always wins over allowlist. Project-level overrides user-level for the same entry.

## 6. Audit Log Schema

### 6.1 Storage

Audit logs are stored per session as JSON Lines files:

```
~/.slate-agent/audit/<session_id>.jsonl
```

Each line is a self-contained JSON object. JSON Lines format enables append-only writes and streaming reads.

### 6.2 Entry Schema

```json
{
  "timestamp": "2026-02-26T14:30:00.123Z",
  "session_id": "abc-123",
  "work_item_id": "wi-456",
  "agent_role": "Engineer",
  "model_id": "claude-sonnet-4-5",
  "action_type": "tool_call",
  "tool_name": "Bash",
  "tool_args": {"command": "npm test", "timeout": 120000},
  "risk_level": "low",
  "confirmation_required": false,
  "confirmation_response": null,
  "result_summary": "exit code 0, 15 tests passed",
  "tokens_used": 150,
  "duration_ms": 3200
}
```

### 6.3 Action Types

| Action Type              | Description                                              | Phase |
|--------------------------|----------------------------------------------------------|-------|
| `tool_call`              | Agent invoked a tool (Bash, Write, Read, etc.)           | 2     |
| `llm_request`            | LLM API call (prompt tokens, completion tokens, model)   | 7     |
| `confirmation_prompt`    | Safety confirmation was shown to user                    | 2     |
| `work_item_state_change` | Work Item transitioned state (pending → running, etc.)   | 7     |
| `memory_write`           | Agent wrote to memory system                             | 7     |
| `file_modified`          | File was created, modified, or deleted                   | 7     |

### 6.4 Retention

- Default retention: 30 days
- Configurable:
  ```toml
  [safety.audit]
  retention_days = 30
  ```
- A cleanup routine runs on daemon startup, removing `.jsonl` files older than the retention period.

### 6.5 Privacy Boundary

When `[privacy] send_command_output_to_llm = false` is set, command output is excluded from the LLM context window. However, the audit log still records the output locally. The audit system operates independently of LLM context filtering.

## 7. Phase 2 Safety (Basic)

Phase 2 delivers the minimum viable safety layer:

- **Risk classification**: pattern matching on Bash commands, built-in levels for other tools
- **Confirmation prompts**: high and critical risk actions require user approval via TUI
- **Allowlist/denylist**: user-level and project-level persistent lists in TOML config
- **Basic audit logging**: `tool_call` and `confirmation_prompt` action types only
- **Hard rule**: critical-risk commands never auto-execute, regardless of allowlist (allowlist skips the prompt but critical still requires explicit `y`)

Implementation scope:
- Risk classifier module in `slated` that evaluates tool calls before execution
- Confirmation IPC messages (ConfirmationRequest / ConfirmationResponse)
- Confirmation prompt widget in `slate` TUI
- JSONL audit writer in `slated`
- Config parser for `[safety]` section

## 8. Phase 7 Safety (Advanced)

Phase 7 extends the safety system with isolation, full auditing, and compliance alignment.

### 8.1 OS-Level Sandboxing

Worker bash sessions spawned by `slated` run inside OS-level sandboxes:

**Linux — bubblewrap (bwrap)**:
- Mount namespace isolation: worker sees only the project directory and essential system paths
- Network restriction: no network access by default (configurable)
- Filesystem: read-only except the working directory
- Applied per-worker, not to the daemon itself

**macOS — seatbelt profiles (sandbox-exec)**:
- Equivalent restrictions via seatbelt profile files
- Read-only filesystem except the working directory
- Network access restricted by default

**Configuration**:
```toml
[safety.sandbox]
enabled = true
policy = "strict"  # "strict" = read-only except cwd, "permissive" = write to home dir
```

**Graceful degradation**: if sandbox setup fails (e.g., bwrap not installed), fall back to confirmation-based safety and warn the user that sandboxing is unavailable.

### 8.2 Docker-Based Sandboxing

An advanced option for stronger isolation:

- Dedicated container per Work Item
- Project directory mounted as a volume
- Agent runs with full permissions inside the container (safe because isolated)
- Network access configurable per Work Item
- Container image configurable:
  ```toml
  [safety.sandbox]
  docker_image = "slate-agent-sandbox:latest"
  docker_network = "none"  # or "bridge" for network access
  ```

### 8.3 Full Audit Trail

All action types are recorded (not just `tool_call`):
- `llm_request` entries track token usage and model selection
- `work_item_state_change` entries track the DAG execution lifecycle
- `memory_write` entries track what the agent persists
- `file_modified` entries track every file-system change with before/after hashes

### 8.4 Privacy Controls

Granular privacy settings:

```toml
[privacy]
send_command_output_to_llm = true
send_file_contents_to_llm = true
audit_record_output = true       # record command output in audit log
audit_record_file_diffs = false  # record file diffs in audit log
```

### 8.5 Compliance Alignment

- OWASP LLM Top 10 alignment checklist (documented in a separate appendix)
- NIST AI RMF alignment notes covering governance, risk mapping, and measurement
- These are reference documents, not runtime features

## 9. IPC Extensions for Safety

Two new IPC message types support the confirmation flow:

### 9.1 ConfirmationRequest (slated → slate)

| Field          | Type   | Description                                    |
|----------------|--------|------------------------------------------------|
| session_id     | string | Active session identifier                      |
| work_item_id   | string | Work Item that triggered the action            |
| request_id     | string | Unique ID for correlating request and response |
| command        | string | The command or action to be confirmed          |
| risk_level     | enum   | `high` or `critical`                           |
| description    | string | Human-readable explanation of the risk         |

### 9.2 ConfirmationResponse (slate → slated)

| Field            | Type   | Description                                    |
|------------------|--------|------------------------------------------------|
| session_id       | string | Active session identifier                      |
| request_id       | string | Matches the ConfirmationRequest                |
| approved         | bool   | Whether the user approved the action           |
| add_to_allowlist | bool   | User selected "always" (session-level)         |
| add_to_denylist  | bool   | User selected "never" (session-level)          |

## 10. Open Questions

1. **Dry-run mode**: Should there be a mode where the agent shows all planned actions but executes none? This would be useful for reviewing agent behavior before granting trust.

2. **Piped commands**: How should risk classification handle piped commands like `find . | xargs rm`? The current regex approach classifies based on the full command string, so `rm` in `xargs rm` would match. But `$(command substitution)` and backtick execution are harder to catch statically.

3. **Tamper-evident audit logs**: Should audit logs be cryptographically signed or use hash chains to detect tampering? This adds complexity but could be valuable for compliance-sensitive environments.

4. **Custom risk patterns**: Should users be able to add their own risk classification patterns in config, or only modify the allowlist/denylist? Custom patterns would allow domain-specific risk rules (e.g., classifying `kubectl delete` as critical).
