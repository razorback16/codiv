use codiv_common::conversation::{ConversationEvent, SessionInfo};

use crate::markdown::MarkdownStream;
use crate::ui::blocks::ToolResultAction;
use crate::ui::theme::Theme;

use super::helpers::{accumulate_ai_lines, build_tool_rendered_lines};
use crate::ui::terminal::state::{PendingSessionPicker, TerminalState, TokenUsage};
use crate::ui::terminal::utils::{parser_push_notice, rerender_all, reset_screen, NoticeKind};

/// Handle the `SessionList` daemon message.
pub(super) fn handle_session_list(
    sessions: Vec<SessionInfo>,
    parser: &mut vt100::Parser,
    state: &mut TerminalState,
    theme: &Theme,
) {
    if sessions.is_empty() {
        parser_push_notice(parser, NoticeKind::Notice, "No saved sessions.");
    } else {
        use crate::ui::terminal::state::VISIBLE_SESSIONS;
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

/// Handle the `SessionReplay` daemon message.
pub(super) fn handle_session_replay(
    events: Vec<ConversationEvent>,
    parser: &mut vt100::Parser,
    state: &mut TerminalState,
    md_stream_width: u16,
    theme: &Theme,
) {
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

/// Handle the `CompactionStarted` daemon message.
pub(super) fn handle_compaction_started(
    request_id: String,
    compacted_event_count: usize,
    parser: &mut vt100::Parser,
    state: &mut TerminalState,
    theme: &Theme,
) {
    state.ui.prompt_is_live = false;
    state.ui.prompt_anchor_row = None;
    reset_screen(parser, &mut state.ui.scroll_offset, &mut state.tracker);

    // Show pending yellow header + placeholder summary (2-line tool block pattern)
    let scrollback_line = crate::ui::terminal::utils::get_scrollback_line(parser);
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

/// Handle the `CompactionComplete` daemon message.
pub(super) fn handle_compaction_complete(
    summary: String,
    compacted_event_count: usize,
    parser: &mut vt100::Parser,
    state: &mut TerminalState,
    theme: &Theme,
) {
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
