mod helpers;
mod lease;
mod session;
mod streaming;

use std::time::Instant;

use codiv_common::permissions::PermissionMode;

use crate::ipc::messages as ipc_messages;
use crate::markdown::MarkdownStream;
use crate::ui::blocks::BlockRegistry;
use crate::ui::theme::Theme;

use super::state::{PendingConfirmation, TerminalState, TokenUsage};
use super::utils::finalize_thinking;

// Re-export the public API so callers (event_loop.rs) don't change.
pub(crate) use lease::{handle_lease_message, promote_pending_lease};

/// Mutable state used by the daemon message handler.
/// Constructed from `TerminalState` sub-structs for live messages,
/// or from replay-local variables during `SessionReplay`.
pub(super) struct DaemonStreamState<'a> {
    pub md_stream: &'a mut MarkdownStream,
    pub agent_streaming: &'a mut bool,
    pub last_daemon_timestamp: &'a mut u64,
    pub model_alias: &'a mut String,
    pub token_usage: &'a mut TokenUsage,
    pub tracker: &'a mut BlockRegistry,
    pub ai_start_scrollback: &'a mut Option<u64>,
    pub ai_rendered_lines: &'a mut Vec<String>,
    pub thinking_buffer: &'a mut String,
    pub thinking_start: &'a mut Option<Instant>,
    pub thinking_scrollback: &'a mut Option<u64>,
    pub pending_confirmation: &'a mut Option<PendingConfirmation>,
    pub permission_mode: &'a mut PermissionMode,
    pub last_permission_outcome: &'a mut Option<(String, bool, String)>,
    pub session_id: &'a mut Option<String>,
    pub session_name: &'a mut Option<String>,
    pub active_request_id: &'a mut Option<String>,
    pub pending_compaction_count: &'a mut Option<usize>,
}

/// Finalize an in-progress thinking block via the shared utility.
fn finalize_thinking_ds(
    parser: &mut vt100::Parser,
    ds: &mut DaemonStreamState,
    theme: &Theme,
) -> bool {
    finalize_thinking(
        parser,
        ds.thinking_start,
        ds.thinking_buffer,
        ds.thinking_scrollback,
        ds.tracker,
        theme.ansi_thinking,
    )
}

pub(crate) fn handle_daemon_message(
    msg: ipc_messages::DaemonMessage,
    parser: &mut vt100::Parser,
    state: &mut TerminalState,
    md_stream_width: u16,
    theme: &Theme,
) {
    // Note: lease protocol messages are no-ops here (handled by handle_lease_message).
    match msg {
        ipc_messages::DaemonMessage::SessionList { sessions } => {
            session::handle_session_list(sessions, parser, state, theme);
        }
        ipc_messages::DaemonMessage::SessionReplay { events } => {
            session::handle_session_replay(events, parser, state, md_stream_width, theme);
        }
        // Lease protocol messages are handled by handle_lease_message (called
        // before this function in the event loop).
        ipc_messages::DaemonMessage::AcquireShellLease { .. }
        | ipc_messages::DaemonMessage::ExecuteLeasedCommand { .. }
        | ipc_messages::DaemonMessage::ReleaseShellLease { .. }
        | ipc_messages::DaemonMessage::CancelLeasedCommand { .. } => {}
        ipc_messages::DaemonMessage::Notice { message } => {
            state.notice_hint = Some((message, Instant::now()));
            state.ui.needs_render = true;
        }
        ipc_messages::DaemonMessage::CompactionStarted {
            request_id,
            compacted_event_count,
        } => {
            session::handle_compaction_started(request_id, compacted_event_count, parser, state, theme);
        }
        ipc_messages::DaemonMessage::CompactionComplete {
            summary,
            compacted_event_count,
        } => {
            session::handle_compaction_complete(summary, compacted_event_count, parser, state, theme);
        }
        // All other messages delegate to the core handler
        other => {
            let mut ds = DaemonStreamState {
                md_stream: &mut state.stream.md_stream,
                agent_streaming: &mut state.stream.agent_streaming,
                last_daemon_timestamp: &mut state.last_daemon_timestamp,
                model_alias: &mut state.model_alias,
                token_usage: &mut state.token_usage,
                tracker: &mut state.tracker,
                ai_start_scrollback: &mut state.stream.ai_start_scrollback,
                ai_rendered_lines: &mut state.stream.ai_rendered_lines,
                thinking_buffer: &mut state.stream.thinking_buffer,
                thinking_start: &mut state.stream.thinking_start,
                thinking_scrollback: &mut state.stream.thinking_scrollback,
                pending_confirmation: &mut state.modal.pending_confirmation,
                permission_mode: &mut state.permission_mode,
                last_permission_outcome: &mut state.last_permission_outcome,
                session_id: &mut state.session_id,
                session_name: &mut state.session_name,
                active_request_id: &mut state.stream.active_request_id,
                pending_compaction_count: &mut state.pending_compaction_count,
            };
            streaming::handle_single_message(other, parser, &mut ds, theme);
        }
    }
}
