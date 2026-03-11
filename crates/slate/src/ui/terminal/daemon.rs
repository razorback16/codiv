use std::time::{Duration, Instant};

use slate_common::conversation::ConversationEvent;
use slate_common::permissions::PermissionMode;

use crate::ipc::messages as ipc_messages;
use crate::markdown::MarkdownStream;
use crate::ui::blocks::{canonical_tool_name, BlockRegistry, ToolResultAction};

use super::state::{PendingConfirmation, PendingSessionPicker};
use super::utils::{get_scrollback_line, parser_push_styled};

/// Finalize an in-progress thinking block: overwrite the placeholder line
/// with a "Thought for Ns" summary and register it in the block tracker.
fn finalize_thinking(
    parser: &mut vt100::Parser,
    tracker: &mut BlockRegistry,
    thinking_buffer: &mut String,
    thinking_start: &mut Option<Instant>,
    thinking_scrollback: &mut Option<u64>,
) -> bool {
    if let Some(start) = thinking_start.take() {
        let duration_secs = start.elapsed().as_secs_f32();
        // Move cursor up one line and clear it (overwrite placeholder)
        parser.process(b"\x1b[A\r\x1b[K");
        let summary = format!(
            "\x1b[90mThought for {:.0}s\x1b[0m\r\n",
            duration_secs
        );
        parser.process(summary.as_bytes());
        let content = std::mem::take(thinking_buffer);
        if let Some(sl) = thinking_scrollback.take() {
            tracker.record_pending_thinking(content, duration_secs, sl);
        }
        true
    } else {
        false
    }
}

/// Items produced by converting a `ConversationEvent` for replay through the
/// live rendering path.
enum ReplayItem {
    /// User prompt — rendered directly (not a DaemonMessage).
    UserPrompt { text: String },
    /// Shell command — rendered directly (not a DaemonMessage).
    ShellCommand { command: String, output: String, exit_code: i32 },
    /// Set agent_streaming = true so AgentComplete knows to flush.
    SetAgentStreaming,
    /// Backdate `thinking_start` so `finalize_thinking` computes the right duration.
    SetThinkingDuration(f32),
    /// A synthetic DaemonMessage to feed through `handle_single_message`.
    Daemon(ipc_messages::DaemonMessage),
}

/// Convert a single `ConversationEvent` into one or more `ReplayItem`s that,
/// when processed sequentially, reproduce the same visual output as the live
/// streaming path.
fn convert_event_to_replay_items(event: &ConversationEvent) -> Vec<ReplayItem> {
    match event {
        ConversationEvent::UserPrompt { text, .. } => {
            vec![ReplayItem::UserPrompt { text: text.clone() }]
        }
        ConversationEvent::ShellCommand { command, output, exit_code, .. } => {
            vec![ReplayItem::ShellCommand {
                command: command.clone(),
                output: output.clone(),
                exit_code: *exit_code,
            }]
        }
        ConversationEvent::AssistantReasoning { text, duration_secs, .. } => {
            vec![
                ReplayItem::SetAgentStreaming,
                ReplayItem::Daemon(ipc_messages::DaemonMessage::AgentStreamChunk {
                    request_id: String::new(),
                    chunk: ipc_messages::StreamChunk::Reasoning(text.clone()),
                }),
                ReplayItem::SetThinkingDuration(*duration_secs),
            ]
        }
        ConversationEvent::AssistantText { text, .. } => {
            vec![
                ReplayItem::SetAgentStreaming,
                ReplayItem::Daemon(ipc_messages::DaemonMessage::AgentStreamChunk {
                    request_id: String::new(),
                    chunk: ipc_messages::StreamChunk::Text(text.clone()),
                }),
                ReplayItem::Daemon(ipc_messages::DaemonMessage::AgentComplete {
                    request_id: String::new(),
                    summary: String::new(),
                }),
            ]
        }
        ConversationEvent::ToolCall { tool_name, arguments, .. } => {
            let canonical = canonical_tool_name(tool_name);
            vec![
                ReplayItem::Daemon(ipc_messages::DaemonMessage::AgentStreamChunk {
                    request_id: String::new(),
                    chunk: ipc_messages::StreamChunk::ToolCallDelta {
                        tool_call_id: String::new(),
                        tool_name: canonical.to_string(),
                        delta: String::new(),
                    },
                }),
                ReplayItem::Daemon(ipc_messages::DaemonMessage::AgentStreamChunk {
                    request_id: String::new(),
                    chunk: ipc_messages::StreamChunk::ToolCall {
                        name: tool_name.clone(),
                        arguments: arguments.clone(),
                    },
                }),
            ]
        }
        ConversationEvent::ToolResult { tool_name, result, .. } => {
            vec![ReplayItem::Daemon(ipc_messages::DaemonMessage::AgentStreamChunk {
                request_id: String::new(),
                chunk: ipc_messages::StreamChunk::ToolResult {
                    name: tool_name.clone(),
                    result: result.clone(),
                },
            })]
        }
        ConversationEvent::Error { request_id, message } => {
            vec![ReplayItem::Daemon(ipc_messages::DaemonMessage::Error {
                request_id: request_id.clone(),
                message: message.clone(),
            })]
        }
    }
}

