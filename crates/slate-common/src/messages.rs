use crate::permissions::PermissionMode;
use serde::{Deserialize, Serialize};

/// A record of a recently executed command, provided by the client as context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandRecord {
    pub command: String,
    pub output: String,
    pub exit_code: i32,
    pub timestamp: u64,
}

/// Session context sent by the client with each AgentRequest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionContext {
    pub cwd: String,
    pub recent_commands: Vec<CommandRecord>,
    pub env_vars: Vec<(String, String)>,
}

/// Messages sent from the slate client to the slated daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    AgentRequest {
        prompt: String,
        request_id: String,
        context: SessionContext,
        #[serde(default)]
        thinking: bool,
    },
    EnvSnapshot {
        env_vars: Vec<(String, String)>,
        path: String,
        cwd: String,
    },
    Confirmation {
        request_id: String,
        approved: bool,
        #[serde(default)]
        add_to_allowlist: bool,
        #[serde(default)]
        add_to_denylist: bool,
        #[serde(default)]
        comment: Option<String>,
    },
    SetPermissionMode {
        mode: PermissionMode,
    },
    Heartbeat {
        timestamp: u64,
    },
    Shutdown {
        reason: String,
    },
    CommandResult {
        command: String,
        output: String,
        exit_code: i32,
        cwd: String,
    },
}

/// Messages sent from the slated daemon to the slate client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonMessage {
    AgentStreamChunk {
        request_id: String,
        chunk: StreamChunk,
    },
    AgentComplete {
        request_id: String,
        summary: String,
    },
    ConfirmationRequest {
        request_id: String,
        description: String,
        risk: RiskLevel,
        tool_name: String,
        tool_args: String,
    },
    PermissionOutcome {
        tool_name: String,
        granted: bool,
        reason: String,
    },
    PermissionModeChanged {
        mode: PermissionMode,
    },
    Heartbeat {
        timestamp: u64,
    },
    Error {
        request_id: String,
        message: String,
    },
    AgentMeta {
        model_alias: String,
        total_tokens: usize,
        context_window: usize,
    },
    /// Sent when a new SQLite session is created for this client.
    SessionCreated {
        session_id: String,
        name: Option<String>,
    },
    /// Sent when the background LLM name generation completes.
    SessionNameUpdated {
        session_id: String,
        name: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamChunk {
    Text(String),
    Reasoning(String),
    ToolCallDelta { tool_call_id: String, tool_name: String, delta: String },
    ToolCall { name: String, arguments: String },
    ToolResult { name: String, result: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

/// Frame a message for IPC transport: 4-byte BE length prefix + bincode payload.
pub fn frame_message<T: Serialize>(msg: &T) -> Result<Vec<u8>, bincode::Error> {
    let payload = bincode::serialize(msg)?;
    let len = payload.len() as u32;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Parse the 4-byte big-endian frame header into a payload length.
pub fn parse_frame_header(header: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*header)
}
