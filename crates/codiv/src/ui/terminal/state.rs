use std::time::Instant;

use crate::markdown::MarkdownStream;
use crate::shell::bash_coprocess::GitInfo;
use crate::shell::completion_engine::CompletionEngine;
use crate::ui::blocks::{BlockRegistry, InputMode};
use crate::ui::completion_popup::CompletionPopup;
use crate::ui::input::InputLine;
use crate::ui::selection::TextSelection;
use crate::ui::theme::Theme;
use crate::ui::tool_modal::ToolResultModal;
use codiv_common::permissions::PermissionMode;

use super::animation::AnimationState;

/// Tracks a command that has been submitted to bash but hasn't completed yet.
pub(crate) struct PendingCommand {
    pub(crate) sentinel: String,
    pub(crate) accumulated: String,
    pub(crate) command: String,
    pub(crate) last_activity: Instant,
    pub(crate) needs_env_refresh: bool,
}

const ENV_MODIFIERS: &[&str] = &[
    "cd", "source", ".", "pushd", "popd", "export", "unset",
    "eval", "direnv", "nvm", "pyenv", "rbenv", "conda", "activate",
];

pub(crate) fn command_modifies_env(cmd: &str) -> bool {
    for segment in cmd.split([';', '|', '&']) {
        let first_word = segment.split_whitespace().next().unwrap_or("");
        let base = first_word.rsplit('/').next().unwrap_or(first_word);
        if ENV_MODIFIERS.contains(&base) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_cd() {
        assert!(command_modifies_env("cd /tmp"));
        assert!(command_modifies_env("cd"));
    }

    #[test]
    fn detects_source() {
        assert!(command_modifies_env("source ~/.bashrc"));
        assert!(command_modifies_env(". ~/.bashrc"));
    }

    #[test]
    fn detects_export() {
        assert!(command_modifies_env("export FOO=bar"));
    }

    #[test]
    fn detects_in_compound() {
        assert!(command_modifies_env("echo hi; cd /tmp"));
        assert!(command_modifies_env("make && cd build"));
    }

    #[test]
    fn normal_commands_false() {
        assert!(!command_modifies_env("ls -la"));
        assert!(!command_modifies_env("cargo build"));
        assert!(!command_modifies_env("git status"));
    }
}

/// Tracks a pending permission confirmation prompt from the daemon.
#[allow(dead_code)]
pub(crate) struct PendingConfirmation {
    pub(crate) request_id: String,
    pub(crate) description: String,
    pub(crate) risk: codiv_common::messages::RiskLevel,
    pub(crate) tool_name: String,
    pub(crate) tool_args: String,
    pub(crate) prompt_lines: u16,
    pub(crate) selected_index: usize,
    pub(crate) option_count: usize,
}

/// Maximum number of sessions visible at once in the picker.
pub(crate) const VISIBLE_SESSIONS: usize = 5;

/// Tracks a pending session picker (inline list of saved sessions).
pub(crate) struct PendingSessionPicker {
    pub(crate) sessions: Vec<codiv_common::conversation::SessionInfo>,
    pub(crate) selected_index: usize,
    /// First visible session index in the scrollable window.
    pub(crate) viewport_offset: usize,
    pub(crate) prompt_lines: u16,
}

/// Default scrollback limit (number of lines retained).
pub(crate) const MAX_SCROLLBACK: usize = 10_000;

/// Consolidated UI state for the terminal event loop.
#[allow(dead_code)]
pub(crate) struct TerminalState {
    // Timing
    pub last_heartbeat_sent: Instant,
    pub last_reconnect_attempt: Instant,
    pub last_daemon_timestamp: u64,

    // Command execution
    pub pending_command: Option<PendingCommand>,
    pub cmd_start_scrollback: Option<u64>,

    // AI / daemon
    pub agent_streaming: bool,
    pub ai_start_scrollback: Option<u64>,
    pub thinking_buffer: String,
    pub thinking_start: Option<Instant>,
    pub thinking_scrollback: Option<u64>,
    pub model_alias: String,
    pub context_usage: (usize, usize),
    pub md_stream: MarkdownStream,

    // Input
    pub input: InputLine,
    pub input_mode: InputMode,
    pub completion_engine: CompletionEngine,
    pub completion_popup: CompletionPopup,

    // UI state
    pub scroll_offset: usize,
    pub prompt_is_live: bool,
    pub prompt_anchor_row: Option<u16>,
    pub selection: TextSelection,
    pub clipboard: Option<arboard::Clipboard>,
    pub tracker: BlockRegistry,
    pub tool_result_modal: ToolResultModal,
    pub was_alt_screen: bool,
    pub anim: AnimationState,
    pub needs_render: bool,

    // Settings / modes
    pub thinking_enabled: bool,
    pub permission_mode: PermissionMode,

    // Confirmation / session
    pub pending_confirmation: Option<PendingConfirmation>,
    pub last_permission_outcome: Option<(String, bool, String)>,
    pub session_id: Option<String>,
    pub session_name: Option<String>,
    pub pending_session_picker: Option<PendingSessionPicker>,

    // Shell state
    pub cwd: String,
    pub git_info: Option<GitInfo>,
    pub cached_env_vars: Vec<(String, String)>,
}

impl TerminalState {
    #[allow(dead_code)]
    pub fn new(
        initial_cwd: String,
        has_client: bool,
        md_stream_width: u16,
        theme: &Theme,
    ) -> Self {
        let now = Instant::now();
        Self {
            // Timing
            last_heartbeat_sent: now,
            last_reconnect_attempt: now,
            last_daemon_timestamp: 0,

            // Command execution
            pending_command: None,
            cmd_start_scrollback: None,

            // AI / daemon
            agent_streaming: false,
            ai_start_scrollback: None,
            thinking_buffer: String::new(),
            thinking_start: None,
            thinking_scrollback: None,
            model_alias: String::new(),
            context_usage: (0, 0),
            md_stream: MarkdownStream::new(md_stream_width, theme),

            // Input
            input: InputLine::new(),
            input_mode: if has_client {
                InputMode::Ai
            } else {
                InputMode::Command
            },
            completion_engine: CompletionEngine::new(),
            completion_popup: CompletionPopup::new(),

            // UI state
            scroll_offset: 0,
            prompt_is_live: false,
            prompt_anchor_row: None,
            selection: TextSelection::new(),
            clipboard: None,
            tracker: BlockRegistry::new(),
            tool_result_modal: ToolResultModal::new(),
            was_alt_screen: false,
            anim: AnimationState::new(),
            needs_render: true,

            // Settings / modes
            thinking_enabled: false,
            permission_mode: PermissionMode::default(),

            // Confirmation / session
            pending_confirmation: None,
            last_permission_outcome: None,
            session_id: None,
            session_name: None,
            pending_session_picker: None,

            // Shell state
            cwd: initial_cwd,
            git_info: None,
            cached_env_vars: Vec::new(),
        }
    }
}
