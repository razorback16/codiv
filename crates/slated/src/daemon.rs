use crate::agent::permissions::PermissionContext;
use crate::ipc::server::{ClientId, IpcServer};
use crate::session::ClientSession;
use crate::store::SessionStore;
use slate_common::conversation::ConversationEvent;
use slate_common::messages::{ClientMessage, DaemonMessage};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::info;

const SYSTEM_PROMPT: &str = "You are a helpful coding assistant embedded in a terminal. \
    You can see the user's recent terminal commands and their output in the conversation history. \
    Use this context to give relevant, concise answers. \
    When referencing files or directories, use paths relative to the user's current working directory when possible.";

fn create_agent(cwd: String, env_vars: Vec<(String, String)>) -> crate::agent::agent::Agent {
    let config = crate::agent::config::AppConfig::load();
    let assignment = config.models.assignment_for(&slate_common::types::AgentRole::Engineer);
    let provider_config = config.models.resolve_provider_config(&assignment);
    crate::agent::agent::Agent::new(
        slate_common::types::AgentRole::Engineer,
        assignment,
        provider_config,
        SYSTEM_PROMPT.to_string(),
        cwd,
        env_vars,
    )
}

/// Generate a short initial name from the first user prompt (truncated to
/// 60 chars on a word boundary).
fn truncated_name(prompt: &str) -> String {
    let trimmed = prompt.trim().replace('\n', " ");
    if trimmed.len() <= 60 {
        return trimmed;
    }
    // Find a word boundary at or before 60
    let mut end = 60;
    while end > 0 && !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    // Walk back to last space
    if let Some(pos) = trimmed[..end].rfind(' ') {
        format!("{}…", &trimmed[..pos])
    } else {
        format!("{}…", &trimmed[..end])
    }
}

pub struct Daemon {
    ipc: IpcServer,
    sessions: HashMap<ClientId, ClientSession>,
    store: SessionStore,
    /// Notifies the event loop when a spawned agent task completes so
    /// `collect_returned_agents` runs immediately (not on next IPC message).
    agent_done_tx: tokio::sync::mpsc::Sender<()>,
    agent_done_rx: tokio::sync::mpsc::Receiver<()>,
    /// Channel for background name-gen tasks to send results back to the
    /// event loop so the name can be persisted to SQLite.
    name_update_tx: tokio::sync::mpsc::Sender<(String, String, tokio::sync::mpsc::Sender<Vec<u8>>)>,
    name_update_rx: tokio::sync::mpsc::Receiver<(String, String, tokio::sync::mpsc::Sender<Vec<u8>>)>,
}

