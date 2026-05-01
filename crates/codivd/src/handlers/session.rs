use crate::daemon::{create_agent, Daemon};
use crate::ipc::server::ClientId;
use codiv_common::messages::DaemonMessage;
use tracing::info;

pub(crate) async fn handle_new_session(daemon: &mut Daemon, client_id: ClientId) {
    let client_tx = match daemon.ipc.client_sender(client_id) {
        Some(tx) => tx,
        None => return,
    };

    // Create a fresh agent
    if let Some(session) = daemon.sessions.get_mut(&client_id) {
        let cwd = session.ipc.cwd.clone();
        let cfg = daemon.config.read().unwrap();
        let agent = create_agent(cwd.clone(), &cfg);
        drop(cfg);
        session.agent_state.agent = Some(agent);
        session.persistence.session_id = None;
        session.persistence.event_seq = 0;
    }

    // Create a new SQLite session
    let sid = daemon.ensure_session(client_id);

    if let Some(sid) = sid {
        let msg = DaemonMessage::SessionCreated {
            session_id: sid,
            name: None,
        };
        match codiv_common::messages::frame_message(&msg) {
            Ok(frame) => {
                if client_tx.send(frame).await.is_err() {
                    tracing::warn!("failed to send SessionCreated (NewSession): client disconnected");
                }
            }
            Err(e) => tracing::error!("failed to frame SessionCreated: {}", e),
        }
    }
}

pub(crate) async fn handle_load_session(
    daemon: &mut Daemon,
    client_id: ClientId,
    target_sid: String,
) {
    let client_tx = match daemon.ipc.client_sender(client_id) {
        Some(tx) => tx,
        None => return,
    };

    // Load all events for the target session
    let events = daemon
        .store
        .load_events(&target_sid, None)
        .unwrap_or_default();

    // Look up session info for the name
    let session_name = daemon
        .store
        .list_sessions(100)
        .unwrap_or_default()
        .into_iter()
        .find(|s| s.id == target_sid)
        .and_then(|s| s.name);

    // Update the client session to point at the loaded session
    if let Some(session) = daemon.sessions.get_mut(&client_id) {
        // Clear agent history and reload from events
        let cwd = session.ipc.cwd.clone();
        let cfg = daemon.config.read().unwrap();
        let mut agent = create_agent(cwd, &cfg);
        drop(cfg);
        agent.add_tool_events(&events);
        session.agent_state.agent = Some(agent);
        session.persistence.session_id = Some(target_sid.clone());
        // Set event_seq to the number of persisted events so new
        // events continue from the correct sequence number.
        session.persistence.event_seq = events.len() as u32;
    }

    // Confirm session switch
    let created_msg = DaemonMessage::SessionCreated {
        session_id: target_sid.clone(),
        name: session_name,
    };
    match codiv_common::messages::frame_message(&created_msg) {
        Ok(frame) => {
            if client_tx.send(frame).await.is_err() {
                tracing::warn!("failed to send SessionCreated (LoadSession): client disconnected");
            }
        }
        Err(e) => tracing::error!("failed to frame SessionCreated: {}", e),
    }

    // Send full event replay
    let replay_msg = DaemonMessage::SessionReplay { events };
    match codiv_common::messages::frame_message(&replay_msg) {
        Ok(frame) => {
            if client_tx.send(frame).await.is_err() {
                tracing::warn!("failed to send SessionReplay: client disconnected");
            }
        }
        Err(e) => tracing::error!("failed to frame SessionReplay: {}", e),
    }
}

pub(crate) async fn handle_list_sessions(daemon: &mut Daemon, client_id: ClientId) {
    let client_tx = match daemon.ipc.client_sender(client_id) {
        Some(tx) => tx,
        None => return,
    };

    let current_sid = daemon
        .sessions
        .get(&client_id)
        .and_then(|s| s.persistence.session_id.as_deref());
    let sessions: Vec<_> = daemon
        .store
        .list_sessions(50)
        .unwrap_or_default()
        .into_iter()
        .filter(|s| current_sid != Some(s.id.as_str()))
        .collect();
    let msg = DaemonMessage::SessionList { sessions };
    if let Ok(frame) = codiv_common::messages::frame_message(&msg) {
        let _ = client_tx.send(frame).await;
    }
}

