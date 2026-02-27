# Unified Session Timeline Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the single-prompt agent with a proper multi-turn conversation that includes shell command history as synthetic tool calls, using the aisdk native `Messages` API.

**Architecture:** The session timeline is a `Vec<SessionEvent>` in the `Agent` struct. Shell commands become synthetic tool call/result pairs; user AI queries become `Message::User`; AI responses become `Message::Assistant`. The client sends `CommandResult` IPC messages to the daemon in real-time. A reusable truncation utility keeps large outputs bounded.

**Tech Stack:** Rust, aisdk 0.5.2 (`Message::builder()`, `Messages`), tokio, bincode IPC

**Status:** All 7 tasks complete. Shell commands embedded as assistant/user message pairs (simpler than the originally planned `ToolCallInfo`/`ToolResultInfo` approach).

---

### Task 1: Output Truncation Utility

**Files:**
- Create: `crates/slate-common/src/truncate.rs`
- Modify: `crates/slate-common/src/lib.rs`

**Step 1: Write the failing test**

Add to `crates/slate-common/src/truncate.rs`:

```rust
/// Truncate long output keeping the first `head_lines` and last `tail_lines`.
/// If the output fits within head + tail lines, return it unchanged.
/// Otherwise, insert an omission marker in the middle.
pub fn truncate_output(output: &str, head_lines: usize, tail_lines: usize) -> String {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_output_unchanged() {
        let input = "line1\nline2\nline3";
        assert_eq!(truncate_output(input, 20, 20), input);
    }

    #[test]
    fn empty_output_unchanged() {
        assert_eq!(truncate_output("", 20, 20), "");
    }

    #[test]
    fn exact_boundary_unchanged() {
        // 40 lines = head(20) + tail(20), should not truncate
        let lines: Vec<String> = (1..=40).map(|i| format!("line {i}")).collect();
        let input = lines.join("\n");
        assert_eq!(truncate_output(&input, 20, 20), input);
    }

    #[test]
    fn truncates_long_output() {
        let lines: Vec<String> = (1..=100).map(|i| format!("line {i}")).collect();
        let input = lines.join("\n");
        let result = truncate_output(&input, 3, 3);
        assert!(result.contains("line 1"));
        assert!(result.contains("line 3"));
        assert!(result.contains("line 98"));
        assert!(result.contains("line 100"));
        assert!(result.contains("94 lines omitted"));
        assert!(!result.contains("line 50"));
    }

    #[test]
    fn single_line_head_tail() {
        let lines: Vec<String> = (1..=10).map(|i| format!("line {i}")).collect();
        let input = lines.join("\n");
        let result = truncate_output(&input, 1, 1);
        assert!(result.starts_with("line 1\n"));
        assert!(result.ends_with("line 10"));
        assert!(result.contains("8 lines omitted"));
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p slate-common truncate -- --nocapture`
Expected: FAIL with "not yet implemented"

**Step 3: Write minimal implementation**

Replace `todo!()` in `crates/slate-common/src/truncate.rs`:

```rust
pub fn truncate_output(output: &str, head_lines: usize, tail_lines: usize) -> String {
    if output.is_empty() {
        return String::new();
    }

    let lines: Vec<&str> = output.lines().collect();
    let total = lines.len();

    if total <= head_lines + tail_lines {
        return output.to_string();
    }

    let head: Vec<&str> = lines[..head_lines].to_vec();
    let tail: Vec<&str> = lines[total - tail_lines..].to_vec();
    let omitted = total - head_lines - tail_lines;

    format!(
        "{}\n... ({} lines omitted) ...\n{}",
        head.join("\n"),
        omitted,
        tail.join("\n")
    )
}
```

**Step 4: Export module**

In `crates/slate-common/src/lib.rs`, add:

```rust
pub mod truncate;
```

**Step 5: Run tests to verify they pass**

Run: `cargo test -p slate-common truncate -- --nocapture`
Expected: all 5 tests PASS

**Step 6: Build workspace**

Run: `cargo build --workspace`
Expected: no errors, no new warnings

**Step 7: Commit**

