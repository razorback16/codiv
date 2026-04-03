use crate::ipc::server::{ClientId, IpcServer};
use crate::session::ClientSession;
use crate::store::SessionStore;
use codiv_common::conversation::ConversationEvent;
use codiv_common::messages::{ClientMessage, DaemonMessage};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::info;

const SYSTEM_PROMPT: &str = "You are a helpful coding assistant embedded in a terminal. \
    You can see the user's recent terminal commands and their output in the conversation history. \
    Use this context to give relevant, concise answers. \
    When referencing files or directories, use paths relative to the user's current working directory when possible.";

pub(crate) fn create_agent(
    cwd: String,
    config: &crate::agent::config::AppConfig,
) -> crate::agent::agent::Agent {
    let assignment = config
        .models
        .assignment_for(&codiv_common::types::AgentRole::Engineer);
    let provider_config = config.models.resolve_provider_config(&assignment);
    crate::agent::agent::Agent::new(
        codiv_common::types::AgentRole::Engineer,
        assignment,
        provider_config,
        SYSTEM_PROMPT.to_string(),
        cwd,
    )
}

/// Generate a short initial name from the first user prompt (truncated to
/// 60 chars on a word boundary).
pub(crate) fn truncated_name(prompt: &str) -> String {
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
    pub(crate) ipc: IpcServer,
    pub(crate) sessions: HashMap<ClientId, ClientSession>,
    pub(crate) store: SessionStore,
    /// Notifies the event loop when a spawned agent task completes so
    /// `collect_returned_agents` runs immediately (not on next IPC message).
    pub(crate) agent_done_tx: tokio::sync::mpsc::Sender<()>,
    agent_done_rx: tokio::sync::mpsc::Receiver<()>,
    /// Channel for background name-gen tasks to send results back to the
    /// event loop so the name can be persisted to SQLite.
    name_update_tx: tokio::sync::mpsc::Sender<(String, String, tokio::sync::mpsc::Sender<Vec<u8>>)>,
    name_update_rx:
        tokio::sync::mpsc::Receiver<(String, String, tokio::sync::mpsc::Sender<Vec<u8>>)>,
    pub(crate) config: Arc<RwLock<crate::agent::config::AppConfig>>,
    _config_watcher: crate::agent::config::ConfigWatcherGuard,
    config_change_rx: tokio::sync::mpsc::Receiver<()>,
    _token_refresh_task: tokio::task::JoinHandle<()>,
}

