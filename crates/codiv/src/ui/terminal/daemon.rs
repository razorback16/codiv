use std::time::Instant;

use codiv_common::conversation::ConversationEvent;
use codiv_common::permissions::PermissionMode;

use crate::ipc::messages as ipc_messages;
use crate::markdown::MarkdownStream;
use crate::ui::blocks::{canonical_tool_name, BlockRegistry, ToolResultAction};
use crate::ui::theme::Theme;

use super::state::{PendingConfirmation, PendingSessionPicker, TerminalState, TokenUsage};
use super::utils::{finalize_thinking, get_scrollback_line, parser_push_notice, rerender_all, reset_screen, NoticeKind, write_block_lines_to_parser};

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


/// Split ANSI bytes by `\r\n` and append complete lines to the accumulator.
/// Partial lines (no trailing `\r\n`) are appended to the last entry or added as a new entry.
fn accumulate_ai_lines(acc: &mut Vec<String>, ansi: &[u8]) {
    let text = String::from_utf8_lossy(ansi);
    let mut parts: Vec<&str> = text.split("\r\n").collect();
    // If text ends with \r\n, split produces a trailing empty string — pop it.
    if text.ends_with("\r\n") && parts.last() == Some(&"") {
        parts.pop();
    }
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            // Append to the last incomplete line, or start a new one.
            if let Some(last) = acc.last_mut() {
                last.push_str(part);
            } else {
                acc.push(part.to_string());
            }
            // If there are more parts, this line is now complete — next parts start new lines.
        } else {
            acc.push(part.to_string());
        }
    }
    // If the original text ended with \r\n, the current last line is complete.
    // Push an empty string to start a new line for future appends.
    if text.ends_with("\r\n") {
        acc.push(String::new());
    }
}

/// Build the ANSI-formatted rendered lines for a tool block result.
/// Each line does NOT include trailing `\r\n` — the caller adds that.
fn build_tool_rendered_lines(
    tool_name: &str,
    header: &str,
    summary: &str,
    preview_lines: &[String],
    perm: Option<&(String, bool, String)>,
    theme: &Theme,
) -> Vec<String> {
    let mut lines = Vec::new();

    if let Some((_perm_tool, granted, reason)) = perm {
        if *granted {
            lines.push(format!("{}{}\x1b[0m", theme.ansi_tool_done, header));
            lines.push(format!("{}  \u{2514} {}\x1b[0m", theme.ansi_tool_done_suffix, reason));
            let is_tool_error = (tool_name.eq_ignore_ascii_case("bash")
                && !summary.contains("exit 0"))
                || summary.contains(" failed:");
            let color = if is_tool_error {
                theme.ansi_exit_failure
            } else {
                theme.ansi_tool_done_suffix
            };
            lines.push(format!("{}{}\x1b[0m", color, summary));
            for pline in preview_lines {
                lines.push(format!("{}{}\x1b[0m", theme.ansi_thinking, pline));
            }
        } else {
            lines.push(format!("{}{}\x1b[0m", theme.ansi_tool_denied, header));
            lines.push(format!("{}  \u{2514} {}\x1b[0m", theme.ansi_tool_denied_suffix, reason));
        }
    } else {
        lines.push(format!("{}{}\x1b[0m", theme.ansi_tool_done, header));
        let is_tool_error = (tool_name.eq_ignore_ascii_case("bash")
            && !summary.contains("exit 0"))
            || summary.contains(" failed:");
        let color = if is_tool_error {
            theme.ansi_exit_failure
        } else {
            theme.ansi_tool_done_suffix
        };
        lines.push(format!("{}{}\x1b[0m", color, summary));
        for pline in preview_lines {
            lines.push(format!("{}{}\x1b[0m", theme.ansi_thinking, pline));
        }
    }

    lines
}

