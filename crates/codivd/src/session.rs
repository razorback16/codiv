use crate::agent::agent::Agent;
use crate::agent::permissions::PermissionContext;
use crate::agent::relay_manager::RelayManager;
use codiv_common::conversation::ConversationEvent;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::oneshot;

/// IPC-related state synced from the client (env, cwd, heartbeat).
pub struct IpcState {
    pub env_vars: Vec<(String, String)>,
    pub path: String,
    pub cwd: String,
    pub last_heartbeat: Instant,
}

/// Agent lifecycle state (the agent itself, its return channel, task handle,
/// and compaction bookkeeping).
pub struct AgentState {
    pub agent: Option<Agent>,
    pub agent_return_rx: Option<oneshot::Receiver<(Agent, Vec<ConversationEvent>)>>,
    pub agent_task: Option<tokio::task::JoinHandle<()>>,
    /// Set after an LLM call if input_tokens exceeds compaction threshold.
    pub needs_compaction: bool,
    /// Last known input_tokens from the most recent LLM call.
    pub last_input_tokens: usize,
}

/// Permission context for the session.
pub struct PermissionState {
    pub permission_ctx: Option<Arc<PermissionContext>>,
}

/// SQLite persistence bookkeeping.
pub struct PersistenceState {
    /// SQLite session ID (UUID v4), assigned on first AgentRequest.
    pub session_id: Option<String>,
    /// Monotonically increasing event sequence number for this session.
    pub event_seq: u32,
    /// First prompt text, set when the session is created so that background
    /// LLM name generation can be deferred until the agent completes.
    pub pending_name_gen: Option<String>,
}

/// Shell relay state.
pub struct RelayState {
    /// Shell relay manager for lease-based command execution.
    pub relay_manager: Option<Arc<RelayManager>>,
}

pub struct ClientSession {
    pub ipc: IpcState,
    pub agent_state: AgentState,
    pub permissions: PermissionState,
    pub persistence: PersistenceState,
    pub relay: RelayState,
}

impl ClientSession {
    pub fn new() -> Self {
        Self {
            ipc: IpcState {
                env_vars: Vec::new(),
                path: String::new(),
                cwd: String::new(),
                last_heartbeat: Instant::now(),
            },
            agent_state: AgentState {
                agent: None,
                agent_return_rx: None,
                agent_task: None,
                needs_compaction: false,
                last_input_tokens: 0,
            },
            permissions: PermissionState {
                permission_ctx: None,
            },
            persistence: PersistenceState {
                session_id: None,
                event_seq: 0,
                pending_name_gen: None,
            },
            relay: RelayState {
                relay_manager: None,
            },
        }
    }

    pub fn is_stale(&self, timeout_secs: u64) -> bool {
        self.ipc.last_heartbeat.elapsed().as_secs() > timeout_secs
    }

    /// Allocate the next event sequence number and bump the counter.
    pub fn next_seq(&mut self) -> u32 {
        let seq = self.persistence.event_seq;
        self.persistence.event_seq += 1;
        seq
    }
}
