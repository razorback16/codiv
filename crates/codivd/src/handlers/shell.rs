use crate::daemon::Daemon;
use crate::ipc::server::ClientId;
use codiv_common::messages::ClientMessage;

/// Handle all shell lease protocol messages by routing to the session's
/// RelayManager.
pub(crate) async fn handle_shell_lease_message(
    daemon: &mut Daemon,
    client_id: ClientId,
    msg: ClientMessage,
) {
    if let Some(session) = daemon.sessions.get_mut(&client_id) {
        // Update session cwd from completion messages
        match &msg {
            ClientMessage::CommandCompleted { cwd, .. }
            | ClientMessage::CommandFailed { cwd, .. }
            | ClientMessage::CommandCancelled { cwd, .. } => {
                session.ipc.cwd = cwd.clone();
            }
            _ => {}
        }
        if let Some(ref relay) = session.relay.relay_manager {
            relay.handle_client_message(&msg).await;
        }
    }
}
