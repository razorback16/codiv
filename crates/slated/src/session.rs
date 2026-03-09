use crate::agent::agent::Agent;
use crate::agent::permissions::PermissionContext;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::oneshot;

pub struct ClientSession {
    pub env_vars: Vec<(String, String)>,
    pub path: String,
    pub cwd: String,
    pub last_heartbeat: Instant,
    pub agent: Option<Agent>,
    pub agent_return_rx: Option<oneshot::Receiver<Agent>>,
    pub permission_ctx: Option<Arc<PermissionContext>>,
}

impl ClientSession {
    pub fn new() -> Self {
        Self {
            env_vars: Vec::new(),
            path: String::new(),
            cwd: String::new(),
            last_heartbeat: Instant::now(),
            agent: None,
            agent_return_rx: None,
            permission_ctx: None,
        }
    }

    pub fn is_stale(&self, timeout_secs: u64) -> bool {
        self.last_heartbeat.elapsed().as_secs() > timeout_secs
    }

}
