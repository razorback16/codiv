use crate::agent::permissions::PermissionContext;
use crate::daemon::{create_agent, truncated_name, Daemon};
use crate::ipc::server::ClientId;
use codiv_common::conversation::ConversationEvent;
use codiv_common::messages::DaemonMessage;
use std::sync::Arc;
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
        .sessions
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

    let rid = request_id.clone();

    // Ensure SQLite session exists
    let session_id = daemon.ensure_session(client_id);

    // Persist the UserPrompt event
    let user_event = ConversationEvent::UserPrompt {
        text: prompt.clone(),
        request_id: request_id.clone(),
    };
    daemon.persist_event(client_id, &user_event);

    // If this is the first prompt, set initial name + send SessionCreated.
    // Background LLM name generation is deferred until the agent
    // completes (in collect_returned_agents) so it doesn't compete
    // with the main LLM call.
    if let Some(ref sid) = session_id {
        let session = daemon.sessions.get(&client_id);
        let is_first = session
            .map(|s| s.persistence.event_seq == 1)
            .unwrap_or(false);
        if is_first {
            let initial_name = truncated_name(&prompt);
            if let Err(e) = daemon.store.update_session_name(sid, &initial_name) {
                tracing::error!("failed to set initial session name: {}", e);
            }
            // Send SessionCreated to client
            let msg = DaemonMessage::SessionCreated {
                session_id: sid.clone(),
                name: Some(initial_name.clone()),
            };
            if let Ok(frame) = codiv_common::messages::frame_message(&msg) {
                let _ = client_tx.send(frame).await;
            }
            // Mark for deferred name generation after agent completes
            if let Some(session) = daemon.sessions.get_mut(&client_id) {
                session.persistence.pending_name_gen = Some(prompt.clone());
            }
        }
    }

    // Take the agent out of the session so we can move it into the task.
    // If none exists yet, create one.
    let session = daemon.sessions.get_mut(&client_id);
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
    agent.add_user_message(&prompt, &request_id);

    // Create or reuse permission context for this session
    let permission_ctx = if let Some(ref s) = daemon.sessions.get(&client_id) {
        s.permissions.permission_ctx.clone()
    } else {
        None
    };
    let permission_ctx = match permission_ctx {
        Some(ctx) => Some(ctx),
        None => {
            let cfg = daemon.config.read().unwrap();
            let ctx = Arc::new(PermissionContext::new(
                cfg.permissions.mode,
                client_tx.clone(),
                cfg.models.clone(),
            ));
            if let Some(session) = daemon.sessions.get_mut(&client_id) {
                session.permissions.permission_ctx = Some(Arc::clone(&ctx));
            }
            Some(ctx)
        }
    };

    // Set up the RelayManager for this agent request
    {
        let session = daemon.sessions.get_mut(&client_id).unwrap();
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

    // We need to put the agent back after the spawn completes.
    // Use a channel to return it along with collected tool events.
    let (agent_return_tx, agent_return_rx) = tokio::sync::oneshot::channel::<(
        crate::agent::agent::Agent,
        Vec<ConversationEvent>,
    )>();

    let done_tx = daemon.agent_done_tx.clone();
    let agent_task_handle = tokio::spawn(async move {
        // Send model alias before streaming starts (tokens unknown yet).
        let meta_msg = DaemonMessage::AgentMeta {
            model_alias: agent.model_config.model_alias(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            context_window: agent.model_config.context_window(),
        };
        if let Ok(frame) = codiv_common::messages::frame_message(&meta_msg) {
            let _ = client_tx.send(frame).await;
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
                if let Ok(frame) = codiv_common::messages::frame_message(&msg) {
                    let _ = client_tx.send(frame).await;
                }
                let _ = agent_return_tx.send((agent, persist_events));
            }
            Err(e) => {
                info!("agent error for request {}: {}", rid, e);
                let msg = DaemonMessage::Error {
                    request_id: rid.clone(),
                    message: e.clone(),
                };
                if let Ok(frame) = codiv_common::messages::frame_message(&msg) {
                    let _ = client_tx.send(frame).await;
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

    // Store the receiver on the session so collect_returned_agents picks it up.
    if let Some(session) = daemon.sessions.get_mut(&client_id) {
        session.agent_state.agent_return_rx = Some(agent_return_rx);
        session.agent_state.agent_task = Some(agent_task_handle);
    }
}

pub(crate) async fn handle_cancel_request(
    daemon: &mut Daemon,
    client_id: ClientId,
    request_id: String,
) {
    info!("cancel request: {}", request_id);
    if let Some(session) = daemon.sessions.get_mut(&client_id) {
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
            let events = daemon.store.load_events(sid, None).unwrap_or_default();
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
        if let Ok(frame) = codiv_common::messages::frame_message(&msg) {
            let _ = client_tx.send(frame).await;
        }
    }
}