```bash
git add crates/slate-common/src/truncate.rs crates/slate-common/src/lib.rs
git commit -m "feat: add reusable output truncation utility in slate-common"
```

---

### Task 2: Add `ClientMessage::CommandResult` IPC variant

**Files:**
- Modify: `crates/slate-common/src/messages.rs`

**Step 1: Add the variant**

In `crates/slate-common/src/messages.rs`, add a new variant to the `ClientMessage` enum after `Confirmation`:

```rust
    CommandResult {
        command: String,
        output: String,
        exit_code: i32,
        cwd: String,
    },
```

**Step 2: Build workspace**

Run: `cargo build --workspace`
Expected: success (the daemon's `dispatch` match on `ClientMessage` will warn about non-exhaustive pattern — that's expected and fixed in Task 5)

Actually, since Rust requires exhaustive matching, this will cause a compile error in `daemon.rs`. Add a placeholder arm in `daemon.rs` `dispatch()` method (after `Confirmation` arm):

```rust
            ClientMessage::CommandResult { .. } => {
                // Task 5: will store in session timeline
            }
```

**Step 3: Build workspace again**

Run: `cargo build --workspace`
Expected: success, no errors

**Step 4: Run all tests**

Run: `cargo test --workspace`
Expected: all tests pass

**Step 5: Commit**

```bash
git add crates/slate-common/src/messages.rs crates/slated/src/daemon.rs
git commit -m "feat: add ClientMessage::CommandResult IPC variant"
```

---

### Task 3: Rewrite Agent with `SessionEvent` and aisdk `Messages` API

**Files:**
- Modify: `crates/slated/src/agent/agent.rs`
- Modify: `crates/slated/src/agent/config.rs`
- Modify: `crates/slated/Cargo.toml`

This is the largest task. We replace the concatenated prompt approach with proper aisdk multi-turn messages.

**Step 1: Update Cargo.toml**

Add `serde_json` dependency to `crates/slated/Cargo.toml` (needed for `ToolCallInfo.input` which is `serde_json::Value`):

```toml
serde_json = "1"
```

Note: `serde_json` is already listed in the Cargo.toml, so this step is a no-op. Verify it's there.

**Step 2: Rewrite `agent.rs`**

Replace the entire contents of `crates/slated/src/agent/agent.rs` with:

