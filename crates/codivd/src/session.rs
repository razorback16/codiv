use crate::agent::agent::Agent;
use crate::agent::permissions::PermissionContext;
use crate::agent::shell_backend::RelayResult;
use codiv_common::conversation::ConversationEvent;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::oneshot;

pub struct ClientSession {
    pub env_vars: Vec<(String, String)>,
    pub path: String,
    pub cwd: String,
    pub last_heartbeat: Instant,
    pub agent: Option<Agent>,
    pub agent_return_rx: Option<oneshot::Receiver<(Agent, Vec<ConversationEvent>)>>,
    pub agent_task: Option<tokio::task::JoinHandle<()>>,
    pub permission_ctx: Option<Arc<PermissionContext>>,
    /// SQLite session ID (UUID v4), assigned on first AgentRequest.
    pub session_id: Option<String>,
    /// Monotonically increasing event sequence number for this session.
    pub event_seq: u32,
    /// First prompt text, set when the session is created so that background
    /// LLM name generation can be deferred until the agent completes.
    pub pending_name_gen: Option<String>,
    /// Pending command executions relayed to the client, keyed by execution_id.
    pub pending_executions: Option<Arc<Mutex<HashMap<String, oneshot::Sender<RelayResult>>>>>,
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
            agent_task: None,
            permission_ctx: None,
            session_id: None,
            event_seq: 0,
            pending_name_gen: None,
            pending_executions: None,
        }
    }

    pub fn is_stale(&self, timeout_secs: u64) -> bool {
        self.last_heartbeat.elapsed().as_secs() > timeout_secs
    }

    /// Allocate the next event sequence number and bump the counter.
    pub fn next_seq(&mut self) -> u32 {
        let seq = self.event_seq;
        self.event_seq += 1;
        seq
    }
}