/// Handle a single `DaemonMessage` — all match arms except `SessionList` and
/// `SessionReplay`, which are handled in the outer `handle_daemon_message`.
#[allow(clippy::too_many_arguments)]
fn handle_single_message(
    msg: ipc_messages::DaemonMessage,
    parser: &mut vt100::Parser,
    md_stream: &mut MarkdownStream,
    agent_streaming: &mut bool,
    last_daemon_timestamp: &mut u64,
    model_alias: &mut String,
    context_usage: &mut (usize, usize),
    tracker: &mut BlockRegistry,
    ai_start_scrollback: &mut Option<u64>,
    thinking_buffer: &mut String,
    thinking_start: &mut Option<Instant>,
    thinking_scrollback: &mut Option<u64>,
    pending_confirmation: &mut Option<PendingConfirmation>,
    permission_mode: &mut PermissionMode,
    last_permission_outcome: &mut Option<(String, bool, String)>,
    session_id: &mut Option<String>,
    session_name: &mut Option<String>,
) {
    match msg {
        ipc_messages::DaemonMessage::AgentStreamChunk {
            request_id: _,
            chunk,
        } => {
            match chunk {
                ipc_messages::StreamChunk::Text(t) => {
                    finalize_thinking(parser, tracker, thinking_buffer, thinking_start, thinking_scrollback);
                    // Strip leading whitespace from the first text chunk of a response.
                    let t = if ai_start_scrollback.is_none() {
                        t.trim_start().to_string()
                    } else {
                        t
                    };
                    if t.is_empty() {
                        // Entire chunk was leading whitespace — skip it.
                    } else {
                        if ai_start_scrollback.is_none() {
                            *ai_start_scrollback = Some(get_scrollback_line(parser));
                        }
                        if let Some(ansi) = md_stream.push(&t) {
                            parser.process(&ansi);
                        }
                    }
                }
                ipc_messages::StreamChunk::Reasoning(t) => {
                    if thinking_start.is_none() {
                        *thinking_start = Some(Instant::now());
                        *thinking_scrollback = Some(get_scrollback_line(parser));
                        // Write placeholder line
                        parser.process(b"\x1b[90mThinking...\x1b[0m\r\n");
                    }
                    thinking_buffer.push_str(&t);
                }
                ipc_messages::StreamChunk::ToolCallDelta { tool_call_id: _, tool_name, delta: _ } => {
                    // On the FIRST delta for a tool call, show spinner placeholder
                    if tracker.pending_tool().is_none() {
                        let had_thinking = finalize_thinking(parser, tracker, thinking_buffer, thinking_start, thinking_scrollback);
                        // Flush any buffered markdown
                        let pending = md_stream.finish();
                        if !pending.is_empty() {
                            parser.process(&pending);
                        }
                        md_stream.reset();

                        // Close AI response block if open
                        let had_ai_content = ai_start_scrollback.is_some();
                        if let Some(start) = ai_start_scrollback.take() {
                            let ai_end = get_scrollback_line(parser);
                            let line_count = (ai_end.saturating_sub(start)) as u16;
                            if line_count > 0 {
                                tracker.record_ai_response(start, line_count);
                            }
                        }
                        if !pending.is_empty() || had_ai_content || had_thinking {
                            parser.process(b"\r\n");
                        }

                        // Record and show yellow bold header (name only, args come later)
                        let scrollback_line = get_scrollback_line(parser);
                        let canonical = canonical_tool_name(&tool_name);
                        log::debug!("ToolCallDelta: scrollback_line={}, tool={}", scrollback_line, canonical);
                        tracker.record_tool_call_delta(canonical, scrollback_line);
                        let header_line = format!("\x1b[1m\x1b[33m{}\x1b[0m\r\n", canonical);
                        parser.process(header_line.as_bytes());
                    }
                    // Subsequent deltas: no-op
                }
                ipc_messages::StreamChunk::ToolCall { name, arguments } => {
                    // If no ToolCallDelta preceded this, do the visual transition now
                    if tracker.pending_tool().is_none() {
                        let had_thinking = finalize_thinking(parser, tracker, thinking_buffer, thinking_start, thinking_scrollback);
                        let pending = md_stream.finish();
                        if !pending.is_empty() {
                            parser.process(&pending);
                        }
                        md_stream.reset();

                        let had_ai_content = ai_start_scrollback.is_some();
                        if let Some(start) = ai_start_scrollback.take() {
                            let ai_end = get_scrollback_line(parser);
                            let line_count = (ai_end.saturating_sub(start)) as u16;
                            if line_count > 0 {
                                tracker.record_ai_response(start, line_count);
                            }
                        }
                        if !pending.is_empty() || had_ai_content || had_thinking {
                            parser.process(b"\r\n");
                        }
                    }

                    tracker.record_tool_call(&name, &arguments);
                    // Don't update header here — the ConfirmationRequest (if permission-gated)
                    // or ToolResult handler will render the final header with full args.
                    // This avoids a double-render when ConfirmationRequest follows immediately.
                }
                ipc_messages::StreamChunk::ToolResult { name, result } => {
                    {
                        let had_pending = tracker.pending_tool().is_some();
                        if had_pending {
                            // Move up to overwrite the yellow header line
                            parser.process(b"\x1b[A\r\x1b[K");
                        }
                        let scrollback_line = get_scrollback_line(parser);
                        match tracker.record_tool_result(&name, &result, scrollback_line) {
                            ToolResultAction::Merged => {
                                // Consume any pending permission outcome if it matches this tool
                                if last_permission_outcome.as_ref().is_some_and(|(t, _, _)| t.eq_ignore_ascii_case(&name)) {
                                    let _ = last_permission_outcome.take();
                                }
                                // Block merged into previous — update the summary
                                // line in the VT100 buffer by moving cursor up and
                                // rewriting the line.
                                let summary = tracker
                                    .last_tool_block_mut()
                                    .map(|tb| tb.summary.clone());
                                if let Some(summary) = summary {
                                    parser.process(b"\x1b[A\r\x1b[K");
                                    let line = format!("\x1b[32m{}\x1b[0m\r\n", summary);
                                    parser.process(line.as_bytes());
                                }
                            }
                            ToolResultAction::Summary { header, summary } => {
                                // Check if there's a permission outcome matching this tool
                                let perm = if last_permission_outcome.as_ref().is_some_and(|(t, _, _)| t.eq_ignore_ascii_case(&name)) {
                                    last_permission_outcome.take()
                                } else {
                                    None
                                };

                                if let Some((ref _perm_tool, granted, ref reason)) = perm {
                                    if granted {
                                        // Green header + "└ {reason}" + green summary
                                        let header_line = format!("\x1b[1m\x1b[32m{}\x1b[0m\r\n", header);
                                        parser.process(header_line.as_bytes());
                                        let perm_line = format!("\x1b[32m  \u{2514} {}\x1b[0m\r\n", reason);
                                        parser.process(perm_line.as_bytes());
                                        // Normal summary line
                                        let is_bash_error = name.eq_ignore_ascii_case("bash")
                                            && !summary.contains("exit 0");
                                        let color = if is_bash_error { "\x1b[31m" } else { "\x1b[32m" };
                                        let summary_line = format!("{}{}\x1b[0m\r\n", color, summary);
                                        parser.process(summary_line.as_bytes());
                                    } else {
                                        // Red header + "└ {reason}" (no tool summary since tool wasn't executed)
                                        let header_line = format!("\x1b[1m\x1b[31m{}\x1b[0m\r\n", header);
                                        parser.process(header_line.as_bytes());
                                        let perm_line = format!("\x1b[31m  \u{2514} {}\x1b[0m\r\n", reason);
                                        parser.process(perm_line.as_bytes());
                                    }
                                } else {
                                    // No permission check — render as before (green header + summary)
                                    let header_line = format!("\x1b[1m\x1b[32m{}\x1b[0m\r\n", header);
                                    parser.process(header_line.as_bytes());
                                    let is_bash_error = name.eq_ignore_ascii_case("bash")
                                        && !summary.contains("exit 0");
                                    let color = if is_bash_error { "\x1b[31m" } else { "\x1b[32m" };
                                    let summary_line = format!("{}{}\x1b[0m\r\n", color, summary);
                                    parser.process(summary_line.as_bytes());
                                }
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
            finalize_thinking(parser, tracker, thinking_buffer, thinking_start, thinking_scrollback);
            if *agent_streaming {
                let final_bytes = md_stream.finish();
                if !final_bytes.is_empty() {
                    parser.process(&final_bytes);
                }
                let ai_end = get_scrollback_line(parser);
                let had_ai_content = ai_start_scrollback.is_some();
                if let Some(start) = ai_start_scrollback.take() {
                    let line_count = (ai_end.saturating_sub(start)) as u16;
                    if line_count > 0 {
                        tracker.record_ai_response(start, line_count);
                    }
                }
                // Flush any pending thinking that was never consumed by record_ai_response
                // (thinking-only response with no text content).
                tracker.flush_pending_thinking();
                if !final_bytes.is_empty() || had_ai_content {
                    parser.process(b"\r\n"); // AI block trailing separator
                }
                md_stream.reset();
                *agent_streaming = false;
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
            let args: serde_json::Value = serde_json::from_str(&tool_args).unwrap_or(serde_json::Value::Null);
            let header_lines = crate::ui::blocks::build_tool_header_lines(&tool_name, &args, 10);

            // Move cursor up to overwrite the placeholder header line
            if tracker.pending_tool().is_some() {
                parser.process(b"\x1b[A\r\x1b[K");
            }

            let mut prompt_lines: u16 = 0;

            // Render multi-line yellow header
            for (i, line) in header_lines.iter().enumerate() {
                if i == 0 {
                    let header_line = format!("\x1b[1m\x1b[33m{}\x1b[0m\r\n", line);
                    parser.process(header_line.as_bytes());
                } else {
                    let content_line = format!("\x1b[33m{}\x1b[0m\r\n", line);
                    parser.process(content_line.as_bytes());
                }
                prompt_lines += 1;
            }

            // Render risk label with ⎿
            let risk_label = match risk {
                slate_common::messages::RiskLevel::Critical => "\x1b[31m\x1b[1m\u{1F534} CRITICAL\x1b[0m",
                slate_common::messages::RiskLevel::High => "\x1b[33m\x1b[1m\u{26A0}\u{FE0F}  HIGH RISK\x1b[0m",
                slate_common::messages::RiskLevel::Medium => "\x1b[33m\u{26A0}\u{FE0F}  MEDIUM\x1b[0m",
                slate_common::messages::RiskLevel::Low => "\x1b[32mLOW\x1b[0m",
            };
            parser.process(format!("  \u{23BF} {}\r\n", risk_label).as_bytes());
            prompt_lines += 1;

            // Build options (extra indent, no ⎿)
            let options: Vec<&str> = if risk == slate_common::messages::RiskLevel::Critical {
                vec![
                    "1. Yes, allow this action",
                    "2. No, reject",
                    "3. No, and never allow (session)",
                ]
            } else {
                vec![
                    "1. Yes, allow this action",
                    "2. Yes, and always allow (session)",
                    "3. No, reject",
                    "4. No, and never allow (session)",
                ]
            };
            let option_count = options.len();

            // Render options with first one selected
            for (i, option) in options.iter().enumerate() {
                let (prefix, color) = if i == 0 {
                    ("\u{203a}", "\x1b[1;37m")  // › bold white for selected
                } else {
                    (" ", "\x1b[37m")  // normal white
                };
                parser.process(format!("    {}{} {}\x1b[0m\r\n", color, prefix, option).as_bytes());
                prompt_lines += 1;
            }

            *pending_confirmation = Some(PendingConfirmation {
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
            *permission_mode = mode;
        }
        ipc_messages::DaemonMessage::PermissionOutcome {
            tool_name,
            granted,
            reason,
        } => {
            *last_permission_outcome = Some((tool_name, granted, reason));
        }
        ipc_messages::DaemonMessage::Error {
            request_id,
            message,
        } => {
            *agent_streaming = false;
            md_stream.reset();
            let _ = request_id; // suppress unused warning
            parser_push_styled(parser, &format!("[error] {}", message), "\x1b[31m");
        }
        ipc_messages::DaemonMessage::AgentMeta {
            model_alias: alias,
            total_tokens,
            context_window,
        } => {
            *model_alias = alias;
            *context_usage = (total_tokens, context_window);
        }
        ipc_messages::DaemonMessage::Heartbeat { timestamp } => {
            *last_daemon_timestamp = timestamp;
        }
        ipc_messages::DaemonMessage::SessionCreated {
            session_id: sid,
            name,
        } => {
            *session_id = Some(sid);
            *session_name = name;
        }
        ipc_messages::DaemonMessage::SessionNameUpdated {
            session_id: _,
            name,
        } => {
            *session_name = Some(name);
        }
        // SessionList and SessionReplay are handled in handle_daemon_message
        ipc_messages::DaemonMessage::SessionList { .. }
        | ipc_messages::DaemonMessage::SessionReplay { .. } => {}
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_daemon_message(
    msg: ipc_messages::DaemonMessage,
    parser: &mut vt100::Parser,
    md_stream: &mut MarkdownStream,
    agent_streaming: &mut bool,
    last_daemon_timestamp: &mut u64,
    model_alias: &mut String,
    context_usage: &mut (usize, usize),
    _cwd: &str,
    tracker: &mut BlockRegistry,
    ai_start_scrollback: &mut Option<u64>,
    thinking_buffer: &mut String,
    thinking_start: &mut Option<Instant>,
    thinking_scrollback: &mut Option<u64>,
    pending_confirmation: &mut Option<PendingConfirmation>,
    permission_mode: &mut PermissionMode,
    last_permission_outcome: &mut Option<(String, bool, String)>,
    session_id: &mut Option<String>,
    session_name: &mut Option<String>,
    pending_session_picker: &mut Option<PendingSessionPicker>,
    scroll_offset: &mut usize,
    md_stream_width: u16,
) {
    match msg {
        ipc_messages::DaemonMessage::SessionList { sessions } => {
            if sessions.is_empty() {
                parser_push_styled(parser, "No saved sessions.", "\x1b[90m");
            } else {
                let mut prompt_lines: u16 = 0;
                // Session entries (no header — the select line below is sufficient)
                for (i, s) in sessions.iter().enumerate() {
                    let name = s.name.as_deref().unwrap_or("(unnamed)");
                    let time = slate_common::conversation::relative_time(&s.updated_at);
                    let (prefix, color) = if i == 0 {
                        ("\u{203a}", "\x1b[1;37m")
                    } else {
                        (" ", "\x1b[37m")
                    };
                    let line = format!("{}  {}[{}] {} \x1b[90m({})\x1b[0m\r\n", color, prefix, i + 1, name, time);
                    parser.process(line.as_bytes());
                    prompt_lines += 1;
                }
                // Select prompt (blank line + instruction)
                let select_line = format!("\r\nSelect session [1-{}] or Esc to cancel:\r\n", sessions.len());
                parser.process(select_line.as_bytes());
                prompt_lines += 2; // blank line + select line

                *pending_session_picker = Some(PendingSessionPicker {
                    sessions,
                    selected_index: 0,
                    prompt_lines,
                });
            }
        }
        ipc_messages::DaemonMessage::SessionReplay { events } => {
            // 1. Clear screen
            let screen = parser.screen();
            let rows = screen.size().0;
            let cols = screen.size().1;
            *parser = vt100::Parser::new(rows, cols, super::state::MAX_SCROLLBACK);
            *scroll_offset = 0;
            tracker.clear();

            // 2. Create fresh local replay state
            let mut replay_md = MarkdownStream::new(md_stream_width);
            let mut replay_agent_streaming = false;
            let mut replay_timestamp: u64 = 0;
            let mut replay_model = String::new();
            let mut replay_context: (usize, usize) = (0, 0);
            let mut replay_ai_start: Option<u64> = None;
            let mut replay_thinking_buffer = String::new();
            let mut replay_thinking_start: Option<Instant> = None;
            let mut replay_thinking_scrollback: Option<u64> = None;
            let mut replay_pending_confirmation: Option<PendingConfirmation> = None;
            let mut replay_permission_mode = PermissionMode::default();
            let mut replay_last_perm: Option<(String, bool, String)> = None;
            // Use the real session_id/session_name (already set by SessionCreated)
            // so replayed state doesn't clobber them — pass dummies to handle_single_message.
            let mut replay_session_id: Option<String> = None;
            let mut replay_session_name: Option<String> = None;

            // 3. Convert each event and process through handle_single_message
            for event in &events {
                let items = convert_event_to_replay_items(event);
                for item in items {
                    match item {
                        ReplayItem::UserPrompt { text } => {
                            let scrollback_line = get_scrollback_line(parser);
                            let prompt_display = format!("\x1b[1m{}\x1b[0m\r\n", text);
                            parser.process(prompt_display.as_bytes());
                            tracker.record_prompt(&text, scrollback_line, crate::ui::blocks::InputMode::Ai);
                            parser.process(b"\r\n");
                        }
                        ReplayItem::ShellCommand { command, output, exit_code } => {
                            let scrollback_line = get_scrollback_line(parser);
                            let cmd_display = format!("\x1b[1m{}\x1b[0m\r\n", command);
                            parser.process(cmd_display.as_bytes());
                            // Show truncated output, converting bare \n to \r\n for vt100
                            let out_preview = if output.len() > 200 {
                                format!("{}...", &output[..200])
                            } else {
                                output.clone()
                            };
                            if !out_preview.is_empty() {
                                let normalized = out_preview.replace("\r\n", "\n").replace('\n', "\r\n");
                                parser.process(normalized.as_bytes());
                                if !out_preview.ends_with('\n') {
                                    parser.process(b"\r\n");
                                }
                            }
                            let color = if exit_code == 0 { "\x1b[32m" } else { "\x1b[31m" };
                            parser.process(format!("{}exit {}\x1b[0m\r\n", color, exit_code).as_bytes());
                            let end = get_scrollback_line(parser);
                            let line_count = (end.saturating_sub(scrollback_line)) as u16;
                            tracker.record_cmd_response(&command, scrollback_line, line_count, exit_code);
                            parser.process(b"\r\n");
                        }
                        ReplayItem::SetAgentStreaming => {
                            replay_agent_streaming = true;
                        }
                        ReplayItem::SetThinkingDuration(d) => {
                            // Backdate thinking_start so finalize_thinking computes the stored duration
                            if replay_thinking_start.is_some() {
                                replay_thinking_start = Some(Instant::now() - Duration::from_secs_f32(d));
                            }
                        }
                        ReplayItem::Daemon(daemon_msg) => {
                            handle_single_message(
                                daemon_msg,
                                parser,
                                &mut replay_md,
                                &mut replay_agent_streaming,
                                &mut replay_timestamp,
                                &mut replay_model,
                                &mut replay_context,
                                tracker,
                                &mut replay_ai_start,
                                &mut replay_thinking_buffer,
                                &mut replay_thinking_start,
                                &mut replay_thinking_scrollback,
                                &mut replay_pending_confirmation,
                                &mut replay_permission_mode,
                                &mut replay_last_perm,
                                &mut replay_session_id,
                                &mut replay_session_name,
                            );
                        }
                    }
                }
            }
            // Final separator before returning to normal input
            parser.process(b"\r\n");
        }
        // All other messages delegate to the core handler
        other => {
            handle_single_message(
                other,
                parser,
                md_stream,
                agent_streaming,
                last_daemon_timestamp,
                model_alias,
                context_usage,
                tracker,
                ai_start_scrollback,
                thinking_buffer,
                thinking_start,
                thinking_scrollback,
                pending_confirmation,
                permission_mode,
                last_permission_outcome,
                session_id,
                session_name,
            );
        }
    }
}