/// Handle a single `DaemonMessage` — all match arms except `SessionList` and
/// `SessionReplay`, which are handled in the outer `handle_daemon_message`.
fn handle_single_message(
    msg: ipc_messages::DaemonMessage,
    parser: &mut vt100::Parser,
    ds: &mut DaemonStreamState,
    theme: &Theme,
) {
    match msg {
        ipc_messages::DaemonMessage::AgentStreamChunk {
            request_id: _,
            chunk,
        } => {
            match chunk {
                ipc_messages::StreamChunk::Text(t) => {
                    if finalize_thinking_ds(parser, ds, theme) {
                        parser.process(b"\r\n");
                    }
                    // Strip leading whitespace from the first text chunk of a response.
                    let t = if ds.ai_start_scrollback.is_none() {
                        t.trim_start().to_string()
                    } else {
                        t
                    };
                    if t.is_empty() {
                        // Entire chunk was leading whitespace — skip it.
                    } else {
                        if ds.ai_start_scrollback.is_none() {
                            *ds.ai_start_scrollback = Some(get_scrollback_line(parser));
                        }
                        if let Some(ansi) = ds.md_stream.push(&t) {
                            parser.process(&ansi);
                            accumulate_ai_lines(ds.ai_rendered_lines, &ansi);
                        }
                    }
                }
                ipc_messages::StreamChunk::Reasoning(t) => {
                    if ds.thinking_start.is_none() {
                        *ds.thinking_start = Some(Instant::now());
                        *ds.thinking_scrollback = Some(get_scrollback_line(parser));
                        // Write placeholder line
                        let thinking_line = format!("{}Thinking...\x1b[0m\r\n", theme.ansi_thinking);
                        parser.process(thinking_line.as_bytes());
                    }
                    ds.thinking_buffer.push_str(&t);
                }
                ipc_messages::StreamChunk::ToolCallDelta {
                    tool_call_id: _,
                    tool_name,
                    delta: _,
                } => {
                    // On the FIRST delta for a tool call, show spinner placeholder
                    if ds.tracker.pending_tool().is_none() {
                        let finalized = finalize_thinking_ds(parser, ds, theme);
                        // Flush any buffered markdown
                        let pending = ds.md_stream.finish();
                        if !pending.is_empty() {
                            parser.process(&pending);
                            accumulate_ai_lines(ds.ai_rendered_lines, &pending);
                        }
                        ds.md_stream.reset();

                        // Close AI response block if open
                        let had_ai_content = ds.ai_start_scrollback.is_some();
                        if let Some(start) = ds.ai_start_scrollback.take() {
                            let ai_end = get_scrollback_line(parser);
                            let line_count = (ai_end.saturating_sub(start)) as u16;
                            if line_count > 0 {
                                let mut lines = std::mem::take(ds.ai_rendered_lines);
                                // Remove trailing empty string from accumulator
                                if lines.last().is_some_and(|s| s.is_empty()) {
                                    lines.pop();
                                }
                                ds.tracker.record_ai_response(start, line_count, lines);
                            } else {
                                ds.ai_rendered_lines.clear();
                            }
                        }
                        if finalized || !pending.is_empty() || had_ai_content {
                            parser.process(b"\r\n");
                        }

                        // Record and show yellow bold header (name only, args come later)
                        let scrollback_line = get_scrollback_line(parser);
                        let canonical = canonical_tool_name(&tool_name);
                        log::debug!(
                            "ToolCallDelta: scrollback_line={}, tool={}",
                            scrollback_line,
                            canonical
                        );
                        ds.tracker.record_tool_call_delta(canonical, scrollback_line);
                        let header_line = format!("{}{}\x1b[0m\r\n", theme.ansi_tool_pending, canonical);
                        parser.process(header_line.as_bytes());
                    }
                    // Subsequent deltas: no-op
                }
                ipc_messages::StreamChunk::ToolCall { name, arguments, pre_edit_content } => {
                    // If no ToolCallDelta preceded this, do the visual transition now
                    if ds.tracker.pending_tool().is_none() {
                        let finalized = finalize_thinking_ds(parser, ds, theme);
                        let pending = ds.md_stream.finish();
                        if !pending.is_empty() {
                            parser.process(&pending);
                            accumulate_ai_lines(ds.ai_rendered_lines, &pending);
                        }
                        ds.md_stream.reset();

                        let had_ai_content = ds.ai_start_scrollback.is_some();
                        if let Some(start) = ds.ai_start_scrollback.take() {
                            let ai_end = get_scrollback_line(parser);
                            let line_count = (ai_end.saturating_sub(start)) as u16;
                            if line_count > 0 {
                                let mut lines = std::mem::take(ds.ai_rendered_lines);
                                if lines.last().is_some_and(|s| s.is_empty()) {
                                    lines.pop();
                                }
                                ds.tracker.record_ai_response(start, line_count, lines);
                            } else {
                                ds.ai_rendered_lines.clear();
                            }
                        }
                        if finalized || !pending.is_empty() || had_ai_content {
                            parser.process(b"\r\n");
                        }
                    }

                    ds.tracker.record_tool_call(&name, &arguments, pre_edit_content.as_deref());
                    // Don't update header here — the ConfirmationRequest (if permission-gated)
                    // or ToolResult handler will render the final header with full args.
                    // This avoids a double-render when ConfirmationRequest follows immediately.
                }
                ipc_messages::StreamChunk::ToolResult { name, result } => {
                    {
                        // Move up to overwrite the placeholder header only
                        // if the cursor is still past the tool's start line.
                        // When a confirmation dialog was cleared, the cursor is
                        // already at the start — moving up would eat the spacer.
                        if let Some(start) = ds.tracker.pending_tool_start_index() {
                            if get_scrollback_line(parser) > start {
                                parser.process(b"\x1b[A\r\x1b[K");
                            }
                        }
                        let scrollback_line = get_scrollback_line(parser);
                        match ds.tracker.record_tool_result(&name, &result, scrollback_line) {
                            ToolResultAction::Merged { start_index, header, summary, preview_lines } => {
                                // Consume any pending permission outcome if it matches this tool
                                if ds.last_permission_outcome
                                    .as_ref()
                                    .is_some_and(|(t, _, _)| t.eq_ignore_ascii_case(&name))
                                {
                                    let _ = ds.last_permission_outcome.take();
                                }
                                // Erase from current position back to the block's start line.
                                let current_line = get_scrollback_line(parser);
                                let lines_to_erase = (current_line.saturating_sub(start_index)) as u16;
                                for _ in 0..lines_to_erase {
                                    parser.process(b"\x1b[A\r\x1b[K");
                                }
                                // Build rendered lines for the merged block
                                let rendered_lines = build_tool_rendered_lines(
                                    &name, &header, &summary, &preview_lines, None, theme,
                                );
                                write_block_lines_to_parser(parser, &rendered_lines);
                                ds.tracker.set_last_tool_rendered_lines(rendered_lines);
                                parser.process(b"\r\n"); // trailing separator
                            }
                            ToolResultAction::Summary { header, summary, preview_lines } => {
                                // Check if there's a permission outcome matching this tool
                                let perm = if ds.last_permission_outcome
                                    .as_ref()
                                    .is_some_and(|(t, _, _)| t.eq_ignore_ascii_case(&name))
                                {
                                    ds.last_permission_outcome.take()
                                } else {
                                    None
                                };

                                let rendered_lines = build_tool_rendered_lines(
                                    &name, &header, &summary, &preview_lines, perm.as_ref(), theme,
                                );
                                write_block_lines_to_parser(parser, &rendered_lines);
                                ds.tracker.set_last_tool_rendered_lines(rendered_lines);
                                parser.process(b"\r\n"); // trailing separator
                            }
                        }
                    }
                }
            };
        }
        ipc_messages::DaemonMessage::AgentComplete {
            request_id: _,
            summary: _,
        } => {
            // During compaction, skip AgentComplete finalization —
            // CompactionComplete will handle everything.
            if ds.pending_compaction_count.is_some() {
                return;
            }
            let finalized = finalize_thinking_ds(parser, ds, theme);
            if *ds.agent_streaming {
                let final_bytes = ds.md_stream.finish();
                if !final_bytes.is_empty() {
                    parser.process(&final_bytes);
                    accumulate_ai_lines(ds.ai_rendered_lines, &final_bytes);
                }
                let ai_end = get_scrollback_line(parser);
                let had_ai_content = ds.ai_start_scrollback.is_some();
                if let Some(start) = ds.ai_start_scrollback.take() {
                    let line_count = (ai_end.saturating_sub(start)) as u16;
                    if line_count > 0 {
                        let mut lines = std::mem::take(ds.ai_rendered_lines);
                        if lines.last().is_some_and(|s| s.is_empty()) {
                            lines.pop();
                        }
                        ds.tracker.record_ai_response(start, line_count, lines);
                    } else {
                        ds.ai_rendered_lines.clear();
                    }
                }
                if finalized || !final_bytes.is_empty() || had_ai_content {
                    parser.process(b"\r\n"); // AI block trailing separator
                }
                ds.md_stream.reset();
                *ds.agent_streaming = false;
                *ds.active_request_id = None;
            }
        }
        ipc_messages::DaemonMessage::ConfirmationRequest {
            request_id,
            description,
            risk,
            tool_name,
            tool_args,
        } => {
            // Overwrite the yellow placeholder header with the full tool call details.
            // The ToolCallDelta rendered just the tool name (e.g. "Bash"); now we have
            // the full args from the ConfirmationRequest and can show the complete header.
            let args: serde_json::Value =
                serde_json::from_str(&tool_args).unwrap_or(serde_json::Value::Null);
            let header_lines = crate::ui::blocks::build_tool_header_lines(&tool_name, &args, 10);

            // Move cursor up to overwrite the placeholder header line
            if ds.tracker.pending_tool().is_some() {
                parser.process(b"\x1b[A\r\x1b[K");
            }

            let mut prompt_lines: u16 = 0;

            // Render multi-line yellow header
            for (i, line) in header_lines.iter().enumerate() {
                if i == 0 {
                    let header_line = format!("{}{}\x1b[0m\r\n", theme.ansi_tool_pending, line);
                    parser.process(header_line.as_bytes());
                } else {
                    let content_line = format!("{}{}\x1b[0m\r\n", theme.ansi_tool_pending, line);
                    parser.process(content_line.as_bytes());
                }
                prompt_lines += 1;
            }

            // Render risk label with ⎿
            let risk_label = match risk {
                codiv_common::messages::RiskLevel::Critical => {
                    "\x1b[31m\x1b[1m\u{1F534} CRITICAL\x1b[0m"
                }
                codiv_common::messages::RiskLevel::High => {
                    "\x1b[33m\x1b[1m\u{26A0}\u{FE0F}  HIGH RISK\x1b[0m"
                }
                codiv_common::messages::RiskLevel::Medium => {
                    "\x1b[33m\u{26A0}\u{FE0F}  MEDIUM\x1b[0m"
                }
                codiv_common::messages::RiskLevel::Low => "\x1b[32mLOW\x1b[0m",
            };
            parser.process(format!("  \u{23BF} {}\r\n", risk_label).as_bytes());
            prompt_lines += 1;

            // Build options (extra indent, no ⎿)
            // Permanent allow/deny only for bash and non-native tools
            let is_native = matches!(tool_name.as_str(), "read" | "write" | "edit" | "glob" | "grep");
            let options: Vec<&str> = if is_native {
                vec![
                    "1. Yes, allow this action",
                    "2. No, reject",
                ]
            } else if risk == codiv_common::messages::RiskLevel::Critical {
                vec![
                    "1. Yes, allow this action",
                    "2. No, reject",
                    "3. No, and never allow (permanent)",
                ]
            } else {
                vec![
                    "1. Yes, allow this action",
                    "2. Yes, and always allow (permanent)",
                    "3. No, reject",
                    "4. No, and never allow (permanent)",
                ]
            };
            let option_count = options.len();

            // Render options with first one selected
            for (i, option) in options.iter().enumerate() {
                let (prefix, color) = if i == 0 {
                    ("\u{203a}", theme.ansi_prompt_selected) // › bold for selected
                } else {
                    (" ", theme.ansi_prompt_unselected)
                };
                parser.process(format!("    {}{} {}\x1b[0m\r\n", color, prefix, option).as_bytes());
                prompt_lines += 1;
            }

            *ds.pending_confirmation = Some(PendingConfirmation {
                request_id,
                description,
                risk,
                tool_name,
                tool_args,
                prompt_lines,
                selected_index: 0,
                option_count,
            });
        }
        ipc_messages::DaemonMessage::PermissionModeChanged { mode } => {
            *ds.permission_mode = mode;
        }
        ipc_messages::DaemonMessage::PermissionOutcome {
            tool_name,
            granted,
            reason,
        } => {
            *ds.last_permission_outcome = Some((tool_name, granted, reason));
        }
        ipc_messages::DaemonMessage::Error {
            request_id,
            message,
        } => {
            *ds.agent_streaming = false;
            *ds.active_request_id = None;
            ds.md_stream.reset();
            let _ = request_id; // suppress unused warning
            parser_push_notice(parser, NoticeKind::Error, &format!("[error] {}", message));
        }
        ipc_messages::DaemonMessage::AgentMeta {
            model_alias: alias,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            context_window,
        } => {
            if !alias.is_empty() { *ds.model_alias = alias; }
            if input_tokens > 0 || output_tokens > 0 {
                ds.token_usage.record_request(input_tokens, output_tokens, cache_read_tokens, context_window);
            } else {
                ds.token_usage.context_window = context_window;
            }
        }
        ipc_messages::DaemonMessage::Heartbeat { timestamp } => {
            *ds.last_daemon_timestamp = timestamp;
        }
        ipc_messages::DaemonMessage::SessionCreated {
            session_id: sid,
            name,
        } => {
            *ds.session_id = Some(sid);
            *ds.session_name = name;
            ds.token_usage.reset();
        }
        ipc_messages::DaemonMessage::SessionNameUpdated {
            session_id: _,
            name,
        } => {
            *ds.session_name = Some(name);
        }
        ipc_messages::DaemonMessage::Notice { .. } => {}
        // Shell lease protocol messages are handled in handle_daemon_message (needs full state)
        ipc_messages::DaemonMessage::AcquireShellLease { .. } => {}
        ipc_messages::DaemonMessage::ExecuteLeasedCommand { .. } => {}
        ipc_messages::DaemonMessage::ReleaseShellLease { .. } => {}
        ipc_messages::DaemonMessage::CancelLeasedCommand { .. } => {}
        // CompactionStarted/Complete are handled in handle_daemon_message (needs full state)
        ipc_messages::DaemonMessage::CompactionStarted { .. } => {}
        ipc_messages::DaemonMessage::CompactionComplete { .. } => {}
        // SessionList and SessionReplay are handled in handle_daemon_message
        ipc_messages::DaemonMessage::SessionList { .. }
        | ipc_messages::DaemonMessage::SessionReplay { .. } => {}
    }
}