pub(crate) async fn handle_command_result(
    daemon: &mut Daemon,
    client_id: ClientId,
    command: String,
    output: String,
    exit_code: i32,
    cwd: String,
) {
    use codiv_common::conversation::ConversationEvent;
    use std::sync::Arc;

    // Ensure a session exists so we can persist the shell command
    daemon.ensure_session(client_id);

    // Name the session if this is the first event (command-initiated session)
    if let Some(session) = daemon.sessions.get(&client_id) {
        if session.persistence.event_seq == 0 {
            if let Some(sid) = session.persistence.session_id.clone() {
                let base_cmd = command.split_whitespace().next().unwrap_or(&command);
                let name = format!("command {}", base_cmd);
                if let Err(e) = daemon.store.update_session_name(&sid, &name) {
                    tracing::error!("failed to set command session name: {}", e);
                }
                if let Some(client_tx) = daemon.ipc.client_sender(client_id) {
                    let msg = DaemonMessage::SessionCreated {
                        session_id: sid,
                        name: Some(name),
                    };
                    match codiv_common::messages::frame_message(&msg) {
                        Ok(frame) => {
                            if client_tx.send(frame).await.is_err() {
                                tracing::warn!("failed to send SessionCreated (CommandResult): client disconnected");
                            }
                        }
                        Err(e) => tracing::error!("failed to frame SessionCreated: {}", e),
                    }
                }
            }
        }
    }

    // Persist ShellCommand event (truncate large output)
    let truncated_output = if output.len() > 10000 {
        // Find the largest index <= 10000 that falls on a UTF-8 char
        // boundary to avoid panicking on multi-byte characters.
        let mut end = 10000;
        while end > 0 && !output.is_char_boundary(end) {
            end -= 1;
        }
        let mut t = output[..end].to_string();
        t.push_str("\n... (truncated)");
        t
    } else {
        output.clone()
    };
    let event = ConversationEvent::ShellCommand {
        command: command.clone(),
        output: truncated_output,
        exit_code,
        cwd: cwd.clone(),
    };
    daemon.persist_event(client_id, &event);

    if let Some(session) = daemon.sessions.get_mut(&client_id) {
        // Keep session cwd in sync with the client's actual cwd.
        session.ipc.cwd = cwd.clone();

        let session_cwd = session.ipc.cwd.clone();
        let config_ref = Arc::clone(&daemon.config);
        let agent = session.agent_state.agent.get_or_insert_with(|| {
            let cfg = config_ref.read().unwrap();
            create_agent(session_cwd, &cfg)
        });
        // Update the agent's cwd so tools execute in the right directory.
        agent.cwd = cwd.clone();
        agent.add_command_result(&command, &output, exit_code, &cwd);
        info!(
            "recorded command result from client {}: {}",
            client_id, command
        );
    }
}

/// Clean up sessions that have not sent a heartbeat within `timeout_secs`.
pub(crate) async fn cleanup_stale_sessions(daemon: &mut Daemon) {
    let stale: Vec<ClientId> = daemon
        .sessions
        .iter()
        .filter(|(_, s)| s.is_stale(120))
        .map(|(id, _)| *id)
        .collect();

    for id in stale {
        // If the session has an active agent task, abort it before removal
        // to prevent the agent's return channel send from silently failing
        // and losing results.
        if let Some(session) = daemon.sessions.get(&id) {
            if session.agent_state.agent_task.is_some() {
                info!(
                    "aborting active agent task for stale session {} before cleanup",
                    id
                );
            }
        }
        if let Some(mut session) = daemon.sessions.remove(&id) {
            info!("cleaning stale session {}", id);
            // Cancel all pending relay operations
            if let Some(ref relay) = session.relay.relay_manager {
                relay
                    .cancel_all(codiv_common::messages::CancelReason::SessionStale)
                    .await;
            }
            // Abort the agent task so it doesn't try to send on a dropped channel
            if let Some(task) = session.agent_state.agent_task.take() {
                task.abort();
            }
        }
        daemon.ipc.disconnect(id);
    }
}
