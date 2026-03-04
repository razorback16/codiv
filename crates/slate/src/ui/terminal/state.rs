use std::time::Instant;

/// Tracks a command that has been submitted to bash but hasn't completed yet.
pub(crate) struct PendingCommand {
    pub(crate) sentinel: String,
    pub(crate) accumulated: String,
    pub(crate) command: String,
    pub(crate) last_activity: Instant,
}

/// Default scrollback limit (number of lines retained).
pub(crate) const MAX_SCROLLBACK: usize = 10_000;