```rust
use crate::agent::config::{self, ModelAssignment};
use aisdk::core::messages::{Message, Messages};
use aisdk::core::tools::{ToolCallInfo, ToolDetails, ToolResultInfo};
use slate_common::truncate::truncate_output;
use slate_common::types::AgentRole;
use tokio::sync::mpsc;
use tracing::info;

/// Maximum number of events to keep in the session timeline.
const MAX_HISTORY_EVENTS: usize = 50;

/// Default head/tail lines for output truncation.
const TRUNCATE_HEAD: usize = 20;
const TRUNCATE_TAIL: usize = 20;

/// A single event in the unified session timeline.
pub enum SessionEvent {
    /// User ran a shell command in the terminal.
    ShellCommand {
        command: String,
        output: String,
        exit_code: i32,
        cwd: String,
    },
    /// User asked the AI a question (e.g., `? ...`).
    UserQuery(String),
    /// AI responded with text.
    AssistantResponse(String),
}

pub struct Agent {
    pub role: AgentRole,
    pub system_prompt: String,
    pub model_config: ModelAssignment,
    pub history: Vec<SessionEvent>,
}

impl Agent {
    pub fn new(role: AgentRole, model_config: ModelAssignment, system_prompt: String) -> Self {
        Self {
            role,
            system_prompt,
            model_config,
            history: Vec::new(),
        }
    }

    pub fn add_user_message(&mut self, content: &str) {
        self.history.push(SessionEvent::UserQuery(content.to_string()));
        self.enforce_limit();
    }

    pub fn add_assistant_message(&mut self, content: &str) {
        self.history.push(SessionEvent::AssistantResponse(content.to_string()));
        self.enforce_limit();
    }

    pub fn add_command_result(&mut self, command: &str, output: &str, exit_code: i32, cwd: &str) {
        let truncated = truncate_output(output, TRUNCATE_HEAD, TRUNCATE_TAIL);
        self.history.push(SessionEvent::ShellCommand {
            command: command.to_string(),
            output: truncated,
            exit_code,
            cwd: cwd.to_string(),
        });
        self.enforce_limit();
    }

    fn enforce_limit(&mut self) {
        if self.history.len() > MAX_HISTORY_EVENTS {
            let excess = self.history.len() - MAX_HISTORY_EVENTS;
            self.history.drain(..excess);
        }
    }

    /// Build aisdk Messages from the session timeline.
    ///
    /// ShellCommands become synthetic tool call + tool result pairs.
    /// UserQueries become User messages.
    /// AssistantResponses become Assistant messages.
    fn build_messages(&self) -> Messages {
        let mut builder = Message::builder()
            .system(&self.system_prompt);

        let mut tool_counter: usize = 0;

        for event in &self.history {
            match event {
                SessionEvent::ShellCommand { command, output, exit_code, cwd } => {
                    let tool_id = format!("term-{}", tool_counter);
                    tool_counter += 1;

                    // Synthetic assistant tool call
                    let mut call_info = ToolCallInfo::new("terminal");
                    call_info.id(tool_id.clone());
                    call_info.input(serde_json::json!({
                        "command": command,
                        "cwd": cwd,
                    }));

                    builder = builder.assistant(format!(
                        "Running command: {} (in {})", command, cwd
                    ));

                    // Tool result
                    let result_text = if *exit_code == 0 {
                        output.clone()
                    } else {
                        format!("{}\n[exit code: {}]", output, exit_code)
                    };
                    builder = builder.user(format!(
                        "[Terminal output for `{}`]:\n{}", command, result_text
                    ));
                }
                SessionEvent::UserQuery(text) => {
                    builder = builder.user(text.clone());
                }
                SessionEvent::AssistantResponse(text) => {
                    builder = builder.assistant(text.clone());
                }
            }
        }

        builder.build()
    }

    /// Run a streaming LLM call, forwarding chunks over IPC via `client_tx`.
    /// Returns the accumulated response text.
    pub async fn run_streaming(
        &self,
        request_id: &str,
        client_tx: &mpsc::Sender<Vec<u8>>,
    ) -> Result<String, String> {
        info!(
            "agent {:?} calling {}/{} ({} events in history)",
            self.role, self.model_config.provider, self.model_config.model, self.history.len()
        );

        let messages = self.build_messages();

        config::stream_from_config(
            &self.model_config,
            messages,
            request_id,
            client_tx,
        )
        .await
        .map_err(|e| e.to_string())
    }
}
```

**Step 3: Update `config.rs` to accept `Messages`**

In `crates/slated/src/agent/config.rs`, change `stream_from_config` and `run_stream` to accept `Messages` instead of separate `system` and `prompt` strings.

Replace the function signatures and implementations:

Change the `stream_from_config` signature from:
```rust
pub async fn stream_from_config(
    assignment: &ModelAssignment,
    system: &str,
    prompt: &str,
    request_id: &str,
    tx: &mpsc::Sender<Vec<u8>>,
) -> Result<String, DynError> {
```

To:
```rust
pub async fn stream_from_config(
    assignment: &ModelAssignment,
    messages: aisdk::core::messages::Messages,
    request_id: &str,
    tx: &mpsc::Sender<Vec<u8>>,
) -> Result<String, DynError> {
```

And update the three match arms to pass `messages.clone()` instead of `system, prompt`:
```rust
    match assignment.provider.as_str() {
        "anthropic" => {
            run_stream(Anthropic::model_name(&assignment.model), messages, request_id, tx, assignment).await
        }
        "openai" => {
            run_stream(OpenAI::model_name(&assignment.model), messages, request_id, tx, assignment).await
        }
        "google" => {
            run_stream(Google::model_name(&assignment.model), messages, request_id, tx, assignment).await
        }
        other => Err(format!("unsupported provider: {}", other).into()),
    }
```

