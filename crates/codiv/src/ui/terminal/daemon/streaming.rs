use std::time::Instant;

use crate::ipc::messages as ipc_messages;
use crate::ui::blocks::{canonical_tool_name, ToolResultAction};
use crate::ui::theme::Theme;

use super::helpers::{accumulate_ai_lines, build_tool_rendered_lines};
use super::{DaemonStreamState, finalize_thinking_ds};
use crate::ui::terminal::utils::{get_scrollback_line, parser_push_notice, NoticeKind, write_block_lines_to_parser};

/// Handle a single `DaemonMessage` — all match arms except `SessionList` and
/// `SessionReplay`, which are handled in the outer `handle_daemon_message`.
pub(super) fn handle_single_message(
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
            let width = ds.tracker.width();
            let header_lines = crate::ui::blocks::build_tool_header_lines(&tool_name, &args, 10, width);

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

            *ds.pending_confirmation = Some(crate::ui::terminal::state::PendingConfirmation {
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
