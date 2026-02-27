use std::time::Instant;

pub struct ClientSession {
    pub env_vars: Vec<(String, String)>,
    pub path: String,
    pub cwd: String,
    pub last_heartbeat: Instant,
}

impl ClientSession {
    pub fn new() -> Self {
        Self {
            env_vars: Vec::new(),
            path: String::new(),
            cwd: String::new(),
            last_heartbeat: Instant::now(),
        }
    }

    pub fn is_stale(&self, timeout_secs: u64) -> bool {
        self.last_heartbeat.elapsed().as_secs() > timeout_secs
    }
}
