use crate::ipc::messages as ipc_messages;

use crate::ui::terminal::state::TerminalState;

/// Handle a lease protocol message. Returns frames to send to the daemon.
/// Separated from handle_daemon_message to avoid borrow conflicts with `client`.
pub(crate) fn handle_lease_message(
    msg: &ipc_messages::DaemonMessage,
    state: &mut TerminalState,
) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    match msg {
        ipc_messages::DaemonMessage::AcquireShellLease {
            lease_id,
            request_id,
        } => {
            if state.cmd.pending_command.is_none() && state.shell.shell_relay.active_lease.is_none() {
                state.shell.shell_relay.active_lease = Some(super::super::state::ActiveLease {
                    lease_id: lease_id.clone(),
                    request_id: request_id.clone(),
                    current_command: None,
                });
                if let Some(frame) = ipc_messages::build_shell_lease_acquired(lease_id) {
                    frames.push(frame);
                }
            } else {
                let queue_len = state.shell.shell_relay.pending_leases.len() + 1;
                state.shell.shell_relay.pending_leases.push_back(
                    super::super::state::PendingLease { lease_id: lease_id.clone(), request_id: request_id.clone() },
                );
                if let Some(frame) = ipc_messages::build_shell_lease_queued(lease_id, queue_len) {
                    frames.push(frame);
                }
            }
        }
        ipc_messages::DaemonMessage::ExecuteLeasedCommand {
            lease_id,
            execution_id,
            command,
            execution_timeout_ms,
        } => {
            if let Some(ref mut lease) = state.shell.shell_relay.active_lease {
                if lease.lease_id == *lease_id {
                    lease.current_command = Some(super::super::state::ActiveLeasedCommand {
                        execution_id: execution_id.clone(),
                        command: command.clone(),
                        _timeout_ms: *execution_timeout_ms,
                    });
                }
            }
        }
        ipc_messages::DaemonMessage::ReleaseShellLease { lease_id, .. } => {
            if let Some(ref lease) = state.shell.shell_relay.active_lease {
                if lease.lease_id == *lease_id {
                    state.shell.shell_relay.active_lease = None;
                    if let Some(frame) = ipc_messages::build_shell_lease_released(lease_id) {
                        frames.push(frame);
                    }
                    // Promote next pending lease if coprocess is free.
                    frames.extend(promote_pending_lease_frames(state));
                }
            }
        }
        ipc_messages::DaemonMessage::CancelLeasedCommand {
            lease_id,
            execution_id,
            ..
        } => {
            if let Some(frame) = ipc_messages::build_command_cancelled(
                lease_id,
                execution_id,
                &state.shell.cwd,
            ) {
                frames.push(frame);
            }
        }
        _ => {}
    }
    frames
}

/// Try to promote the next pending lease and return frames to send.
fn promote_pending_lease_frames(state: &mut TerminalState) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    if state.shell.shell_relay.active_lease.is_some() || state.cmd.pending_command.is_some() {
        return frames;
    }
    if let Some(pending) = state.shell.shell_relay.pending_leases.pop_front() {
        let lease_id = pending.lease_id.clone();
        state.shell.shell_relay.active_lease = Some(super::super::state::ActiveLease {
            lease_id: lease_id.clone(),
            request_id: pending.request_id,
            current_command: None,
        });
        if let Some(frame) = ipc_messages::build_shell_lease_acquired(&lease_id) {
            frames.push(frame);
        }
    }
    frames
}

/// Try to promote the next pending lease to active when the coprocess is free.
/// Sends frames directly to the client.
pub(crate) fn promote_pending_lease(
    state: &mut TerminalState,
    client: &mut Option<crate::ipc::client::CodivdClient>,
) {
    let frames = promote_pending_lease_frames(state);
    if let Some(ref mut c) = client {
        for frame in frames {
            c.send(&frame);
        }
    }
}
