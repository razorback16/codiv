pub use codiv_common::config::{FRAME_HEADER_SIZE, MAX_MESSAGE_SIZE};
pub use codiv_common::messages::{
    frame_message, parse_frame_header, ClientMessage, DaemonMessage,
    SessionContext, StreamChunk,
};

use std::time::{SystemTime, UNIX_EPOCH};

/// Build a framed EnvSnapshot message from session data.
pub fn build_env_snapshot(
    env_vars: &[(String, String)],
    path: &str,
    cwd: &str,
) -> Option<Vec<u8>> {
    let msg = ClientMessage::EnvSnapshot {
        env_vars: env_vars.to_vec(),
        path: path.to_string(),
        cwd: cwd.to_string(),
    };
    frame_message(&msg).ok()
}

/// Build a framed Heartbeat message.
pub fn build_heartbeat() -> Option<Vec<u8>> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let msg = ClientMessage::Heartbeat { timestamp: ts };
    frame_message(&msg).ok()
}

/// Build a framed AgentRequest message with session context.
pub fn build_agent_request(
    prompt: &str,
    request_id: &str,
    context: SessionContext,
    thinking: bool,
) -> Option<Vec<u8>> {
    let msg = ClientMessage::AgentRequest {
        prompt: prompt.to_string(),
        request_id: request_id.to_string(),
        context,
        thinking,
    };
    frame_message(&msg).ok()
}

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

/// Build a framed Confirmation response message.
pub fn build_confirmation(
    request_id: &str,
    approved: bool,
    add_to_allowlist: bool,
    add_to_denylist: bool,
    comment: Option<String>,
) -> Option<Vec<u8>> {
    let msg = ClientMessage::Confirmation {
        request_id: request_id.to_string(),
        approved,
        add_to_allowlist,
        add_to_denylist,
        comment,
    };
    frame_message(&msg).ok()
}

/// Build a framed SetPermissionMode message.
pub fn build_set_permission_mode(mode: codiv_common::permissions::PermissionMode) -> Option<Vec<u8>> {
    let msg = ClientMessage::SetPermissionMode { mode };
    frame_message(&msg).ok()
}

/// Build a framed ListSessions message.
pub fn build_list_sessions() -> Option<Vec<u8>> {
    let msg = ClientMessage::ListSessions;
    frame_message(&msg).ok()
}

/// Build a framed NewSession message.
pub fn build_new_session() -> Option<Vec<u8>> {
    let msg = ClientMessage::NewSession;
    frame_message(&msg).ok()
}

/// Build a framed LoadSession message.
pub fn build_load_session(session_id: &str) -> Option<Vec<u8>> {
    let msg = ClientMessage::LoadSession {
        session_id: session_id.to_string(),
    };
    frame_message(&msg).ok()
}

// --- Shell lease protocol builders ---

/// Build a framed ShellLeaseAcquired message.
pub fn build_shell_lease_acquired(lease_id: &str) -> Option<Vec<u8>> {
    let msg = ClientMessage::ShellLeaseAcquired {
        lease_id: lease_id.to_string(),
    };
    frame_message(&msg).ok()
}

/// Build a framed ShellLeaseQueued message.
pub fn build_shell_lease_queued(lease_id: &str, queue_len: usize) -> Option<Vec<u8>> {
    let msg = ClientMessage::ShellLeaseQueued {
        lease_id: lease_id.to_string(),
        queue_len,
    };
    frame_message(&msg).ok()
}

/// Build a framed CommandStarted message.
pub fn build_command_started(lease_id: &str, execution_id: &str) -> Option<Vec<u8>> {
    let msg = ClientMessage::CommandStarted {
        lease_id: lease_id.to_string(),
        execution_id: execution_id.to_string(),
    };
    frame_message(&msg).ok()
}

/// Build a framed CommandCompleted message.
pub fn build_command_completed(
    lease_id: &str,
    execution_id: &str,
    output: &str,
    exit_code: i32,
    cwd: &str,
) -> Option<Vec<u8>> {
    let msg = ClientMessage::CommandCompleted {
        lease_id: lease_id.to_string(),
        execution_id: execution_id.to_string(),
        output: output.to_string(),
        exit_code,
        cwd: cwd.to_string(),
    };
    frame_message(&msg).ok()
}

/// Build a framed CommandFailed message.
pub fn build_command_failed(
    lease_id: &str,
    execution_id: &str,
    error: &str,
    cwd: &str,
) -> Option<Vec<u8>> {
    let msg = ClientMessage::CommandFailed {
        lease_id: lease_id.to_string(),
        execution_id: execution_id.to_string(),
        error: error.to_string(),
        cwd: cwd.to_string(),
    };
    frame_message(&msg).ok()
}

/// Build a framed CommandCancelled message.
pub fn build_command_cancelled(
    lease_id: &str,
    execution_id: &str,
    cwd: &str,
) -> Option<Vec<u8>> {
    let msg = ClientMessage::CommandCancelled {
        lease_id: lease_id.to_string(),
        execution_id: execution_id.to_string(),
        cwd: cwd.to_string(),
    };
    frame_message(&msg).ok()
}

/// Build a framed ShellLeaseReleased message.
pub fn build_shell_lease_released(lease_id: &str) -> Option<Vec<u8>> {
    let msg = ClientMessage::ShellLeaseReleased {
        lease_id: lease_id.to_string(),
    };
    frame_message(&msg).ok()
}

/// Build a framed CompactRequest message.
pub fn build_compact_request() -> Option<Vec<u8>> {
    let msg = ClientMessage::CompactRequest;
    frame_message(&msg).ok()
}

/// Build a framed CancelRequest message.
pub fn build_cancel_request(request_id: &str) -> Option<Vec<u8>> {
    let msg = ClientMessage::CancelRequest {
        request_id: request_id.to_string(),
    };
    frame_message(&msg).ok()
}
