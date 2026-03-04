use crate::ipc::messages as ipc_messages;
use crate::markdown::MarkdownStream;
use crate::ui::blocks::{BlockRegistry, ToolResultAction};

use super::utils::{get_scrollback_line, parser_push_styled};

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
) {
    match msg {
        ipc_messages::DaemonMessage::AgentStreamChunk {
            request_id: _,
            chunk,
        } => {
            match chunk {
                ipc_messages::StreamChunk::Text(t) => {
                    if ai_start_scrollback.is_none() {
                        *ai_start_scrollback = Some(get_scrollback_line(parser));
                    }
                    if let Some(ansi) = md_stream.push(&t) {
                        parser.process(&ansi);
                    }
                }
                ipc_messages::StreamChunk::Reasoning(t) => {
                    if ai_start_scrollback.is_none() {
                        *ai_start_scrollback = Some(get_scrollback_line(parser));
                    }
                    let t = t.replace('\n', "\r\n");
                    parser.process(t.as_bytes());
                }
                ipc_messages::StreamChunk::ToolCall { name, arguments } => {
                    // Flush any buffered markdown text so it appears before the tool call.
                    let pending = md_stream.finish();
                    if !pending.is_empty() {
                        parser.process(&pending);
                    }
                    md_stream.reset();
                    tracker.record_tool_call(&name, &arguments);
                }
                ipc_messages::StreamChunk::ToolResult { name, result } => {
                    {
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
