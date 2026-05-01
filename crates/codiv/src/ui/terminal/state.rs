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

/// A pending shell lease request from the daemon.
pub(crate) struct PendingLease {
    pub(crate) lease_id: String,
    pub(crate) request_id: String,
}

/// An active shell lease held by the daemon.
pub(crate) struct ActiveLease {
    pub(crate) lease_id: String,
    #[allow(dead_code)]
    pub(crate) request_id: String,
    /// The command to execute within this lease (set by ExecuteLeasedCommand).
    pub(crate) current_command: Option<ActiveLeasedCommand>,
}

/// A command queued for execution within an active lease.
pub(crate) struct ActiveLeasedCommand {
    pub(crate) execution_id: String,
    pub(crate) command: String,
    pub(crate) _timeout_ms: u64,
}

/// State for the shell lease protocol on the client side.
pub(crate) struct ShellRelayState {
    pub(crate) active_lease: Option<ActiveLease>,
    pub(crate) pending_leases: std::collections::VecDeque<PendingLease>,
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

// ---------------------------------------------------------------------------
// Sub-state structs
// ---------------------------------------------------------------------------

/// Command execution state.
pub(crate) struct CommandState {
    pub(crate) pending_command: Option<PendingCommand>,
    pub(crate) cmd_start_scrollback: Option<u64>,
    /// Accumulates PTY bytes during command execution for CmdResponseBlock replay.
    pub(crate) cmd_output_capture: Vec<u8>,
}

/// AI streaming state.
pub(crate) struct AgentStreamState {
    pub(crate) agent_streaming: bool,
    pub(crate) active_request_id: Option<String>,
    pub(crate) ai_start_scrollback: Option<u64>,
    pub(crate) thinking_buffer: String,
    pub(crate) thinking_start: Option<Instant>,
    pub(crate) thinking_scrollback: Option<u64>,
    pub(crate) md_stream: MarkdownStream,
    /// Accumulates ANSI-rendered lines during AI streaming for AiResponseBlock.
    pub(crate) ai_rendered_lines: Vec<String>,
}

/// Shell and environment state.
pub(crate) struct ShellState {
    pub(crate) cwd: String,
    pub(crate) git_info: Option<GitInfo>,
    pub(crate) cached_env_vars: Vec<(String, String)>,
    pub(crate) shell_relay: ShellRelayState,
}

/// UI rendering state.
pub(crate) struct UiState {
    pub(crate) scroll_offset: usize,
    pub(crate) prompt_is_live: bool,
    pub(crate) prompt_anchor_row: Option<u16>,
    pub(crate) needs_render: bool,
    pub(crate) anim: AnimationState,
}

/// Modal dialog state.
pub(crate) struct ModalState {
    pub(crate) pending_confirmation: Option<PendingConfirmation>,
    pub(crate) pending_session_picker: Option<PendingSessionPicker>,
}

// ---------------------------------------------------------------------------
// Additional sub-state structs
// ---------------------------------------------------------------------------

/// AI model metadata state.
pub(crate) struct ModelState {
    pub alias: String,
    pub token_usage: TokenUsage,
}

/// User input state.
pub(crate) struct InputState {
    pub line: InputLine,
    pub mode: InputMode,
    pub completion_engine: CompletionEngine,
    pub completion_popup: CompletionPopup,
}

/// Session tracking state.
pub(crate) struct SessionState {
    pub id: Option<String>,
    pub name: Option<String>,
    pub last_permission_outcome: Option<(String, bool, String)>,
}

/// Runtime settings and modes.
pub(crate) struct SettingsState {
    pub thinking_enabled: bool,
    pub permission_mode: PermissionMode,
}

/// Contextual hint state.
pub(crate) struct HintState {
    pub hint_shown_at: Option<Instant>,
    pub last_mouse_drag: Option<Instant>,
    pub term_supports_option_select: bool,
    pub hint_seed: u32,
    pub notice_hint: Option<(String, Instant)>,
}

// ---------------------------------------------------------------------------
// TerminalState
// ---------------------------------------------------------------------------

/// Consolidated UI state for the terminal event loop.
#[allow(dead_code)]
pub(crate) struct TerminalState {
    // Timing
    pub last_heartbeat_sent: Instant,
    pub last_reconnect_attempt: Instant,
    pub last_daemon_timestamp: u64,

    // Sub-state groups
    pub cmd: CommandState,
    pub stream: AgentStreamState,
    pub shell: ShellState,
    pub ui: UiState,
    pub modal: ModalState,
    pub model: ModelState,
    pub input: InputState,
    pub session: SessionState,
    pub settings: SettingsState,
    pub hints: HintState,

    // Block registry and tool result modal
    pub tracker: BlockRegistry,
    pub tool_result_modal: ToolResultModal,
    pub was_alt_screen: bool,

    /// When set, a compaction is in progress; holds the compacted event count.
    pub pending_compaction_count: Option<usize>,
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

            // Sub-state groups
            cmd: CommandState {
                pending_command: None,
                cmd_start_scrollback: None,
                cmd_output_capture: Vec::new(),
            },
            stream: AgentStreamState {
                agent_streaming: false,
                active_request_id: None,
                ai_start_scrollback: None,
                thinking_buffer: String::new(),
                thinking_start: None,
                thinking_scrollback: None,
                md_stream: MarkdownStream::new(md_stream_width, theme),
                ai_rendered_lines: Vec::new(),
            },
            shell: ShellState {
                cwd: initial_cwd,
                git_info: None,
                cached_env_vars: Vec::new(),
                shell_relay: ShellRelayState {
                    active_lease: None,
                    pending_leases: std::collections::VecDeque::new(),
                },
            },
            ui: UiState {
                scroll_offset: 0,
                prompt_is_live: false,
                prompt_anchor_row: None,
                needs_render: true,
                anim: AnimationState::new(),
            },
            modal: ModalState {
                pending_confirmation: None,
                pending_session_picker: None,
            },
            model: ModelState {
                alias: String::new(),
                token_usage: TokenUsage::default(),
            },
            input: InputState {
                line: InputLine::new(),
                mode: if has_client {
                    InputMode::Ai
                } else {
                    InputMode::Command
                },
                completion_engine: CompletionEngine::new(),
                completion_popup: CompletionPopup::new(),
            },
            session: SessionState {
                id: None,
                name: None,
                last_permission_outcome: None,
            },
            settings: SettingsState {
                thinking_enabled: false,
                permission_mode: PermissionMode::default(),
            },
            hints: HintState {
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
                hint_seed: 0,
                notice_hint: None,
            },

            // Block registry and tool result modal
            tracker: {
                let mut t = BlockRegistry::new();
                t.set_width(md_stream_width);
                t
            },
            tool_result_modal: ToolResultModal::new(),
            was_alt_screen: false,

            pending_compaction_count: None,
        }
    }
}
