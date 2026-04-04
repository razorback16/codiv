use crate::agent::permissions::PermissionContext;
use crate::daemon::Daemon;
use crate::ipc::server::ClientId;
use codiv_common::messages::DaemonMessage;
use std::sync::Arc;
use tracing::info;

pub(crate) async fn handle_confirmation(
    daemon: &mut Daemon,
    client_id: ClientId,
    request_id: String,
    approved: bool,
    add_to_allowlist: bool,
    add_to_denylist: bool,
    comment: Option<String>,
) {
    if let Some(session) = daemon.sessions.get(&client_id) {
        if let Some(ref pctx) = session.permissions.permission_ctx {
            // Extract metadata for this request
            let meta = {
                let mut meta_guard = pctx.pending_meta.lock().unwrap();
                meta_guard.remove(&request_id)
            };

            let mut pending = pctx.pending.lock().unwrap();
            if let Some(sender) = pending.remove(&request_id) {
                // Persist permission decision if requested
                if let Some((ref tool_name, ref args)) = meta {
                    let command_prefix =
                        crate::agent::permission_evaluator::extract_args_pattern(tool_name, args);
                    let entry = match command_prefix {
                        Some(ref prefix) => format!("{}:{}", tool_name, prefix),
                        None => tool_name.to_string(),
                    };

                    if add_to_allowlist && approved {
                        crate::agent::config::add_permission_to_config(&entry, "allow");
                    } else if add_to_denylist && !approved {
                        crate::agent::config::add_permission_to_config(&entry, "deny");
                    }
                    // One-time decisions are not cached — they apply only to this call.
                }

                let result = crate::agent::permissions::ConfirmationResult {
                    approved,
                    add_to_allowlist,
                    add_to_denylist,
                    comment,
                };
                let _ = sender.send(result);
            }
        }
    }
}

pub(crate) async fn handle_set_permission_mode(
    daemon: &mut Daemon,
    client_id: ClientId,
    mode: codiv_common::permissions::PermissionMode,
) {
    info!("permission mode change requested: {}", mode);
    if let Some(session) = daemon.sessions.get_mut(&client_id) {
        match session.permissions.permission_ctx {
            Some(ref pctx) => {
                pctx.set_mode(mode);
            }
            None => {
                // Create PermissionContext eagerly so the mode is
                // applied even before the first AgentRequest.
                if let Some(client_tx) = daemon.ipc.client_sender(client_id) {
                    let cfg = daemon.config.read().expect("daemon config RwLock poisoned");
                    let ctx = Arc::new(PermissionContext::new(
                        mode,
                        client_tx,
                        cfg.models.clone(),
                        Arc::clone(&daemon.config),
                    ));
                    drop(cfg);
                    session.permissions.permission_ctx = Some(ctx);
                }
            }
        }
    }
    // Acknowledge the mode change back to the client.
    if let Some(client_tx) = daemon.ipc.client_sender(client_id) {
        let msg = DaemonMessage::PermissionModeChanged { mode };
        if let Ok(frame) = codiv_common::messages::frame_message(&msg) {
            let _ = client_tx.send(frame).await;
        }
    }
}
