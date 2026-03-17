use std::time::{Duration, Instant};

use codiv_common::conversation::ConversationEvent;
use codiv_common::permissions::PermissionMode;

use crate::ipc::messages as ipc_messages;
use crate::markdown::MarkdownStream;
use crate::ui::blocks::{canonical_tool_name, BlockRegistry, ToolResultAction};
use crate::ui::theme::Theme;

use super::state::{PendingConfirmation, PendingSessionPicker, TerminalState};
use super::utils::{finalize_thinking, get_scrollback_line, parser_push_notice, reset_screen, NoticeKind};

/// Mutable state used by the daemon message handler.
/// Constructed from `TerminalState` fields for live messages,
/// or from replay-local variables during `SessionReplay`.
pub(super) struct DaemonStreamState<'a> {
    pub md_stream: &'a mut MarkdownStream,
    pub agent_streaming: &'a mut bool,
    pub last_daemon_timestamp: &'a mut u64,
    pub model_alias: &'a mut String,
    pub context_usage: &'a mut (usize, usize),
    pub tracker: &'a mut BlockRegistry,
    pub ai_start_scrollback: &'a mut Option<u64>,
    pub thinking_buffer: &'a mut String,
    pub thinking_start: &'a mut Option<Instant>,
    pub thinking_scrollback: &'a mut Option<u64>,
    pub pending_confirmation: &'a mut Option<PendingConfirmation>,
    pub permission_mode: &'a mut PermissionMode,
    pub last_permission_outcome: &'a mut Option<(String, bool, String)>,
    pub session_id: &'a mut Option<String>,
    pub session_name: &'a mut Option<String>,
    pub active_request_id: &'a mut Option<String>,
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

/// Items produced by converting a `ConversationEvent` for replay through the
/// live rendering path.
enum ReplayItem {
    /// User prompt — rendered directly (not a DaemonMessage).
    UserPrompt { text: String },
    /// Shell command — rendered directly (not a DaemonMessage).
    ShellCommand {
        command: String,
        output: String,
        exit_code: i32,
    },
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
        ConversationEvent::ShellCommand {
            command,
            output,
            exit_code,
            ..
        } => {
            vec![ReplayItem::ShellCommand {
                command: command.clone(),
                output: output.clone(),
                exit_code: *exit_code,
            }]
        }
        ConversationEvent::AssistantReasoning {
            text,
            duration_secs,
            ..
        } => {
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
        ConversationEvent::ToolCall {
            tool_name,
            arguments,
            ..
        } => {
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
        ConversationEvent::ToolResult {
            tool_name, result, ..
        } => {
            vec![ReplayItem::Daemon(
                ipc_messages::DaemonMessage::AgentStreamChunk {
                    request_id: String::new(),
                    chunk: ipc_messages::StreamChunk::ToolResult {
                        name: tool_name.clone(),
                        result: result.clone(),
                    },
                },
            )]
        }
        ConversationEvent::Error {
            request_id,
            message,
        } => {
            vec![ReplayItem::Daemon(ipc_messages::DaemonMessage::Error {
                request_id: request_id.clone(),
                message: message.clone(),
            })]
        }
    }
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
                        }
                        ds.md_stream.reset();

