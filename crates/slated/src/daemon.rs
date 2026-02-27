use crate::ipc::server::{ClientId, IpcServer};
use crate::session::ClientSession;
use slate_common::messages::{ClientMessage, DaemonMessage};
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::info;

pub struct Daemon {
    ipc: IpcServer,
    sessions: HashMap<ClientId, ClientSession>,
}

impl Daemon {
    pub async fn new() -> std::io::Result<Self> {
        let ipc = IpcServer::new().await?;
        Ok(Self {
            ipc,
            sessions: HashMap::new(),
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
                _ = cleanup_interval.tick() => {
                    self.collect_returned_agents();
                    self.cleanup_stale_sessions().await;
                }
            }
        }
    }

    async fn dispatch(&mut self, client_id: ClientId, msg: ClientMessage) {
        match msg {
            ClientMessage::AgentRequest {
                prompt,
                request_id,
                context: _,
            } => {
                use crate::agent;

                let model_catalog = agent::config::ModelCatalog::load();
                let assignment =
                    model_catalog.assignment_for(&slate_common::types::AgentRole::Engineer);

                if let Some(client_tx) = self.ipc.client_sender(client_id) {
                    let rid = request_id.clone();

                    // Take the agent out of the session so we can move it into the task.
                    // If none exists yet, create one.
                    let session = self.sessions.get_mut(&client_id);
                    let mut agent = session
                        .and_then(|s| s.agent.take())
                        .unwrap_or_else(|| {
                            agent::agent::Agent::new(
                                slate_common::types::AgentRole::Engineer,
                                assignment,
                                "You are a helpful coding assistant. Answer concisely.".to_string(),
                            )
                        });

                    agent.add_user_message(&prompt);

                    // We need to put the agent back after the spawn completes.
                    // Use a channel to return it.
                    let (agent_return_tx, agent_return_rx) =
                        tokio::sync::oneshot::channel::<agent::agent::Agent>();

                    tokio::spawn(async move {
                        match agent.run_streaming(&rid, &client_tx).await {
                            Ok(response) => {
                                agent.add_assistant_message(&response);
                                let msg = DaemonMessage::AgentComplete {
                                    request_id: rid,
                                    summary: response,
                                };
                                if let Ok(frame) = slate_common::messages::frame_message(&msg) {
                                    let _ = client_tx.send(frame).await;
                                }
                            }
                            Err(e) => {
                                let msg = DaemonMessage::Error {
                                    request_id: rid,
                                    message: e,
                                };
                                if let Ok(frame) = slate_common::messages::frame_message(&msg) {
                                    let _ = client_tx.send(frame).await;
                                }
                            }
                        }
                        let _ = agent_return_tx.send(agent);
                    });

                    // Spawn a task to put the agent back into the session.
                    // We can't await here since dispatch is sync w.r.t. the event loop.
                    // Instead, we'll check the return channel in the next iteration.
                    // For simplicity, store the receiver on the session.
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

            ClientMessage::Confirmation { .. } => {
                // Phase 2 step 2: safety confirmation (Task 7+)
            }

            ClientMessage::CommandResult { .. } => {
                // Will be implemented in Task 5
            }
        }
    }

    fn collect_returned_agents(&mut self) {
        for session in self.sessions.values_mut() {
            if let Some(ref mut rx) = session.agent_return_rx {
                match rx.try_recv() {
                    Ok(agent) => {
                        session.agent = Some(agent);
                        session.agent_return_rx = None;
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                        session.agent_return_rx = None;
                    }
                }
            }
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
