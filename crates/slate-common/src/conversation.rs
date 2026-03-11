use serde::{Deserialize, Serialize};

/// A single event in a persisted conversation timeline.
///
/// This replaces the old `SessionEvent` enum and adds tool-call tracking
/// that was previously missing entirely.  Each variant mirrors what the
/// daemon already streams via `DaemonMessage`/`StreamChunk`, but stores
/// aggregated (non-streaming) content suitable for persistence and replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConversationEvent {
    /// User submitted a prompt (AI mode `? …`).
    UserPrompt {
        text: String,
        request_id: String,
    },
    /// User ran a shell command in the terminal.
    ShellCommand {
        command: String,
        output: String,
        exit_code: i32,
        cwd: String,
    },
    /// Final aggregated assistant text for one request.
    AssistantText {
        request_id: String,
        text: String,
    },
    /// Aggregated reasoning/thinking content for one request.
    AssistantReasoning {
        request_id: String,
        text: String,
    },
    /// A tool invocation the model made during a request.
    ToolCall {
        request_id: String,
        tool_name: String,
        arguments: String,
    },
    /// The result returned by a tool invocation.
    ToolResult {
        request_id: String,
        tool_name: String,
        result: String,
    },
    /// An error that occurred during a request.
    Error {
        request_id: String,
        message: String,
    },
}

/// Metadata for a stored session (one row in the `sessions` table).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub name: Option<String>,
    pub parent_id: Option<String>,
    pub cwd: String,
    pub created_at: String,
    pub updated_at: String,
}