impl Daemon {
    pub async fn new() -> std::io::Result<Self> {
        // Eagerly load config so the default config.toml is created on
        // first startup (before any client connects).
        let initial_config = crate::agent::config::AppConfig::load();
        let (config_change_tx, config_change_rx) = tokio::sync::mpsc::channel(4);
        let (config, _config_watcher) = initial_config.into_watched(Some(config_change_tx));

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
            config,
            _config_watcher,
            config_change_rx,
            _token_refresh_task: crate::agent::config::spawn_token_refresh_task(),
        })
    }

    pub async fn run(mut self) {
        let mut cleanup_interval = tokio::time::interval(Duration::from_secs(30));
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to register SIGTERM handler");

        loop {
            tokio::select! {
                _ = sigterm.recv() => {
                    info!("received SIGTERM, shutting down");
                    break;
                }
                _ = tokio::signal::ctrl_c() => {
                    info!("received SIGINT, shutting down");
                    break;
                }
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
                    if let Ok(frame) = codiv_common::messages::frame_message(&msg) {
                        let _ = client_tx.send(frame).await;
                    }
                }
                _ = cleanup_interval.tick() => {
                    self.collect_returned_agents();
                    crate::handlers::session::cleanup_stale_sessions(&mut self).await;
                }
                Some(_) = self.config_change_rx.recv() => {
                    // Notify all connected clients
                    for (&cid, _) in &self.sessions {
                        self.ipc.send(cid, &DaemonMessage::Notice {
                            message: "Configuration reloaded".to_string(),
                        }).await;
                    }
                    // Sync permission mode and invalidate cached agents so the
                    // next request picks up the new provider/model config.
                    if let Ok(cfg) = self.config.read() {
                        for (_, session) in self.sessions.iter_mut() {
                            if let Some(ref pctx) = session.permissions.permission_ctx {
                                pctx.set_mode(cfg.permissions.mode);
                            }
                            // Drop the cached agent so a fresh one is created
                            // with the updated config on the next request.
                            if session.agent_state.agent_task.is_none() {
                                session.agent_state.agent = None;
                            }
                        }
                    }
                }
            }
        }
    }

    // -------------------------------------------------------------------
    // Dispatch: thin router that delegates to handler modules
    // -------------------------------------------------------------------

    async fn dispatch(&mut self, client_id: ClientId, msg: ClientMessage) {
        match msg {
            ClientMessage::AgentRequest {
                prompt,
                request_id,
                context: _,
                thinking,
            } => {
                crate::handlers::agent::handle_agent_request(
                    self, client_id, prompt, request_id, thinking,
                )
                .await;
            }

            ClientMessage::EnvSnapshot {
                env_vars,
                path,
                cwd,
            } => {
                if let Some(session) = self.sessions.get_mut(&client_id) {
                    session.ipc.env_vars = env_vars;
                    session.ipc.path = path;
                    session.ipc.cwd = cwd;
                    info!("received env snapshot from client {}", client_id);
                }
            }

            ClientMessage::Heartbeat { timestamp: _ } => {
                if let Some(session) = self.sessions.get_mut(&client_id) {
                    session.ipc.last_heartbeat = std::time::Instant::now();
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

            ClientMessage::Confirmation {
                request_id,
                approved,
                add_to_allowlist,
                add_to_denylist,
                comment,
            } => {
                crate::handlers::permission::handle_confirmation(
                    self,
                    client_id,
                    request_id,
                    approved,
                    add_to_allowlist,
                    add_to_denylist,
                    comment,
                )
                .await;
            }

            ClientMessage::SetPermissionMode { mode } => {
                crate::handlers::permission::handle_set_permission_mode(self, client_id, mode)
                    .await;
            }

            ClientMessage::ListSessions => {
                crate::handlers::session::handle_list_sessions(self, client_id).await;
            }

            ClientMessage::NewSession => {
                crate::handlers::session::handle_new_session(self, client_id).await;
            }

            ClientMessage::LoadSession {
                session_id: target_sid,
            } => {
                crate::handlers::session::handle_load_session(self, client_id, target_sid).await;
            }

            ClientMessage::CommandResult {
                command,
                output,
                exit_code,
                cwd,
            } => {
                crate::handlers::session::handle_command_result(
                    self, client_id, command, output, exit_code, cwd,
                )
                .await;
            }

            // Shell lease protocol messages — route to RelayManager
            msg @ (ClientMessage::ShellLeaseAcquired { .. }
            | ClientMessage::ShellLeaseQueued { .. }
            | ClientMessage::CommandStarted { .. }
            | ClientMessage::CommandCompleted { .. }
            | ClientMessage::CommandFailed { .. }
            | ClientMessage::CommandCancelled { .. }
            | ClientMessage::ShellLeaseReleased { .. }) => {
                crate::handlers::shell::handle_shell_lease_message(self, client_id, msg).await;
            }

            ClientMessage::CompactRequest => {
                crate::handlers::compaction::handle_compact_request(self, client_id).await;
            }

            ClientMessage::CancelRequest { request_id } => {
                crate::handlers::agent::handle_cancel_request(self, client_id, request_id).await;
            }
        }
    }

    // -------------------------------------------------------------------
    // Persistence helpers (used by handlers via &mut Daemon)
    // -------------------------------------------------------------------

    /// Ensure the client has a SQLite session. Returns the session_id.
    pub(crate) fn ensure_session(&mut self, client_id: ClientId) -> Option<String> {
        let session = self.sessions.get_mut(&client_id)?;
        if let Some(ref id) = session.persistence.session_id {
            return Some(id.clone());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let cwd = session.ipc.cwd.clone();
        if let Err(e) = self.store.create_session(&id, &cwd) {
            tracing::error!("failed to create session in sqlite: {}", e);
            return None;
        }
        session.persistence.session_id = Some(id.clone());
        Some(id)
    }

    /// Persist a single event and bump the session's seq counter.
    pub(crate) fn persist_event(&mut self, client_id: ClientId, event: &ConversationEvent) {
        let session = match self.sessions.get_mut(&client_id) {
            Some(s) => s,
            None => return,
        };
        if session.persistence.session_id.is_none() {
            return;
        }
        let seq = session.next_seq();
        let sid = session.persistence.session_id.as_deref().unwrap();
        if let Err(e) = self.store.append_event(sid, seq, event) {
            tracing::error!("failed to persist event seq={}: {}", seq, e);
        }
    }

    /// Persist multiple events and bump the session's seq counter.
    pub(crate) fn persist_events(&mut self, client_id: ClientId, events: &[ConversationEvent]) {
        for event in events {
            self.persist_event(client_id, event);
        }
    }

    // -------------------------------------------------------------------
    // Agent lifecycle
    // -------------------------------------------------------------------

    fn collect_returned_agents(&mut self) {
        // Collect completed agents from all sessions. We need to batch
        // the persistence calls because we can't borrow self mutably while
        // iterating, so collect the data first.
        let mut to_persist: Vec<(ClientId, Vec<ConversationEvent>)> = Vec::new();
        // Collect deferred name-gen requests: (session_id, prompt, client_tx)
        let mut name_gen_requests: Vec<(
            String,
            String,
            tokio::sync::mpsc::Sender<Vec<u8>>,
        )> = Vec::new();

        for (&cid, session) in self.sessions.iter_mut() {
            if let Some(ref mut rx) = session.agent_state.agent_return_rx {
                match rx.try_recv() {
                    Ok((agent, events)) => {
                        session.agent_state.agent = Some(agent);
                        session.agent_state.agent_return_rx = None;
                        session.agent_state.agent_task = None;
                        if !events.is_empty() {
                            to_persist.push((cid, events));
                        }
                        // If this session has a pending name generation, fire it
                        // now that the main agent task is done.
                        if let Some(prompt) = session.persistence.pending_name_gen.take() {
                            if let Some(ref sid) = session.persistence.session_id {
                                if let Some(tx) = self.ipc.client_sender(cid) {
                                    name_gen_requests.push((sid.clone(), prompt, tx));
                                }
                            }
                        }
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                        session.agent_state.agent_return_rx = None;
                    }
                }
            }
        }

        // Now persist collected events and check compaction threshold
        for (cid, events) in to_persist {
            // Check for token usage to determine if compaction is needed
            if let Some(input_tokens) = events.iter().find_map(|e| match e {
                ConversationEvent::TokenUsage { input_tokens, .. } => Some(*input_tokens),
                _ => None,
            }) {
                if let Some(session) = self.sessions.get_mut(&cid) {
                    session.agent_state.last_input_tokens = input_tokens;
                    let cfg = self.config.read().unwrap();
                    if let Some(ref agent) = session.agent_state.agent {
                        let threshold = agent.model_config.context_window()
                            * cfg.compaction.threshold_percent as usize
                            / 100;
                        if input_tokens > threshold {
                            session.agent_state.needs_compaction = true;
                            info!(
                                "compaction needed: {} tokens > {} threshold ({}%)",
                                input_tokens, threshold, cfg.compaction.threshold_percent
                            );
                        }
                    }
                }
            }
            self.persist_events(cid, &events);
        }

        // Spawn deferred background name generation tasks
        if !name_gen_requests.is_empty() {
            info!(
                "spawning {} deferred name generation task(s)",
                name_gen_requests.len()
            );
        }
        for (sid, prompt, client_tx) in name_gen_requests {
            let name_update_tx = self.name_update_tx.clone();
            let models = self.config.read().unwrap().models.clone();
            tokio::spawn(async move {
                // Use a timeout so name generation never blocks the inference
                // server for too long (important for local models with single-
                // request concurrency).
                match tokio::time::timeout(
                    Duration::from_secs(30),
                    generate_session_name(&prompt, &models),
                )
                .await
                {
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
}

/// Attempt to generate a short session title using a cheap LLM call.
/// Returns `None` on failure (network error, no API key, etc.).
async fn generate_session_name(
    prompt: &str,
    models: &crate::agent::config::ModelCatalog,
) -> Option<String> {
    // Only use "session_namer" role — don't fall back to other roles
    // which may point to expensive models. If not configured, the
    // truncated prompt name is good enough.
    let assignment = match models.roles.get("session_namer").cloned() {
        Some(a) => a,
        None => {
            info!("generate_session_name: no session_namer role configured, skipping");
            return None;
        }
    };
    let provider_config = models.resolve_provider_config(&assignment);

    info!(
        "generate_session_name: using {}/{}",
        assignment.provider, assignment.model
    );

    let result = crate::agent::config::simple_text_completion(
        &assignment,
        &provider_config,
        "Summarize the following user request in 2-4 words on a single line. Reply with ONLY the summary, no quotes or punctuation.",
        prompt,
    )
    .await
    .ok();

    result.map(|s| {
        let s = s.trim().to_string();
        if s.len() > 80 {
            s[..80].to_string()
        } else {
            s
        }
    })
}
