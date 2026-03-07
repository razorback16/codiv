use std::time::Instant;

use crate::ipc::messages as ipc_messages;
use crate::markdown::MarkdownStream;
use crate::ui::blocks::{canonical_tool_name, BlockRegistry, ToolResultAction};

use super::utils::{get_scrollback_line, parser_push_styled};

/// Finalize an in-progress thinking block: overwrite the placeholder line
/// with a "Thought for Ns" summary and register it in the block tracker.
fn finalize_thinking(
    parser: &mut vt100::Parser,
    tracker: &mut BlockRegistry,
    thinking_buffer: &mut String,
    thinking_start: &mut Option<Instant>,
    thinking_scrollback: &mut Option<u64>,
) {
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
            tracker.record_thinking(sl, content, duration_secs);
        }
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
                        finalize_thinking(parser, tracker, thinking_buffer, thinking_start, thinking_scrollback);
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
                        if !pending.is_empty() || had_ai_content {
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
                        finalize_thinking(parser, tracker, thinking_buffer, thinking_start, thinking_scrollback);
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
                        if !pending.is_empty() || had_ai_content {
                            parser.process(b"\r\n");
                        }
                    }

                    tracker.record_tool_call(&name, &arguments);
                    // Update header in-place with full args if we had a delta
                    if tracker.pending_tool().is_some() {
                        parser.process(b"\x1b[A\r\x1b[K");  // move up, clear header line
                        let args: serde_json::Value = serde_json::from_str(&arguments).unwrap_or(serde_json::Value::Null);
                        let header = crate::ui::blocks::build_tool_header(&name, &args);
                        let header_line = format!("\x1b[1m\x1b[33m{}\x1b[0m\r\n", header);
                        parser.process(header_line.as_bytes());
                    }
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
                                // Write 2-line block to VT100.
                                // Header: bold green ● ToolName(args)
                                let header_line = format!("\x1b[1m\x1b[32m{}\x1b[0m\r\n", header);
                                parser.process(header_line.as_bytes());
                                // Summary: colored based on tool
                                let is_bash_error = name.eq_ignore_ascii_case("bash")
                                    && !summary.contains("exit 0");
                                let color = if is_bash_error { "\x1b[31m" } else { "\x1b[32m" };
                                let summary_line = format!("{}{}\x1b[0m\r\n", color, summary);
                                parser.process(summary_line.as_bytes());
                                parser.process(b"\r\n"); // trailing separator after tool block
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
            risk: _,
        } => {
            parser_push_styled(
                parser,
                &format!("[daemon] confirm ({}): {}", request_id, description),
                "\x1b[33m",
            );
        }
        ipc_messages::DaemonMessage::Error {
            request_id,
            message,
        } => {
            *agent_streaming = false;
            md_stream.reset();
            parser_push_styled(parser, &format!("[daemon] error ({}): {}", request_id, message), "\x1b[31m");
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
    }
}