                        // Close AI response block if open
                        let had_ai_content = ds.ai_start_scrollback.is_some();
                        if let Some(start) = ds.ai_start_scrollback.take() {
                            let ai_end = get_scrollback_line(parser);
                            let line_count = (ai_end.saturating_sub(start)) as u16;
                            if line_count > 0 {
                                ds.tracker.record_ai_response(start, line_count);
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
                ipc_messages::StreamChunk::ToolCall { name, arguments } => {
                    // If no ToolCallDelta preceded this, do the visual transition now
                    if ds.tracker.pending_tool().is_none() {
                        let finalized = finalize_thinking_ds(parser, ds, theme);
                        let pending = ds.md_stream.finish();
                        if !pending.is_empty() {
                            parser.process(&pending);
                        }
                        ds.md_stream.reset();

                        let had_ai_content = ds.ai_start_scrollback.is_some();
                        if let Some(start) = ds.ai_start_scrollback.take() {
                            let ai_end = get_scrollback_line(parser);
                            let line_count = (ai_end.saturating_sub(start)) as u16;
                            if line_count > 0 {
                                ds.tracker.record_ai_response(start, line_count);
                            }
                        }
                        if finalized || !pending.is_empty() || had_ai_content {
                            parser.process(b"\r\n");
                        }
                    }

                    ds.tracker.record_tool_call(&name, &arguments);
                    // Don't update header here — the ConfirmationRequest (if permission-gated)
                    // or ToolResult handler will render the final header with full args.
                    // This avoids a double-render when ConfirmationRequest follows immediately.
                }
                ipc_messages::StreamChunk::ToolResult { name, result } => {
                    {
                        let had_pending = ds.tracker.pending_tool().is_some();
                        if had_pending {
                            // Move up to overwrite the yellow header line
                            parser.process(b"\x1b[A\r\x1b[K");
                        }
                        let scrollback_line = get_scrollback_line(parser);
                        match ds.tracker.record_tool_result(&name, &result, scrollback_line) {
                            ToolResultAction::Merged => {
                                // Consume any pending permission outcome if it matches this tool
                                if ds.last_permission_outcome
                                    .as_ref()
                                    .is_some_and(|(t, _, _)| t.eq_ignore_ascii_case(&name))
                                {
                                    let _ = ds.last_permission_outcome.take();
                                }
                                // Block merged into previous — update the summary
                                // line in the VT100 buffer by moving cursor up and
                                // rewriting the line.
                                let summary =
                                    ds.tracker.last_tool_block_mut().map(|tb| tb.summary.clone());
                                if let Some(summary) = summary {
                                    parser.process(b"\x1b[A\r\x1b[K");
                                    let line = format!("{}{}\x1b[0m\r\n", theme.ansi_tool_done_suffix, summary);
                                    parser.process(line.as_bytes());
                                }
                            }
                            ToolResultAction::Summary { header, summary } => {
                                // Check if there's a permission outcome matching this tool
                                let perm = if ds.last_permission_outcome
                                    .as_ref()
                                    .is_some_and(|(t, _, _)| t.eq_ignore_ascii_case(&name))
                                {
                                    ds.last_permission_outcome.take()
                                } else {
                                    None
                                };

                                if let Some((ref _perm_tool, granted, ref reason)) = perm {
                                    if granted {
                                        // Green header + "└ {reason}" + green summary
                                        let header_line =
                                            format!("{}{}\x1b[0m\r\n", theme.ansi_tool_done, header);
                                        parser.process(header_line.as_bytes());
                                        let perm_line =
                                            format!("{}  \u{2514} {}\x1b[0m\r\n", theme.ansi_tool_done_suffix, reason);
                                        parser.process(perm_line.as_bytes());
                                        // Normal summary line
                                        let is_bash_error = name.eq_ignore_ascii_case("bash")
                                            && !summary.contains("exit 0");
                                        let color = if is_bash_error {
                                            theme.ansi_exit_failure
                                        } else {
                                            theme.ansi_tool_done_suffix
                                        };
                                        let summary_line =
                                            format!("{}{}\x1b[0m\r\n", color, summary);
                                        parser.process(summary_line.as_bytes());
                                    } else {
                                        // Red header + "└ {reason}" (no tool summary since tool wasn't executed)
                                        let header_line =
                                            format!("{}{}\x1b[0m\r\n", theme.ansi_tool_denied, header);
                                        parser.process(header_line.as_bytes());
                                        let perm_line =
                                            format!("{}  \u{2514} {}\x1b[0m\r\n", theme.ansi_tool_denied_suffix, reason);
                                        parser.process(perm_line.as_bytes());
                                    }
                                } else {
                                    // No permission check — render as before (green header + summary)
                                    let header_line =
                                        format!("{}{}\x1b[0m\r\n", theme.ansi_tool_done, header);
                                    parser.process(header_line.as_bytes());
                                    let is_bash_error = name.eq_ignore_ascii_case("bash")
                                        && !summary.contains("exit 0");
                                    let color = if is_bash_error {
                                        theme.ansi_exit_failure
                                    } else {
                                        theme.ansi_tool_done_suffix
                                    };
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
            let finalized = finalize_thinking_ds(parser, ds, theme);
            if *ds.agent_streaming {
                let final_bytes = ds.md_stream.finish();
                if !final_bytes.is_empty() {
                    parser.process(&final_bytes);
                }
                let ai_end = get_scrollback_line(parser);
                let had_ai_content = ds.ai_start_scrollback.is_some();
                if let Some(start) = ds.ai_start_scrollback.take() {
                    let line_count = (ai_end.saturating_sub(start)) as u16;
                    if line_count > 0 {
                        ds.tracker.record_ai_response(start, line_count);
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
            total_tokens,
            context_window,
        } => {
            *ds.model_alias = alias;
            *ds.context_usage = (total_tokens, context_window);
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
        }
        ipc_messages::DaemonMessage::SessionNameUpdated {
            session_id: _,
            name,
        } => {
            *ds.session_name = Some(name);
        }
        ipc_messages::DaemonMessage::Notice { message } => {
            parser_push_notice(parser, NoticeKind::Notice, &message);
        }
        // ExecuteCommand is handled in handle_daemon_message (needs full state)
        ipc_messages::DaemonMessage::ExecuteCommand { .. } => {}
        // SessionList and SessionReplay are handled in handle_daemon_message
        ipc_messages::DaemonMessage::SessionList { .. }
        | ipc_messages::DaemonMessage::SessionReplay { .. } => {}
    }
}

pub(crate) fn handle_daemon_message(
    msg: ipc_messages::DaemonMessage,
    parser: &mut vt100::Parser,
    state: &mut TerminalState,
    md_stream_width: u16,
    theme: &Theme,
) {
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

                state.pending_session_picker = Some(PendingSessionPicker {
                    sessions,
                    selected_index: 0,
                    viewport_offset: 0,
                    prompt_lines,
                });
            }
        }
        ipc_messages::DaemonMessage::SessionReplay { events } => {
            // 1. Clear screen and re-emit welcome header
            reset_screen(parser, &mut state.scroll_offset, &mut state.tracker);

            // 2. Create fresh local replay state
            let mut replay_md = MarkdownStream::new(md_stream_width, theme);
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
            let mut replay_active_request_id: Option<String> = None;

            // 3. Convert each event and process through handle_single_message
            for event in &events {
                let items = convert_event_to_replay_items(event);
                for item in items {
                    match item {
                        ReplayItem::UserPrompt { text } => {
                            let scrollback_line = get_scrollback_line(parser);
                            let prompt_display = format!("{}{}\x1b[0m\r\n", theme.ansi_user_prompt, text);
                            parser.process(prompt_display.as_bytes());
                            state.tracker.record_prompt(
                                &text,
                                scrollback_line,
                                crate::ui::blocks::InputMode::Ai,
                            );
                            parser.process(b"\r\n");
                        }
                        ReplayItem::ShellCommand {
                            command,
                            output,
                            exit_code,
                        } => {
                            let scrollback_line = get_scrollback_line(parser);
                            let cmd_display = format!("{}{}\x1b[0m\r\n", theme.ansi_user_prompt, command);
                            parser.process(cmd_display.as_bytes());
                            // Show truncated output, converting bare \n to \r\n for vt100
                            let out_preview = if output.len() > 200 {
                                format!("{}...", &output[..200])
                            } else {
                                output.clone()
                            };
                            if !out_preview.is_empty() {
                                let normalized =
                                    out_preview.replace("\r\n", "\n").replace('\n', "\r\n");
                                parser.process(normalized.as_bytes());
                                if !out_preview.ends_with('\n') {
                                    parser.process(b"\r\n");
                                }
                            }
                            let color = if exit_code == 0 {
                                theme.ansi_exit_success
                            } else {
                                theme.ansi_exit_failure
                            };
                            parser.process(
                                format!("{}exit {}\x1b[0m\r\n", color, exit_code).as_bytes(),
                            );
                            let end = get_scrollback_line(parser);
                            let line_count = (end.saturating_sub(scrollback_line)) as u16;
                            state.tracker.record_cmd_response(
                                &command,
                                scrollback_line,
                                line_count,
                                exit_code,
                            );
                            parser.process(b"\r\n");
                        }
                        ReplayItem::SetAgentStreaming => {
                            replay_agent_streaming = true;
                        }
                        ReplayItem::SetThinkingDuration(d) => {
                            // Backdate thinking_start so finalize_thinking computes the stored duration
                            if replay_thinking_start.is_some() {
                                replay_thinking_start =
                                    Some(Instant::now() - Duration::from_secs_f32(d));
                            }
                        }
                        ReplayItem::Daemon(daemon_msg) => {
                            let mut ds = DaemonStreamState {
                                md_stream: &mut replay_md,
                                agent_streaming: &mut replay_agent_streaming,
                                last_daemon_timestamp: &mut replay_timestamp,
                                model_alias: &mut replay_model,
                                context_usage: &mut replay_context,
                                tracker: &mut state.tracker,
                                ai_start_scrollback: &mut replay_ai_start,
                                thinking_buffer: &mut replay_thinking_buffer,
                                thinking_start: &mut replay_thinking_start,
                                thinking_scrollback: &mut replay_thinking_scrollback,
                                pending_confirmation: &mut replay_pending_confirmation,
                                permission_mode: &mut replay_permission_mode,
                                last_permission_outcome: &mut replay_last_perm,
                                session_id: &mut replay_session_id,
                                session_name: &mut replay_session_name,
                                active_request_id: &mut replay_active_request_id,
                            };
                            handle_single_message(daemon_msg, parser, &mut ds, theme);
                        }
                    }
                }
            }
            // Final separator before returning to normal input
            parser.process(b"\r\n");
        }
        ipc_messages::DaemonMessage::ExecuteCommand {
            execution_id,
            command,
            timeout_ms,
        } => {
            // Queue AI-requested command for execution in the coprocess
            state.pending_ai_executions.push_back(
                super::state::PendingAiExecution {
                    execution_id,
                    command,
                    _timeout_ms: timeout_ms,
                },
            );
        }
        // All other messages delegate to the core handler
        other => {
            let mut ds = DaemonStreamState {
                md_stream: &mut state.md_stream,
                agent_streaming: &mut state.agent_streaming,
                last_daemon_timestamp: &mut state.last_daemon_timestamp,
                model_alias: &mut state.model_alias,
                context_usage: &mut state.context_usage,
                tracker: &mut state.tracker,
                ai_start_scrollback: &mut state.ai_start_scrollback,
                thinking_buffer: &mut state.thinking_buffer,
                thinking_start: &mut state.thinking_start,
                thinking_scrollback: &mut state.thinking_scrollback,
                pending_confirmation: &mut state.pending_confirmation,
                permission_mode: &mut state.permission_mode,
                last_permission_outcome: &mut state.last_permission_outcome,
                session_id: &mut state.session_id,
                session_name: &mut state.session_name,
                active_request_id: &mut state.active_request_id,
            };
            handle_single_message(other, parser, &mut ds, theme);
        }
    }
}
