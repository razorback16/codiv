use std::time::Instant;

/// Tracks a command that has been submitted to bash but hasn't completed yet.
pub(crate) struct PendingCommand {
    pub(crate) sentinel: String,
    pub(crate) accumulated: String,
    pub(crate) command: String,
    pub(crate) last_activity: Instant,
}

/// Tracks a pending permission confirmation prompt from the daemon.
#[allow(dead_code)]
pub(crate) struct PendingConfirmation {
    pub(crate) request_id: String,
    pub(crate) description: String,
    pub(crate) risk: slate_common::messages::RiskLevel,
    pub(crate) tool_name: String,
    pub(crate) tool_args: String,
    pub(crate) prompt_lines: u16,
    pub(crate) selected_index: usize,
    pub(crate) option_count: usize,
}

/// Tracks a pending session picker (inline list of saved sessions).
pub(crate) struct PendingSessionPicker {
    pub(crate) sessions: Vec<slate_common::conversation::SessionInfo>,
    pub(crate) selected_index: usize,
    pub(crate) prompt_lines: u16,
}

/// Default scrollback limit (number of lines retained).
pub(crate) const MAX_SCROLLBACK: usize = 10_000;