impl Daemon {
    pub async fn new() -> std::io::Result<Self> {
        // Eagerly load config so the default config.toml is created on
        // first startup (before any client connects).
        let _ = crate::agent::config::AppConfig::load();

        let ipc = IpcServer::new().await?;
        let store = SessionStore::open().map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, format!("sqlite: {}", e))
        })?;
        let (agent_done_tx, agent_done_rx) = tokio::sync::mpsc::channel(16);
        let (name_update_tx, name_update_rx) = tokio::sync::mpsc::channel(16);
        Ok(Self {
            ipc,
            sessions: HashMap::new(),
            store,
            agent_done_tx,
            agent_done_rx,
            name_update_tx,
            name_update_rx,
        })
    }

    pub async fn run(mut self) {
        let mut cleanup_interval = tokio::time::interval(Duration::from_secs(30));

        loop {
            tokio::select! {
                result = self.ipc.listener.accept() => {
                    if let Ok((stream, _)) = result {
                        let client_id = self.ipc.register_client(stream);
                        self.sessions.insert(client_id, ClientSession::new());
                    }
                }
                Some(msg) = self.ipc.msg_rx.recv() => {
                    self.collect_returned_agents();
                    self.dispatch(msg.client_id, msg.message).await;
                }
                Some(_) = self.agent_done_rx.recv() => {
                    self.collect_returned_agents();
                }
                Some((session_id, name, client_tx)) = self.name_update_rx.recv() => {
                    if let Err(e) = self.store.update_session_name(&session_id, &name) {
                        tracing::error!("failed to persist generated session name: {}", e);
                    }
                    let msg = DaemonMessage::SessionNameUpdated {
                        session_id,
                        name,
                    };
                    if let Ok(frame) = slate_common::messages::frame_message(&msg) {
                        let _ = client_tx.send(frame).await;
                    }
                }
                _ = cleanup_interval.tick() => {
                    self.collect_returned_agents();
                    self.cleanup_stale_sessions().await;
                }
            }
        }
    }

    /// Ensure the client has a SQLite session. Returns the session_id.
    fn ensure_session(&mut self, client_id: ClientId) -> Option<String> {
        let session = self.sessions.get_mut(&client_id)?;
        if let Some(ref id) = session.session_id {
            return Some(id.clone());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let cwd = session.cwd.clone();
        if let Err(e) = self.store.create_session(&id, &cwd) {
            tracing::error!("failed to create session in sqlite: {}", e);
            return None;
        }
        session.session_id = Some(id.clone());
        Some(id)
    }

    /// Persist a single event and bump the session's seq counter.
    fn persist_event(&mut self, client_id: ClientId, event: &ConversationEvent) {
        let session = match self.sessions.get_mut(&client_id) {
            Some(s) => s,
            None => return,
        };
        if session.session_id.is_none() {
            return;
        }
        let seq = session.next_seq();
        let sid = session.session_id.as_deref().unwrap();
        if let Err(e) = self.store.append_event(sid, seq, event) {
            tracing::error!("failed to persist event seq={}: {}", seq, e);
        }
    }

    /// Persist multiple events and bump the session's seq counter.
    fn persist_events(&mut self, client_id: ClientId, events: &[ConversationEvent]) {
        for event in events {
            self.persist_event(client_id, event);
        }
    }

    async fn dispatch(&mut self, client_id: ClientId, msg: ClientMessage) {
        match msg {
            ClientMessage::AgentRequest {
                prompt,
                request_id,
                context: _,
                thinking,
            } => {
                if let Some(client_tx) = self.ipc.client_sender(client_id) {
                    let rid = request_id.clone();

                    // Ensure SQLite session exists
                    let session_id = self.ensure_session(client_id);

                    // Persist the UserPrompt event
                    let user_event = ConversationEvent::UserPrompt {
                        text: prompt.clone(),
                        request_id: request_id.clone(),
                    };
                    self.persist_event(client_id, &user_event);

                    // If this is the first prompt, set initial name + send SessionCreated.
                    // Background LLM name generation is deferred until the agent
                    // completes (in collect_returned_agents) so it doesn't compete
                    // with the main LLM call.
                    if let Some(ref sid) = session_id {
                        let session = self.sessions.get(&client_id);
                        let is_first = session.map(|s| s.event_seq == 1).unwrap_or(false);
                        if is_first {
                            let initial_name = truncated_name(&prompt);
                            if let Err(e) = self.store.update_session_name(sid, &initial_name) {
                                tracing::error!("failed to set initial session name: {}", e);
                            }
                            // Send SessionCreated to client
                            let msg = DaemonMessage::SessionCreated {
                                session_id: sid.clone(),
                                name: Some(initial_name.clone()),
                            };
                            if let Ok(frame) = slate_common::messages::frame_message(&msg) {
                                let _ = client_tx.send(frame).await;
                            }
                            // Mark for deferred name generation after agent completes
                            if let Some(session) = self.sessions.get_mut(&client_id) {
                                session.pending_name_gen = Some(prompt.clone());
                            }
                        }
                    }

                    // Take the agent out of the session so we can move it into the task.
                    // If none exists yet, create one.
                    let session = self.sessions.get_mut(&client_id);
                    let (cwd, env_vars, taken_agent) = match session {
                        Some(s) => {
                            let cwd = s.cwd.clone();
                            let env = s.env_vars.clone();
                            let agent = s.agent.take();
                            (cwd, env, agent)
                        }
                        None => (String::new(), Vec::new(), None),
                    };

                    let mut agent = taken_agent.unwrap_or_else(|| {
                        create_agent(cwd.clone(), env_vars)
                    });

                    // Ensure agent uses the session's latest cwd.
                    agent.cwd = cwd;
                    agent.add_user_message(&prompt, &request_id);

                    // Create or reuse permission context for this session
                    let permission_ctx = if let Some(ref s) = self.sessions.get(&client_id) {
                        s.permission_ctx.clone()
                    } else {
                        None
                    };
                    let permission_ctx = match permission_ctx {
                        Some(ctx) => Some(ctx),
                        None => {
                            let config = crate::agent::config::AppConfig::load();
                            let ctx = Arc::new(PermissionContext::new(
                                config.permissions.mode,
                                client_tx.clone(),
                                config.models,
                            ));
                            if let Some(session) = self.sessions.get_mut(&client_id) {
                                session.permission_ctx = Some(Arc::clone(&ctx));
                            }
                            Some(ctx)
                        }
                    };

                    // We need to put the agent back after the spawn completes.
                    // Use a channel to return it along with collected tool events.
                    let (agent_return_tx, agent_return_rx) =
                        tokio::sync::oneshot::channel::<(crate::agent::agent::Agent, Vec<ConversationEvent>)>();

                    let done_tx = self.agent_done_tx.clone();
                    tokio::spawn(async move {
                        // Send model alias before streaming starts (tokens unknown yet).
                        let meta_msg = DaemonMessage::AgentMeta {
                            model_alias: agent.model_config.model_alias(),
                            total_tokens: 0,
                            context_window: agent.model_config.context_window(),
                        };
                        if let Ok(frame) = slate_common::messages::frame_message(&meta_msg) {
                            let _ = client_tx.send(frame).await;
                        }
                        match agent.run_streaming(&rid, &client_tx, thinking, permission_ctx).await {
                            Ok((response, input_tokens, output_tokens, tool_events)) => {
                                info!("agent completed request {}: {} bytes", rid, response.len());
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
                                // Send updated token usage after streaming.
                                let meta_msg = DaemonMessage::AgentMeta {
                                    model_alias: agent.model_config.model_alias(),
                                    total_tokens: input_tokens + output_tokens,
                                    context_window: agent.model_config.context_window(),
                                };
                                if let Ok(frame) = slate_common::messages::frame_message(&meta_msg) {
                                    let _ = client_tx.send(frame).await;
                                }
                                let msg = DaemonMessage::AgentComplete {
                                    request_id: rid,
                                    summary: response,
                                };
                                if let Ok(frame) = slate_common::messages::frame_message(&msg) {
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
                                if let Ok(frame) = slate_common::messages::frame_message(&msg) {
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
                    if let Some(session) = self.sessions.get_mut(&client_id) {
                        session.agent_return_rx = Some(agent_return_rx);
                    }
                }
            }

            ClientMessage::EnvSnapshot {
                env_vars,
                path,
                cwd,
            } => {
                if let Some(session) = self.sessions.get_mut(&client_id) {
                    session.env_vars = env_vars;
                    session.path = path;
                    session.cwd = cwd;
                    info!("received env snapshot from client {}", client_id);
                }
            }

            ClientMessage::Heartbeat { timestamp: _ } => {
                if let Some(session) = self.sessions.get_mut(&client_id) {
                    session.last_heartbeat = std::time::Instant::now();
                }
                let ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                self.ipc
                    .send(client_id, &DaemonMessage::Heartbeat { timestamp: ts })
                    .await;
            }

            ClientMessage::Shutdown { reason } => {
                info!("shutdown requested: {}", reason);
                std::process::exit(0);
            }

            ClientMessage::Confirmation { request_id, approved, add_to_allowlist, add_to_denylist, comment } => {
                if let Some(session) = self.sessions.get(&client_id) {
                    if let Some(ref pctx) = session.permission_ctx {
                        let mut pending = pctx.pending.lock().unwrap();
                        if let Some(sender) = pending.remove(&request_id) {
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

            ClientMessage::SetPermissionMode { mode } => {
                info!("permission mode change requested: {}", mode);
                if let Some(session) = self.sessions.get_mut(&client_id) {
                    match session.permission_ctx {
                        Some(ref pctx) => {
                            pctx.set_mode(mode);
                        }
                        None => {
                            // Create PermissionContext eagerly so the mode is
                            // applied even before the first AgentRequest.
                            if let Some(client_tx) = self.ipc.client_sender(client_id) {
                                let config = crate::agent::config::AppConfig::load();
                                let ctx = Arc::new(PermissionContext::new(
                                    mode,
                                    client_tx,
                                    config.models,
                                ));
                                session.permission_ctx = Some(ctx);
                            }
                        }
                    }
                }
                // Acknowledge the mode change back to the client.
                if let Some(client_tx) = self.ipc.client_sender(client_id) {
                    let msg = DaemonMessage::PermissionModeChanged { mode };
                    if let Ok(frame) = slate_common::messages::frame_message(&msg) {
                        let _ = client_tx.send(frame).await;
                    }
                }
            }

            ClientMessage::ListSessions => {
                if let Some(client_tx) = self.ipc.client_sender(client_id) {
                    let sessions = self.store.list_sessions(20).unwrap_or_default();
                    let msg = DaemonMessage::SessionList { sessions };
                    if let Ok(frame) = slate_common::messages::frame_message(&msg) {
                        let _ = client_tx.send(frame).await;
                    }
                }
            }

            ClientMessage::LoadSession { session_id: target_sid } => {
                if let Some(client_tx) = self.ipc.client_sender(client_id) {
                    // Load all events for the target session
                    let events = self.store.load_events(&target_sid, None).unwrap_or_default();

                    // Look up session info for the name
                    let session_name = self.store.list_sessions(100)
                        .unwrap_or_default()
                        .into_iter()
                        .find(|s| s.id == target_sid)
                        .and_then(|s| s.name);

                    // Update the client session to point at the loaded session
                    if let Some(session) = self.sessions.get_mut(&client_id) {
                        // Clear agent history and reload from events
                        let cwd = session.cwd.clone();
                        let env_vars = session.env_vars.clone();
                        let mut agent = create_agent(cwd, env_vars);
                        agent.add_tool_events(&events);
                        session.agent = Some(agent);
                        session.session_id = Some(target_sid.clone());
                        // Set event_seq to the number of persisted events so new
                        // events continue from the correct sequence number.
                        session.event_seq = events.len() as u32;
                    }

                    // Confirm session switch
                    let created_msg = DaemonMessage::SessionCreated {
                        session_id: target_sid.clone(),
                        name: session_name,
                    };
                    if let Ok(frame) = slate_common::messages::frame_message(&created_msg) {
                        let _ = client_tx.send(frame).await;
                    }

                    // Send full event replay
                    let replay_msg = DaemonMessage::SessionReplay { events };
                    if let Ok(frame) = slate_common::messages::frame_message(&replay_msg) {
                        let _ = client_tx.send(frame).await;
                    }
                }
            }

            ClientMessage::CommandResult {
                command,
                output,
                exit_code,
                cwd,
            } => {
                // Ensure a session exists so we can persist the shell command
                self.ensure_session(client_id);

                // Persist ShellCommand event (truncate large output)
                let truncated_output = if output.len() > 10000 {
                    let mut t = output[..10000].to_string();
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
                self.persist_event(client_id, &event);

                if let Some(session) = self.sessions.get_mut(&client_id) {
                    // Keep session cwd in sync with the client's actual cwd.
                    session.cwd = cwd.clone();

                    let session_cwd = session.cwd.clone();
                    let session_env_vars = session.env_vars.clone();
                    let agent = session.agent.get_or_insert_with(|| {
                        create_agent(session_cwd, session_env_vars)
                    });
                    // Update the agent's cwd so tools execute in the right directory.
                    agent.cwd = cwd.clone();
                    agent.add_command_result(&command, &output, exit_code, &cwd);
                    info!("recorded command result from client {}: {}", client_id, command);
                }
            }
        }
    }

    fn collect_returned_agents(&mut self) {
        // Collect completed agents from all sessions. We need to batch
        // the persistence calls because we can't borrow self mutably while
        // iterating, so collect the data first.
        let mut to_persist: Vec<(ClientId, Vec<ConversationEvent>)> = Vec::new();
        // Collect deferred name-gen requests: (session_id, prompt, client_tx)
        let mut name_gen_requests: Vec<(String, String, tokio::sync::mpsc::Sender<Vec<u8>>)> = Vec::new();

        for (&cid, session) in self.sessions.iter_mut() {
            if let Some(ref mut rx) = session.agent_return_rx {
                match rx.try_recv() {
                    Ok((agent, events)) => {
                        session.agent = Some(agent);
                        session.agent_return_rx = None;
                        if !events.is_empty() {
                            to_persist.push((cid, events));
                        }
                        // If this session has a pending name generation, fire it
                        // now that the main agent task is done.
                        if let Some(prompt) = session.pending_name_gen.take() {
                            if let Some(ref sid) = session.session_id {
                                if let Some(tx) = self.ipc.client_sender(cid) {
                                    name_gen_requests.push((sid.clone(), prompt, tx));
                                }
                            }
                        }
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                        session.agent_return_rx = None;
                    }
                }
            }
        }

        // Now persist collected events
        for (cid, events) in to_persist {
            self.persist_events(cid, &events);
        }

        // Spawn deferred background name generation tasks
        if !name_gen_requests.is_empty() {
            info!("spawning {} deferred name generation task(s)", name_gen_requests.len());
        }
        for (sid, prompt, client_tx) in name_gen_requests {
            let name_update_tx = self.name_update_tx.clone();
            tokio::spawn(async move {
                // Use a timeout so name generation never blocks the inference
                // server for too long (important for local models with single-
                // request concurrency).
                match tokio::time::timeout(
                    Duration::from_secs(30),
                    generate_session_name(&prompt),
                ).await {
                    Ok(Some(name)) => {
                        info!("session name generated: {:?}", name);
                        // Send back to the event loop so the name is persisted
                        // to SQLite (the store is not available in this task).
                        let _ = name_update_tx.send((sid, name, client_tx)).await;
                    }
                    Ok(None) => {
                        info!("session name generation returned None");
                    }
                    Err(_) => {
                        info!("session name generation timed out");
                    }
                }
            });
        }
    }

    async fn cleanup_stale_sessions(&mut self) {
        let stale: Vec<ClientId> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.is_stale(120))
            .map(|(id, _)| *id)
            .collect();

        for id in stale {
            info!("cleaning stale session {}", id);
            self.sessions.remove(&id);
            self.ipc.disconnect(id);
        }
    }
}

/// Attempt to generate a short session title using a cheap LLM call.
/// Returns `None` on failure (network error, no API key, etc.).
async fn generate_session_name(prompt: &str) -> Option<String> {
    let config = crate::agent::config::AppConfig::load();
    // Only use "session_namer" role — don't fall back to other roles
    // which may point to expensive models. If not configured, the
    // truncated prompt name is good enough.
    let assignment = match config.models.roles.get("session_namer").cloned() {
        Some(a) => a,
        None => {
            info!("generate_session_name: no session_namer role configured, skipping");
            return None;
        }
    };
    let provider_config = config.models.resolve_provider_config(&assignment);

    info!("generate_session_name: using {}/{}", assignment.provider, assignment.model);

    let messages = aisdk::core::messages::Message::builder()
        .system("Generate a concise 3-5 word title for this conversation. Reply with ONLY the title, no quotes or punctuation.")
        .user(format!("First message: {}", &prompt[..prompt.len().min(200)]))
        .build();

    let result = match assignment.provider.as_str() {
        "anthropic" => {
            let model = crate::agent::config::build_anthropic_model_pub(&assignment.model, &provider_config).ok()?;
            generate_with_model(model, messages).await
        }
        "openai" => {
            let model = crate::agent::config::build_openai_model_pub(&assignment.model, &provider_config).ok()?;
            generate_with_model(model, messages).await
        }
        "google" => {
            let model = crate::agent::config::build_google_model_pub(&assignment.model, &provider_config).ok()?;
            generate_with_model(model, messages).await
        }
        _ => None,
    };

    result.map(|s| {
        let s = s.trim().to_string();
        if s.len() > 80 { s[..80].to_string() } else { s }
    })
}

async fn generate_with_model<M>(model: M, messages: aisdk::core::messages::Messages) -> Option<String>
where
    M: aisdk::core::LanguageModel
        + aisdk::core::capabilities::TextInputSupport
        + aisdk::core::capabilities::ToolCallSupport
        + aisdk::core::capabilities::ReasoningSupport
        + Send
        + Sync
        + 'static,
{
    use aisdk::core::LanguageModelRequest;
    use futures::StreamExt;

    let mut request = LanguageModelRequest::builder()
        .model(model)
        .messages(messages)
        .build();

    let mut response = request.stream_text().await.ok()?;
    let mut text = String::new();
    while let Some(chunk) = response.stream.next().await {
        if let aisdk::core::LanguageModelStreamChunkType::Text(t) = chunk {
            text.push_str(&t);
        }
    }
    if text.is_empty() { None } else { Some(text) }
}
