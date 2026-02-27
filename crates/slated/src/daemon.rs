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
                    self.dispatch(msg.client_id, msg.message).await;
                }
                _ = cleanup_interval.tick() => {
                    self.cleanup_stale_sessions().await;
                }
            }
        }
    }

    async fn dispatch(&mut self, client_id: ClientId, msg: ClientMessage) {
        match msg {
            ClientMessage::AgentRequest {
                prompt: _,
                request_id,
                context: _,
            } => {
                // Phase 2 step 2: agent integration (Task 6+)
                self.ipc
                    .send(
                        client_id,
                        &DaemonMessage::Error {
                            request_id,
                            message: "agent not yet implemented".to_string(),
                        },
                    )
                    .await;
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
