pub use slate_common::config::{FRAME_HEADER_SIZE, MAX_MESSAGE_SIZE};
pub use slate_common::messages::{
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
pub fn build_set_permission_mode(mode: slate_common::permissions::PermissionMode) -> Option<Vec<u8>> {
    let msg = ClientMessage::SetPermissionMode { mode };
    frame_message(&msg).ok()
}
