use std::time::Instant;

use crate::markdown::MarkdownStream;
use crate::shell::bash_coprocess::GitInfo;
use crate::shell::completion_engine::CompletionEngine;
use crate::ui::blocks::{BlockRegistry, InputMode};
use crate::ui::completion_popup::CompletionPopup;
use crate::ui::input::InputLine;
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
    /// If set, this command was triggered by the AI agent and the result should
    /// be sent back as `CommandExecutionResult` instead of `CommandResult`.
    pub(crate) ai_execution_id: Option<String>,
}

/// An AI-requested command waiting to be executed in the coprocess.
pub(crate) struct PendingAiExecution {
    pub(crate) execution_id: String,
    pub(crate) command: String,
    pub(crate) _timeout_ms: u64,
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

/// Tracks cumulative token usage across a session.
#[derive(Default, Clone)]
pub(crate) struct TokenUsage {
    /// Latest request's input+output (= used context window).
    pub context_used: usize,
    pub context_window: usize,
    /// Cumulative session totals for cost tracking.
    pub session_input: usize,
    pub session_output: usize,
    pub session_cache_read: usize,
}

impl TokenUsage {
    pub fn record_request(&mut self, input: usize, output: usize, cache_read: usize, ctx_win: usize) {
        self.context_used = input + output;
        self.context_window = ctx_win;
        self.session_input += input;
        self.session_output += output;
        self.session_cache_read += cache_read;
    }
    pub fn reset(&mut self) { *self = Self::default(); }
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
    pub active_request_id: Option<String>,
    pub ai_start_scrollback: Option<u64>,
    pub thinking_buffer: String,
    pub thinking_start: Option<Instant>,
    pub thinking_scrollback: Option<u64>,
    pub model_alias: String,
    pub token_usage: TokenUsage,
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

    // AI-requested command execution queue
    pub pending_ai_executions: std::collections::VecDeque<PendingAiExecution>,

    /// Accumulates PTY bytes during command execution for CmdResponseBlock replay.
    pub cmd_output_capture: Vec<u8>,

    /// Accumulates ANSI-rendered lines during AI streaming for AiResponseBlock.
    pub ai_rendered_lines: Vec<String>,

    // Contextual hint (shown for 5s after trigger)
    pub hint_shown_at: Option<Instant>,
    pub last_mouse_drag: Option<Instant>,
    pub term_supports_option_select: bool,
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
            active_request_id: None,
            ai_start_scrollback: None,
            thinking_buffer: String::new(),
            thinking_start: None,
            thinking_scrollback: None,
            model_alias: String::new(),
            token_usage: TokenUsage::default(),
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

            // AI-requested command execution queue
            pending_ai_executions: std::collections::VecDeque::new(),

            cmd_output_capture: Vec::new(),
            ai_rendered_lines: Vec::new(),

            // Contextual hint (shown for 5s after trigger)
            hint_shown_at: None,
            last_mouse_drag: None,
            term_supports_option_select: {
                let tp = std::env::var("TERM_PROGRAM").unwrap_or_default();
                let lc = std::env::var("LC_TERMINAL").unwrap_or_default();
                matches!(
                    tp.as_str(),
                    "iTerm.app" | "Apple_Terminal" | "WezTerm" | "Alacritty" | "kitty"
                ) || matches!(lc.as_str(), "iTerm2")
            },
        }
    }
}