Change the `run_stream` signature from:
```rust
async fn run_stream<M>(
    model: M,
    system: &str,
    prompt: &str,
    request_id: &str,
    tx: &mpsc::Sender<Vec<u8>>,
    config: &ModelAssignment,
) -> Result<String, DynError>
```

To:
```rust
async fn run_stream<M>(
    model: M,
    messages: aisdk::core::messages::Messages,
    request_id: &str,
    tx: &mpsc::Sender<Vec<u8>>,
    config: &ModelAssignment,
) -> Result<String, DynError>
```

And update the builder from:
```rust
    let mut builder = LanguageModelRequest::builder()
        .model(model)
        .system(system)
        .prompt(prompt);
```

To:
```rust
    let mut builder = LanguageModelRequest::builder()
        .model(model)
        .messages(messages);
```

**Step 4: Build workspace**

Run: `cargo build --workspace`
Expected: success, no errors, no new warnings

**Step 5: Run all tests**

Run: `cargo test --workspace`
Expected: all tests pass

**Step 6: Commit**

```bash
git add crates/slated/src/agent/agent.rs crates/slated/src/agent/config.rs crates/slated/Cargo.toml
git commit -m "feat: rewrite agent with SessionEvent timeline and aisdk Messages API"
```

---

### Task 4: Update System Prompt

**Files:**
- Modify: `crates/slated/src/daemon.rs`

**Step 1: Update the system prompt**

In `crates/slated/src/daemon.rs`, change the system prompt string in the `AgentRequest` handler from:

```rust
"You are a helpful coding assistant. Answer concisely."
```

To:

```rust
"You are a helpful coding assistant embedded in a terminal. \
You can see the user's recent terminal commands and their output in the conversation history. \
Use this context to give relevant, concise answers. \
When referencing files or directories, use paths relative to the user's current working directory when possible."
```

**Step 2: Build and test**

Run: `cargo build --workspace && cargo test --workspace`
Expected: pass

**Step 3: Commit**

```bash
git add crates/slated/src/daemon.rs
git commit -m "feat: update system prompt with terminal context awareness"
```

---

### Task 5: Handle `CommandResult` in Daemon

**Files:**
- Modify: `crates/slated/src/daemon.rs`

**Step 1: Update the `CommandResult` handler**

Replace the placeholder `CommandResult` arm in `dispatch()` with:

```rust
            ClientMessage::CommandResult {
                command,
                output,
                exit_code,
                cwd,
            } => {
                if let Some(session) = self.sessions.get_mut(&client_id) {
                    // If the agent exists, add the command to its timeline.
                    // If it's currently in use (taken for streaming), create one
                    // so the command isn't lost.
                    let agent = session.agent.get_or_insert_with(|| {
                        let catalog = crate::agent::config::ModelCatalog::load();
                        let assignment = catalog.assignment_for(&slate_common::types::AgentRole::Engineer);
                        crate::agent::agent::Agent::new(
                            slate_common::types::AgentRole::Engineer,
                            assignment,
                            "You are a helpful coding assistant embedded in a terminal. \
                            You can see the user's recent terminal commands and their output in the conversation history. \
                            Use this context to give relevant, concise answers. \
                            When referencing files or directories, use paths relative to the user's current working directory when possible.".to_string(),
                        )
                    });
                    agent.add_command_result(&command, &output, exit_code, &cwd);
                    info!("recorded command result from client {}: {}", client_id, command);
                }
            }
```

**Step 2: Build and test**

Run: `cargo build --workspace && cargo test --workspace`
Expected: pass

**Step 3: Commit**

```bash
git add crates/slated/src/daemon.rs
git commit -m "feat: handle CommandResult in daemon, store in session timeline"
```

---

### Task 6: Client Sends `CommandResult` After Shell Commands

**Files:**
- Modify: `crates/slate/src/ui/terminal.rs`
- Modify: `crates/slate/src/ipc/messages.rs`

**Step 1: Add `build_command_result` helper**

In `crates/slate/src/ipc/messages.rs`, add a new function:

```rust
/// Build a framed CommandResult message.
pub fn build_command_result(
    command: &str,
    output: &str,
    exit_code: i32,
    cwd: &str,
) -> Option<Vec<u8>> {
    let msg = ClientMessage::CommandResult {
        command: command.to_string(),
        output: output.to_string(),
        exit_code,
        cwd: cwd.to_string(),
    };
    frame_message(&msg).ok()
}
```

**Step 2: Send `CommandResult` when a command completes**

In `crates/slate/src/ui/terminal.rs`, there are two places where commands complete:

**Location A** — Normal command completion (around line 307-331):

After the line `*cwd = bash.capture_cwd();` and before `pending_command = None;` (around line 328-329), add:

```rust
                // Send command result to daemon for session timeline.
                if let Some(ref mut c) = client {
                    if let Some(frame) = ipc_messages::build_command_result(
                        &pending.command,
                        &result.output,
                        result.exit_code,
                        cwd,
                    ) {
                        c.send(&frame);
                    }
                }
```

**Location B** — Interactive/alternate screen command completion (around line 287-302):

After the `check_complete` call inside the alt-screen recovery block (after `*cwd = bash.capture_cwd();` around line 302), add the same send block. Note: for interactive commands the output may be empty since it went to the alt screen. Still send it — the command name and exit code are valuable context.

```rust
                    // Send command result to daemon for session timeline.
                    if let Some(ref mut c) = client {
                        let cmd_output = result.map(|r| (r.output.clone(), r.exit_code))
                            .unwrap_or_default();
                        if let Some(frame) = ipc_messages::build_command_result(
                            &pending.command,
                            &cmd_output.0,
                            cmd_output.1,
                            cwd,
                        ) {
                            c.send(&frame);
                        }
                    }
```

For the alt-screen path, `result` is an `Option<CommandResult>` from the `check_complete` call. Adjust accordingly — if `check_complete` returned `Some(result)`, use it; otherwise send with empty output and exit code 0.

**Step 3: Build and test**

Run: `cargo build --workspace && cargo test --workspace`
Expected: pass

**Step 4: Commit**

```bash
git add crates/slate/src/ui/terminal.rs crates/slate/src/ipc/messages.rs
git commit -m "feat: client sends CommandResult to daemon after shell commands"
```

---

### Task 7: Manual Integration Test

**Step 1: Build everything**

Run: `cargo build --workspace`
Expected: clean build, no warnings

**Step 2: Start daemon in foreground with debug logging**

```bash
RUST_LOG=slated=debug cargo run -p slated -- --foreground
```

**Step 3: In another terminal, start the client**

```bash
cargo run -p slate
```

**Step 4: Test sequence**

1. Run `ls` — should execute normally
2. Run `pwd` — should execute normally
3. Type `? what directory am I in and what files are here?` — the AI should know the answer from the terminal history
4. Type `? what was the exit code of the last command?` — the AI should know
5. Check daemon logs (`tail -f ~/.slate-agent/slated.log`) — should show "recorded command result" entries and "N events in history"

**Step 5: Verify**

- AI responses reference actual files/directories from the shell output
- No panics, no garbled output
- Streaming renders correctly (CRLF fix from earlier still works)

**Step 6: Final commit if any fixes needed**

```bash
git add -A
git commit -m "fix: integration test fixes for session timeline"
```

---

## Summary

| Task | Description | Files |
|------|-------------|-------|
| 1 | Output truncation utility | `slate-common/src/truncate.rs`, `slate-common/src/lib.rs` |
| 2 | `ClientMessage::CommandResult` IPC | `slate-common/src/messages.rs`, `slated/src/daemon.rs` |
| 3 | Agent rewrite with `SessionEvent` + aisdk `Messages` | `slated/src/agent/agent.rs`, `slated/src/agent/config.rs` |
| 4 | System prompt update | `slated/src/daemon.rs` |
| 5 | Daemon `CommandResult` handler | `slated/src/daemon.rs` |
| 6 | Client sends `CommandResult` | `slate/src/ui/terminal.rs`, `slate/src/ipc/messages.rs` |
| 7 | Manual integration test | — |
