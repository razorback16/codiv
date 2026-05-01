use crate::daemon::{create_agent, Daemon};
use crate::ipc::server::ClientId;
use codiv_common::conversation::ConversationEvent;
use codiv_common::messages::DaemonMessage;
use std::time::Duration;
use tracing::info;

pub(crate) async fn handle_compact_request(daemon: &mut Daemon, client_id: ClientId) {
    let models = {
        let cfg = daemon.config.read().unwrap();
        cfg.models.clone()
    };

    let session = match daemon.persistence.sessions.get_mut(&client_id) {
        Some(s) => s,
        None => return,
    };
    session.agent_state.needs_compaction = false;

    let agent = match session.agent_state.agent.as_mut() {
        Some(a) => a,
        None => return,
    };

    let compacted_event_count = agent.history.len();
    let request_id = uuid::Uuid::new_v4().to_string();

    let client_tx = match daemon.ipc.client_sender(client_id) {
        Some(tx) => tx,
        None => return,
    };

    // Send CompactionStarted so TUI can clear blocks and show tool header
    let msg = DaemonMessage::CompactionStarted {
        request_id: request_id.clone(),
        compacted_event_count,
    };
    if let Ok(frame) = codiv_common::messages::frame_message(&msg) {
        let _ = client_tx.send(frame).await;
    }

    match tokio::time::timeout(
        Duration::from_secs(60),
        agent.compact(&models, &client_tx, &request_id),
    )
    .await
    {
        Ok(Ok((summary, count, input_tokens, output_tokens, cache_read_tokens))) => {
            info!("compaction completed: {} events summarized", count);

            // Persist Summary event to the OLD session
            let event = ConversationEvent::Summary {
                text: summary.clone(),
                compacted_event_count: count,
            };
            daemon.persistence.persist_event(client_id, &event);

            // Send AgentComplete to finalize the AI response block (critical)
            let complete_msg = DaemonMessage::AgentComplete {
                request_id: request_id.clone(),
                summary: summary.clone(),
            };
            match codiv_common::messages::frame_message(&complete_msg) {
                Ok(frame) => {
                    if client_tx.send(frame).await.is_err() {
                        tracing::warn!("failed to send AgentComplete (compaction): client disconnected");
                    }
                }
                Err(e) => tracing::error!("failed to frame AgentComplete: {}", e),
            }

            // Create NEW session — old session is archived
            if let Some(session) = daemon.persistence.sessions.get_mut(&client_id) {
                session.persistence.session_id = None;
                session.persistence.event_seq = 0;
                // Create fresh agent with Summary as first history event
                let cwd = session.ipc.cwd.clone();
                let mut new_agent = {
                    let cfg = daemon.config.read().unwrap();
                    create_agent(cwd, &cfg)
                };
                new_agent.history.push(ConversationEvent::Summary {
                    text: summary.clone(),
                    compacted_event_count: count,
                });
                session.agent_state.agent = Some(new_agent);
            }

            // Create the new SQLite session and persist the Summary event
            let new_sid = daemon.persistence.ensure_session(client_id);
            daemon.persistence.persist_event(client_id, &event);

            // Send SessionCreated BEFORE AgentMeta so the token_usage.reset()
            // in the TUI happens first, then AgentMeta records the new baseline.
            if let Some(ref sid) = new_sid {
                let name = "Compacted conversation".to_string();
                if let Err(e) = daemon.persistence.store.update_session_name(sid, &name) {
                    tracing::error!("failed to set compacted session name: {}", e);
                }
                let session_msg = DaemonMessage::SessionCreated {
                    session_id: sid.clone(),
                    name: Some(name),
                };
                match codiv_common::messages::frame_message(&session_msg) {
                    Ok(frame) => {
                        if client_tx.send(frame).await.is_err() {
                            tracing::warn!("failed to send SessionCreated (compaction): client disconnected");
                        }
                    }
                    Err(e) => tracing::error!("failed to frame SessionCreated: {}", e),
                }
            }

            // Send AgentMeta with token usage (after SessionCreated so reset happens first)
            let context_window = if let Some(session) = daemon.persistence.sessions.get(&client_id) {
                session
                    .agent_state
                    .agent
                    .as_ref()
                    .map(|a| a.model_config.context_window())
                    .unwrap_or(0)
            } else {
                0
            };
            let meta_msg = DaemonMessage::AgentMeta {
                model_alias: String::new(),
                input_tokens,
                output_tokens,
                cache_read_tokens,
                context_window,
            };
            if let Ok(frame) = codiv_common::messages::frame_message(&meta_msg) {
                let _ = client_tx.send(frame).await;
            }

            // Send CompactionComplete to finalize
            let done_msg = DaemonMessage::CompactionComplete {
                summary,
                compacted_event_count: count,
            };
            if let Ok(frame) = codiv_common::messages::frame_message(&done_msg) {
                let _ = client_tx.send(frame).await;
            }
        }
        Ok(Err(e)) => {
            tracing::error!("compaction failed: {}", e);
            // Send AgentComplete to clean up streaming state (critical)
            let complete_msg = DaemonMessage::AgentComplete {
                request_id: request_id.clone(),
                summary: String::new(),
            };
            match codiv_common::messages::frame_message(&complete_msg) {
                Ok(frame) => {
                    if client_tx.send(frame).await.is_err() {
                        tracing::warn!("failed to send AgentComplete (compaction error): client disconnected");
                    }
                }
                Err(e2) => tracing::error!("failed to frame AgentComplete: {}", e2),
            }
            let msg = DaemonMessage::Notice {
                message: format!("Compaction failed: {}", e),
            };
            if let Ok(frame) = codiv_common::messages::frame_message(&msg) {
                let _ = client_tx.send(frame).await;
            }
        }
        Err(_) => {
            tracing::error!("compaction timed out after 60s");
            // Send AgentComplete to clean up streaming state (critical)
            let complete_msg = DaemonMessage::AgentComplete {
                request_id,
                summary: String::new(),
            };
            match codiv_common::messages::frame_message(&complete_msg) {
                Ok(frame) => {
                    if client_tx.send(frame).await.is_err() {
                        tracing::warn!("failed to send AgentComplete (compaction timeout): client disconnected");
                    }
                }
                Err(e) => tracing::error!("failed to frame AgentComplete: {}", e),
            }
            let msg = DaemonMessage::Notice {
                message: "Compaction timed out".to_string(),
            };
            if let Ok(frame) = codiv_common::messages::frame_message(&msg) {
                let _ = client_tx.send(frame).await;
            }
        }
    }
}