/// Handle a lease protocol message. Returns frames to send to the daemon.
/// Separated from handle_daemon_message to avoid borrow conflicts with `client`.
pub(crate) fn handle_lease_message(
    msg: &ipc_messages::DaemonMessage,
    state: &mut TerminalState,
) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    match msg {
        ipc_messages::DaemonMessage::AcquireShellLease {
            lease_id,
            request_id,
        } => {
            if state.cmd.pending_command.is_none() && state.shell.shell_relay.active_lease.is_none() {
                state.shell.shell_relay.active_lease = Some(super::state::ActiveLease {
                    lease_id: lease_id.clone(),
                    request_id: request_id.clone(),
                    current_command: None,
                });
                if let Some(frame) = ipc_messages::build_shell_lease_acquired(lease_id) {
                    frames.push(frame);
                }
            } else {
                let queue_len = state.shell.shell_relay.pending_leases.len() + 1;
                state.shell.shell_relay.pending_leases.push_back(
                    super::state::PendingLease { lease_id: lease_id.clone(), request_id: request_id.clone() },
                );
                if let Some(frame) = ipc_messages::build_shell_lease_queued(lease_id, queue_len) {
                    frames.push(frame);
                }
            }
        }
        ipc_messages::DaemonMessage::ExecuteLeasedCommand {
            lease_id,
            execution_id,
            command,
            execution_timeout_ms,
        } => {
            if let Some(ref mut lease) = state.shell.shell_relay.active_lease {
                if lease.lease_id == *lease_id {
                    lease.current_command = Some(super::state::ActiveLeasedCommand {
                        execution_id: execution_id.clone(),
                        command: command.clone(),
                        _timeout_ms: *execution_timeout_ms,
                    });
                }
            }
        }
        ipc_messages::DaemonMessage::ReleaseShellLease { lease_id, .. } => {
            if let Some(ref lease) = state.shell.shell_relay.active_lease {
                if lease.lease_id == *lease_id {
                    state.shell.shell_relay.active_lease = None;
                    if let Some(frame) = ipc_messages::build_shell_lease_released(lease_id) {
                        frames.push(frame);
                    }
                    // Promote next pending lease if coprocess is free.
                    frames.extend(promote_pending_lease_frames(state));
                }
            }
        }
        ipc_messages::DaemonMessage::CancelLeasedCommand {
            lease_id,
            execution_id,
            ..
        } => {
            if let Some(frame) = ipc_messages::build_command_cancelled(
                lease_id,
                execution_id,
                &state.shell.cwd,
            ) {
                frames.push(frame);
            }
        }
        _ => {}
    }
    frames
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
            if sessions.is_empty() {
                parser_push_notice(parser, NoticeKind::Notice, "No saved sessions.");
            } else {
                use super::state::VISIBLE_SESSIONS;
                let visible_count = sessions.len().min(VISIBLE_SESSIONS);
                let mut prompt_lines: u16 = 0;

                // Show only the first `visible_count` sessions
                for (i, s) in sessions.iter().take(visible_count).enumerate() {
                    let name = s.name.as_deref().unwrap_or("(unnamed)");
                    let time = codiv_common::conversation::relative_time(&s.updated_at);
                    let (prefix, color) = if i == 0 {
                        ("\u{203a}", theme.ansi_prompt_selected)
                    } else {
                        (" ", theme.ansi_prompt_unselected)
                    };
                    let line = format!(
                        "{}  {}[{}] {} {}({})\x1b[0m\r\n",
                        color,
                        prefix,
                        i + 1,
                        name,
                        theme.ansi_thinking,
                        time
                    );
                    parser.process(line.as_bytes());
                    prompt_lines += 1;
                }
                // Select prompt (blank line + instruction)
                let scroll_hint = if sessions.len() > VISIBLE_SESSIONS {
                    " (\u{2191}\u{2193} to scroll)"
                } else {
                    ""
                };
                let select_line = format!(
                    "\r\nSelect session or Esc to cancel:{}\r\n",
                    scroll_hint
                );
                parser.process(select_line.as_bytes());
                prompt_lines += 2; // blank line + select line

                state.modal.pending_session_picker = Some(PendingSessionPicker {
                    sessions,
                    selected_index: 0,
                    viewport_offset: 0,
                    prompt_lines,
                });
            }
        }
        ipc_messages::DaemonMessage::SessionReplay { events } => {
            // Clear screen and block state
            state.tracker.clear();
            state.ui.prompt_anchor_row = None;
            state.tracker.set_replay_mode(true);

            let mut replay_token_usage = TokenUsage::default();
            let mut replay_md = MarkdownStream::new(md_stream_width, theme);

            // Build blocks from events
            for event in &events {
                match event {
                    ConversationEvent::UserPrompt { text, .. } => {
                        let rendered_line = format!("{}{}\x1b[0m", theme.ansi_user_prompt, text);
                        state.tracker.add_prompt_block(
                            text,
                            crate::ui::blocks::InputMode::Ai,
                            vec![rendered_line],
                        );
                    }
                    ConversationEvent::ShellCommand {
                        command,
                        output,
                        exit_code,
                        ..
                    } => {
                        let mut raw_bytes = Vec::new();
                        let cmd_display =
                            format!("{}{}\x1b[0m\r\n", theme.ansi_user_prompt, command);
                        raw_bytes.extend_from_slice(cmd_display.as_bytes());
                        let out_preview = if output.len() > 200 {
                            format!("{}...", &output[..200])
                        } else {
                            output.clone()
                        };
                        if !out_preview.is_empty() {
                            let normalized =
                                out_preview.replace("\r\n", "\n").replace('\n', "\r\n");
                            raw_bytes.extend_from_slice(normalized.as_bytes());
                            if !out_preview.ends_with('\n') {
                                raw_bytes.extend_from_slice(b"\r\n");
                            }
                        }
                        let color = if *exit_code == 0 {
                            theme.ansi_exit_success
                        } else {
                            theme.ansi_exit_failure
                        };
                        let exit_line = format!("{}exit {}\x1b[0m\r\n", color, exit_code);
                        raw_bytes.extend_from_slice(exit_line.as_bytes());
                        // Compute height by counting \r\n in raw_bytes
                        let height = raw_bytes
                            .windows(2)
                            .filter(|w| w == b"\r\n")
                            .count() as u16;
                        state.tracker.add_cmd_response_block(
                            command,
                            *exit_code,
                            raw_bytes,
                            height,
                        );
                    }
                    ConversationEvent::AssistantReasoning {
                        text,
                        duration_secs,
                        ..
                    } => {
                        let rendered_line = format!(
                            "{}Thought for {:.0}s\x1b[0m",
                            theme.ansi_thinking, duration_secs
                        );
                        state.tracker.add_thinking_block(
                            text.clone(),
                            *duration_secs,
                            rendered_line,
                        );
                    }
                    ConversationEvent::AssistantText { text, .. } => {
                        // Render markdown through MarkdownStream
                        replay_md.reset();
                        let trimmed = text.trim_start();
                        if !trimmed.is_empty() {
                            let mut all_ansi = Vec::new();
                            if let Some(ansi) = replay_md.push(trimmed) {
                                all_ansi.extend_from_slice(&ansi);
                            }
                            let final_bytes = replay_md.finish();
                            if !final_bytes.is_empty() {
                                all_ansi.extend_from_slice(&final_bytes);
                            }
                            replay_md.reset();

                            if !all_ansi.is_empty() {
                                let mut lines = Vec::new();
                                accumulate_ai_lines(&mut lines, &all_ansi);
                                // Remove trailing empty line from accumulator
                                if lines.last().is_some_and(|s| s.is_empty()) {
                                    lines.pop();
                                }
                                if !lines.is_empty() {
                                    state.tracker.add_ai_response_block(lines);
                                }
                            }
                        }
                    }
                    ConversationEvent::ToolCall {
                        tool_name,
                        arguments,
                        ..
                    } => {
                        state.tracker.record_tool_call(tool_name, arguments, None);
                    }
                    ConversationEvent::ToolResult {
                        tool_name, result, ..
                    } => {
                        let action =
                            state.tracker.record_tool_result(tool_name, result, 0);
                        let (header, summary, preview_lines) = match action {
                            ToolResultAction::Summary {
                                header,
                                summary,
                                preview_lines,
                            } => (header, summary, preview_lines),
                            ToolResultAction::Merged {
                                header,
                                summary,
                                preview_lines,
                                ..
                            } => (header, summary, preview_lines),
                        };
                        let rendered_lines = build_tool_rendered_lines(
                            tool_name,
                            &header,
                            &summary,
                            &preview_lines,
                            None,
                            theme,
                        );
                        state.tracker.set_last_tool_rendered_lines(rendered_lines);
                    }
                    ConversationEvent::TokenUsage {
                        input_tokens,
                        output_tokens,
                        cache_read_tokens,
                        ..
                    } => {
                        replay_token_usage.record_request(
                            *input_tokens,
                            *output_tokens,
                            *cache_read_tokens,
                            0,
                        );
                    }
                    ConversationEvent::Error { .. } => {
                        // Errors are ephemeral, skip during replay
                    }
                    ConversationEvent::Summary {
                        text,
                        compacted_event_count,
                    } => {
                        // Render summary text through markdown to get ANSI lines
                        let mut preview: Vec<String> = Vec::new();
                        replay_md.reset();
                        let trimmed = text.trim_start();
                        if !trimmed.is_empty() {
                            let mut all_ansi = Vec::new();
                            if let Some(ansi) = replay_md.push(trimmed) {
                                all_ansi.extend_from_slice(&ansi);
                            }
                            let final_bytes = replay_md.finish();
                            if !final_bytes.is_empty() {
                                all_ansi.extend_from_slice(&final_bytes);
                            }
                            replay_md.reset();

                            if !all_ansi.is_empty() {
                                let mut lines = Vec::new();
                                accumulate_ai_lines(&mut lines, &all_ansi);
                                if lines.last().is_some_and(|s| s.is_empty()) {
                                    lines.pop();
                                }
                                preview = lines.into_iter().take(100).collect();
                            }
                        }

                        // One tool block with markdown preview (no separate AI response block)
                        let header = "Compact".to_string();
                        let summary_line = format!("  \u{2514} {} events summarized", compacted_event_count);
                        let tool_rendered = build_tool_rendered_lines(
                            "Compact", &header, &summary_line, &preview, None, theme,
                        );
                        state.tracker.add_tool_block(
                            "Compact", header, summary_line, text.clone(), false, tool_rendered,
                        );
                    }
                }
            }

            state.tracker.set_replay_mode(false);
            state.token_usage = replay_token_usage;

            // Render all blocks at once
            rerender_all(parser, &mut state.tracker, &mut state.ui.scroll_offset);
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
            state.ui.prompt_is_live = false;
            state.ui.prompt_anchor_row = None;
            reset_screen(parser, &mut state.ui.scroll_offset, &mut state.tracker);

            // Show pending yellow header + placeholder summary (2-line tool block pattern)
            let scrollback_line = get_scrollback_line(parser);
            state.tracker.record_tool_call_delta("Compact", scrollback_line);
            let header_line = format!("{}{}\x1b[0m\r\n", theme.ansi_tool_pending, "Compact");
            parser.process(header_line.as_bytes());
            let summary_placeholder = format!("{}  compacting...\x1b[0m\r\n", theme.ansi_tool_pending);
            parser.process(summary_placeholder.as_bytes());

            // Store compacted event count for CompactionComplete
            state.pending_compaction_count = Some(compacted_event_count);

            // Enter streaming mode for the incoming summary text
            state.stream.agent_streaming = true;
            state.stream.active_request_id = Some(request_id);
            state.stream.md_stream.reset();
            state.stream.ai_start_scrollback = None;
            state.stream.ai_rendered_lines.clear();
            state.ui.needs_render = true;
        }
        ipc_messages::DaemonMessage::CompactionComplete {
            summary,
            compacted_event_count,
        } => {
            let final_bytes = state.stream.md_stream.finish();
            if !final_bytes.is_empty() {
                accumulate_ai_lines(&mut state.stream.ai_rendered_lines, &final_bytes);
            }
            let mut ai_lines = std::mem::take(&mut state.stream.ai_rendered_lines);
            if ai_lines.last().is_some_and(|s| s.is_empty()) {
                ai_lines.pop();
            }
            let preview: Vec<String> = ai_lines.into_iter().take(100).collect();

            let summary_line = format!("  \u{2514} {} events summarized", compacted_event_count);
            let rendered_lines = build_tool_rendered_lines(
                "Compact", "Compact", &summary_line, &preview, None, theme,
            );

            state.tracker.clear();
            state.tracker.add_tool_block(
                "Compact",
                "Compact".to_string(),
                summary_line,
                summary,
                false,
                rendered_lines,
            );
            rerender_all(parser, &mut state.tracker, &mut state.ui.scroll_offset);

            state.stream.agent_streaming = false;
            state.stream.active_request_id = None;
            state.stream.ai_start_scrollback = None;
            state.pending_compaction_count = None;
            state.ui.prompt_anchor_row = None;
            state.stream.md_stream.reset();
            state.ui.needs_render = true;
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
            handle_single_message(other, parser, &mut ds, theme);
        }
    }
}

/// Try to promote the next pending lease and return frames to send.
fn promote_pending_lease_frames(state: &mut TerminalState) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    if state.shell.shell_relay.active_lease.is_some() || state.cmd.pending_command.is_some() {
        return frames;
    }
    if let Some(pending) = state.shell.shell_relay.pending_leases.pop_front() {
        let lease_id = pending.lease_id.clone();
        state.shell.shell_relay.active_lease = Some(super::state::ActiveLease {
            lease_id: lease_id.clone(),
            request_id: pending.request_id,
            current_command: None,
        });
        if let Some(frame) = ipc_messages::build_shell_lease_acquired(&lease_id) {
            frames.push(frame);
        }
    }
    frames
}

/// Try to promote the next pending lease to active when the coprocess is free.
/// Sends frames directly to the client.
pub(crate) fn promote_pending_lease(
    state: &mut TerminalState,
    client: &mut Option<crate::ipc::client::CodivdClient>,
) {
    let frames = promote_pending_lease_frames(state);
    if let Some(ref mut c) = client {
        for frame in frames {
            c.send(&frame);
        }
    }
}
