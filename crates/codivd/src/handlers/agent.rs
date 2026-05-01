use crate::agent::permissions::PermissionContext;
use crate::daemon::{create_agent, truncated_name, Daemon};
use crate::ipc::server::ClientId;
use codiv_common::conversation::ConversationEvent;
use codiv_common::messages::DaemonMessage;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::info;

pub(crate) async fn handle_agent_request(
    daemon: &mut Daemon,
    client_id: ClientId,
    prompt: String,
    request_id: String,
    thinking: bool,
) {
    // Auto-compaction check: compact before spawning agent if threshold exceeded
    let should_compact = daemon
        .persistence.sessions
        .get(&client_id)
        .map(|s| s.agent_state.needs_compaction)
        .unwrap_or(false);
    if should_compact {
        super::compaction::handle_compact_request(daemon, client_id).await;
    }

    let client_tx = match daemon.ipc.client_sender(client_id) {
        Some(tx) => tx,
        None => return,
    };

    // Ensure SQLite session exists and persist the user prompt
    let session_id = daemon.persistence.ensure_session(client_id);
    let user_event = ConversationEvent::UserPrompt {
        text: prompt.clone(),
        request_id: request_id.clone(),
    };
    daemon.persistence.persist_event(client_id, &user_event);

    // If this is the first prompt, set initial name + send SessionCreated.
    if let Some(ref sid) = session_id {
        send_session_created(daemon, client_id, sid, &prompt, &client_tx).await;
    }

    // Take (or create) the agent and prepare it for the request.
    let mut agent = ensure_agent(daemon, client_id);
    agent.add_user_message(&prompt, &request_id);

    // Create or reuse permission context for this session
    let permission_ctx = ensure_permission_ctx(daemon, client_id, &client_tx);

    // Set up the RelayManager for this agent request
    {
        let session = daemon.persistence.sessions.get_mut(&client_id).unwrap();
        let relay = session
            .relay
            .relay_manager
            .get_or_insert_with(|| {
                Arc::new(crate::agent::relay_manager::RelayManager::new(
                    client_tx.clone(),
                ))
            })
            .clone();
        agent.relay_manager = Some(relay);
    }

    // Spawn the agent task and store the return channel on the session.
    let (agent_task_handle, agent_return_rx) =
        spawn_agent_task(agent, request_id, client_tx, thinking, permission_ctx, daemon.lifecycle.agent_done_tx.clone());

    if let Some(session) = daemon.persistence.sessions.get_mut(&client_id) {
        session.agent_state.agent_return_rx = Some(agent_return_rx);
        session.agent_state.agent_task = Some(agent_task_handle);
    }
}

/// Take the existing agent from the session (or create a new one) and set its cwd.
fn ensure_agent(daemon: &mut Daemon, client_id: ClientId) -> crate::agent::agent::Agent {
    let session = daemon.persistence.sessions.get_mut(&client_id);
    let (cwd, taken_agent) = match session {
        Some(s) => {
            let cwd = s.ipc.cwd.clone();
            let agent = s.agent_state.agent.take();
            (cwd, agent)
        }
        None => (String::new(), None),
    };

    let mut agent = taken_agent.unwrap_or_else(|| {
        let cfg = daemon.config.read().unwrap();
        create_agent(cwd.clone(), &cfg)
    });

    // Ensure agent uses the session's latest cwd.
    agent.cwd = cwd;
    agent
}

/// If this is the first prompt in the session, persist an initial name,
/// send `SessionCreated` to the client, and mark the session for deferred
/// LLM name generation.
async fn send_session_created(
    daemon: &mut Daemon,
    client_id: ClientId,
    session_id: &str,
    prompt: &str,
    client_tx: &mpsc::Sender<Vec<u8>>,
) {
    let is_first = daemon
        .persistence.sessions
        .get(&client_id)
        .map(|s| s.persistence.event_seq == 1)
        .unwrap_or(false);
    if !is_first {
        return;
    }

    let initial_name = truncated_name(prompt);
    if let Err(e) = daemon.persistence.store.update_session_name(session_id, &initial_name) {
        tracing::error!("failed to set initial session name: {}", e);
    }

    let msg = DaemonMessage::SessionCreated {
        session_id: session_id.to_string(),
        name: Some(initial_name.clone()),
    };
    match codiv_common::messages::frame_message(&msg) {
        Ok(frame) => {
            if client_tx.send(frame).await.is_err() {
                tracing::warn!("failed to send SessionCreated: client disconnected");
            }
        }
        Err(e) => tracing::error!("failed to frame SessionCreated: {}", e),
    }

    // Mark for deferred name generation after agent completes
    if let Some(session) = daemon.persistence.sessions.get_mut(&client_id) {
        session.persistence.pending_name_gen = Some(prompt.to_string());
    }
}

/// Create or reuse the permission context for this session.
fn ensure_permission_ctx(
    daemon: &mut Daemon,
    client_id: ClientId,
    client_tx: &mpsc::Sender<Vec<u8>>,
) -> Option<Arc<PermissionContext>> {
    if let Some(ref s) = daemon.persistence.sessions.get(&client_id) {
        if s.permissions.permission_ctx.is_some() {
            return s.permissions.permission_ctx.clone();
        }
    }

    let cfg = daemon.config.read().expect("daemon config RwLock poisoned");
    let ctx = Arc::new(PermissionContext::new(
        cfg.permissions.mode,
        client_tx.clone(),
        cfg.models.clone(),
        Arc::clone(&daemon.config),
    ));
    drop(cfg);
    if let Some(session) = daemon.persistence.sessions.get_mut(&client_id) {
        session.permissions.permission_ctx = Some(Arc::clone(&ctx));
    }
    Some(ctx)
}

