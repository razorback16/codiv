use crate::conversation::{ConversationEvent, SessionInfo};
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

/// Messages sent from the codiv client to the codivd daemon.
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
    ListSessions,
    LoadSession {
        session_id: String,
    },
    /// Start a fresh session: reset agent state, create a new SQLite session.
    NewSession,
    CancelRequest {
        request_id: String,
    },
    /// Request manual compaction of the current conversation.
    CompactRequest,

    // --- Shell lease protocol (client → daemon) ---

    /// Client acquired the shell lease (coprocess is free).
    ShellLeaseAcquired {
        lease_id: String,
    },
    /// Client queued the lease request (coprocess is busy).
    ShellLeaseQueued {
        lease_id: String,
        queue_len: usize,
    },
    /// Client started executing a leased command in the coprocess.
    CommandStarted {
        lease_id: String,
        execution_id: String,
    },
    /// Leased command completed successfully.
    CommandCompleted {
        lease_id: String,
        execution_id: String,
        output: String,
        exit_code: i32,
        cwd: String,
    },
    /// Leased command failed to start (e.g. coprocess error).
    CommandFailed {
        lease_id: String,
        execution_id: String,
        error: String,
        cwd: String,
    },
    /// Leased command was cancelled (e.g. SIGINT sent to PTY).
    CommandCancelled {
        lease_id: String,
        execution_id: String,
        cwd: String,
    },
    /// Client released the shell lease.
    ShellLeaseReleased {
        lease_id: String,
    },
}

/// Messages sent from the codivd daemon to the codiv client.
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
        input_tokens: usize,
        output_tokens: usize,
        cache_read_tokens: usize,
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
    /// Response to ListSessions with up to 20 recent sessions.
    SessionList {
        sessions: Vec<SessionInfo>,
    },
    /// Full event batch for visual replay of a loaded session.
    SessionReplay {
        events: Vec<ConversationEvent>,
    },
    /// Generic notice pushed to the client (e.g. config reloaded).
    Notice {
        message: String,
    },
    /// Compaction is starting — client should clear blocks and show tool header.
    CompactionStarted {
        request_id: String,
        compacted_event_count: usize,
    },
    /// Compaction completed — client should finalize.
    CompactionComplete {
        summary: String,
        compacted_event_count: usize,
    },
    // --- Shell lease protocol (daemon → client) ---

    /// Request exclusive access to the client's shell coprocess.
    AcquireShellLease {
        lease_id: String,
        request_id: String,
    },
    /// Execute a command within an acquired lease.
    ExecuteLeasedCommand {
        lease_id: String,
        execution_id: String,
        command: String,
        execution_timeout_ms: u64,
    },
    /// Release the shell lease (daemon is done with the shell).
    ReleaseShellLease {
        lease_id: String,
        reason: ReleaseReason,
    },
    /// Cancel an in-flight leased command.
    CancelLeasedCommand {
        lease_id: String,
        execution_id: String,
        reason: CancelReason,
    },
}

/// Why the daemon is releasing a shell lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReleaseReason {
    Completed,
    Cancelled,
    ClientDisconnect,
}

/// Why a leased command is being cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CancelReason {
    UserAbort,
    ExecutionTimeout,
    SessionStale,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamChunk {
    Text(String),
    Reasoning(String),
    ToolCallDelta { tool_call_id: String, tool_name: String, delta: String },
    ToolCall {
        name: String,
        arguments: String,
        /// For edit tools: file content captured before the tool executes,
        /// so the client can compute an accurate diff regardless of timing.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pre_edit_content: Option<String>,
    },
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