/// Spawn the agent task onto the tokio runtime. Returns the task handle and
/// a oneshot receiver that will deliver the agent + collected events back.
fn spawn_agent_task(
    mut agent: crate::agent::agent::Agent,
    request_id: String,
    client_tx: mpsc::Sender<Vec<u8>>,
    thinking: bool,
    permission_ctx: Option<Arc<PermissionContext>>,
    done_tx: mpsc::Sender<()>,
) -> (
    tokio::task::JoinHandle<()>,
    tokio::sync::oneshot::Receiver<(crate::agent::agent::Agent, Vec<ConversationEvent>)>,
) {
    let (agent_return_tx, agent_return_rx) = tokio::sync::oneshot::channel();
    let rid = request_id;

    let handle = tokio::spawn(async move {
        // Send model alias before streaming starts (tokens unknown yet).
        let meta_msg = DaemonMessage::AgentMeta {
            model_alias: agent.model_config.model_alias(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            context_window: agent.model_config.context_window(),
        };
        match codiv_common::messages::frame_message(&meta_msg) {
            Ok(frame) => {
                if client_tx.send(frame).await.is_err() {
                    tracing::warn!("failed to send AgentMeta: client disconnected");
                }
            }
            Err(e) => tracing::error!("failed to frame AgentMeta: {}", e),
        }
        match agent
            .run_streaming(&rid, &client_tx, thinking, permission_ctx)
            .await
        {
            Ok((response, input_tokens, output_tokens, cache_read_tokens, tool_events)) => {
                info!(
                    "agent completed request {}: {} bytes",
                    rid,
                    response.len()
                );
                // Add tool events to agent history
                agent.add_tool_events(&tool_events);
                agent.add_assistant_message(&response, &rid);
                // Build the full set of events to persist:
                // tool events + the final assistant text
                let mut persist_events = tool_events;
                persist_events.push(ConversationEvent::AssistantText {
                    request_id: rid.clone(),
                    text: response.clone(),
                });
                persist_events.push(ConversationEvent::TokenUsage {
                    request_id: rid.clone(),
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                });
                // Send updated token usage after streaming.
                let meta_msg = DaemonMessage::AgentMeta {
                    model_alias: agent.model_config.model_alias(),
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    context_window: agent.model_config.context_window(),
                };
                if let Ok(frame) = codiv_common::messages::frame_message(&meta_msg) {
                    let _ = client_tx.send(frame).await;
                }
                let msg = DaemonMessage::AgentComplete {
                    request_id: rid,
                    summary: response,
                };
                match codiv_common::messages::frame_message(&msg) {
                    Ok(frame) => {
                        if client_tx.send(frame).await.is_err() {
                            tracing::warn!("failed to send AgentComplete: client disconnected");
                        }
                    }
                    Err(e) => tracing::error!("failed to frame AgentComplete: {}", e),
                }
                let _ = agent_return_tx.send((agent, persist_events));
            }
            Err(e) => {
                info!("agent error for request {}: {}", rid, e);
                let msg = DaemonMessage::Error {
                    request_id: rid.clone(),
                    message: e.clone(),
                };
                match codiv_common::messages::frame_message(&msg) {
                    Ok(frame) => {
                        if client_tx.send(frame).await.is_err() {
                            tracing::warn!("failed to send Error: client disconnected");
                        }
                    }
                    Err(e) => tracing::error!("failed to frame Error: {}", e),
                }
                let error_events = vec![ConversationEvent::Error {
                    request_id: rid,
                    message: e,
                }];
                let _ = agent_return_tx.send((agent, error_events));
            }
        }
        // Wake the event loop so collect_returned_agents runs immediately.
        let _ = done_tx.send(()).await;
    });

    (handle, agent_return_rx)
}

pub(crate) async fn handle_cancel_request(
    daemon: &mut Daemon,
    client_id: ClientId,
    request_id: String,
) {
    info!("cancel request: {}", request_id);
    if let Some(session) = daemon.persistence.sessions.get_mut(&client_id) {
        // Cancel all pending relay operations before aborting the task.
        if let Some(ref relay) = session.relay.relay_manager {
            relay
                .cancel_all(codiv_common::messages::CancelReason::UserAbort)
                .await;
        }
        if let Some(handle) = session.agent_state.agent_task.take() {
            handle.abort();
        }
        session.agent_state.agent_return_rx = None;

        // Rebuild agent from persisted events so context is preserved.
        if let Some(ref sid) = session.persistence.session_id {
            let events = daemon.persistence.store.load_events(sid, None).unwrap_or_default();
            let cwd = session.ipc.cwd.clone();
            let cfg = daemon.config.read().unwrap();
            let mut agent = create_agent(cwd, &cfg);
            drop(cfg);
            agent.add_tool_events(&events);
            session.agent_state.agent = Some(agent);
        }
    }
    // Send AgentComplete so client knows streaming ended.
    if let Some(client_tx) = daemon.ipc.client_sender(client_id) {
        let msg = DaemonMessage::AgentComplete {
            request_id,
            summary: String::new(),
        };
        match codiv_common::messages::frame_message(&msg) {
            Ok(frame) => {
                if client_tx.send(frame).await.is_err() {
                    tracing::warn!("failed to send AgentComplete (cancel): client disconnected");
                }
            }
            Err(e) => tracing::error!("failed to frame AgentComplete: {}", e),
        }
    }
}
