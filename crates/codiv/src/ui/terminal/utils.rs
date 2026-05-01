use crate::ipc::client::CodivdClient;
use crate::ipc::messages as ipc_messages;

/// Send an agent request to the daemon. Returns the request_id if the request was sent.
pub(crate) fn send_agent_request(
    client: &mut CodivdClient,
    query: &str,
    cwd: &str,
    thinking: bool,
    env_vars: &[(String, String)],
) -> Option<String> {
    let request_id = format!("agent-{}", rand::random::<u64>());
    let context = ipc_messages::SessionContext {
        cwd: cwd.to_string(),
        recent_commands: Vec::new(),
        env_vars: env_vars.to_vec(),
    };
    if let Some(frame) = ipc_messages::build_agent_request(query, &request_id, context, thinking) {
        client.send(&frame);
        Some(request_id)
    } else {
        None
    }
}

pub(crate) enum NoticeKind {
    Error,
    Warning,
    Notice,
}

impl NoticeKind {
    fn ansi_prefix(&self) -> &'static str {
        match self {
            Self::Error => "\x1b[31m",
            Self::Warning => "\x1b[33m",
            Self::Notice => "\x1b[90m",
        }
    }
}

/// Reset the parser to a blank screen, clear scroll and block state, and re-emit the intro.
pub(crate) fn reset_screen(
    parser: &mut vt100::Parser,
    scroll_offset: &mut usize,
    tracker: &mut crate::ui::blocks::BlockRegistry,
) {
    let screen = parser.screen();
    let rows = screen.size().0;
    let cols = screen.size().1;
    *parser = vt100::Parser::new(rows, cols, super::state::MAX_SCROLLBACK);
    *scroll_offset = 0;
    tracker.clear();
    push_intro(parser);
}

/// Render the welcome header: "codiv v{VERSION} — type 'exit' to quit".
pub(crate) fn push_intro(parser: &mut vt100::Parser) {
    parser_push_notice(
        parser,
        NoticeKind::Notice,
        &format!("codiv v{} — type 'exit' to quit", crate::VERSION),
    );
}

/// Helper: write a terminal notice line with a blank separator after it.
pub(crate) fn parser_push_notice(parser: &mut vt100::Parser, kind: NoticeKind, text: &str) {
    let line = format!("  {}{}\x1b[0m\r\n\r\n", kind.ansi_prefix(), text);
    parser.process(line.as_bytes());
}

/// Finalize an in-progress thinking block: overwrite the placeholder line
/// with a "Thought for Ns" summary and register it in the block tracker.
/// Returns `true` if a thinking block was finalized.
pub(crate) fn finalize_thinking(
    parser: &mut vt100::Parser,
    thinking_start: &mut Option<std::time::Instant>,
    thinking_buffer: &mut String,
    thinking_scrollback: &mut Option<u64>,
    tracker: &mut crate::ui::blocks::BlockRegistry,
    ansi_thinking: &str,
) -> bool {
    if let Some(start) = thinking_start.take() {
        let duration_secs = start.elapsed().as_secs_f32();
        // Move cursor up one line and clear it (overwrite placeholder)
        parser.process(b"\x1b[A\r\x1b[K");
        let rendered_line = format!("{}Thought for {:.0}s\x1b[0m", ansi_thinking, duration_secs);
        parser.process(format!("{}\r\n", rendered_line).as_bytes());
        let content = std::mem::take(thinking_buffer);
        if let Some(sl) = thinking_scrollback.take() {
            tracker.record_thinking_block(content, duration_secs, sl, rendered_line);
        }
        true
    } else {
        false
    }
}

/// Write pre-rendered ANSI lines to the parser, each followed by `\r\n`.
pub(crate) fn write_block_lines_to_parser(parser: &mut vt100::Parser, lines: &[String]) {
    for line in lines {
        parser.process(line.as_bytes());
        parser.process(b"\r\n");
    }
}

/// Re-render all blocks from their stored content into a fresh parser.
/// Resets the parser, writes the intro, then writes each block's rendered content
/// with separators between them. Updates block positions via `recompute_positions()`.
pub(crate) fn rerender_all(
    parser: &mut vt100::Parser,
    tracker: &mut crate::ui::blocks::BlockRegistry,
    scroll_offset: &mut usize,
) {
    // Reset parser to blank state at current size
    let screen = parser.screen();
    let rows = screen.size().0;
    let cols = screen.size().1;
    *parser = vt100::Parser::new(rows, cols, super::state::MAX_SCROLLBACK);
    *scroll_offset = 0;

    // Write intro line
    push_intro(parser);

    // Compute base position (after intro)
    let base = get_scrollback_line(parser);

    // Write each block's content to the parser
    for block in tracker.blocks() {
        match block {
            crate::ui::blocks::Block::Prompt(b) => {
                write_block_lines_to_parser(parser, &b.rendered_lines);
            }
            crate::ui::blocks::Block::CmdResponse(b) => {
                if !b.raw_bytes.is_empty() {
                    parser.process(&b.raw_bytes);
                }
            }
            crate::ui::blocks::Block::AiResponse(b) => {
                write_block_lines_to_parser(parser, &b.rendered_lines);
            }
            crate::ui::blocks::Block::Tool(b) => {
                write_block_lines_to_parser(parser, &b.rendered_lines);
            }
            crate::ui::blocks::Block::Thinking(b) => {
                write_block_lines_to_parser(parser, &b.rendered_lines);
            }
        }
        // Separator between blocks
        parser.process(b"\r\n");
    }

    // Recompute positions
    tracker.recompute_positions(base);
}

/// Get the true scrollback buffer length by probing set_scrollback's clamping behaviour.
/// set_scrollback(usize::MAX) clamps to the actual VecDeque length, so reading it back
/// gives us the true number of lines currently in the scrollback buffer.
pub(crate) fn true_scrollback_len(parser: &mut vt100::Parser) -> usize {
    let saved = parser.screen().scrollback();
    parser.screen_mut().set_scrollback(usize::MAX);
    let len = parser.screen().scrollback();
    parser.screen_mut().set_scrollback(saved);
    len
}

/// Get the absolute line number of the cursor: true_scrollback_len + cursor_row.
pub(crate) fn get_scrollback_line(parser: &mut vt100::Parser) -> u64 {
    let (cursor_row, _) = parser.screen().cursor_position();
    true_scrollback_len(parser) as u64 + cursor_row as u64
}

/// Adjust scroll_offset so the focused block is visible, centering it if off-screen.
pub(crate) fn scroll_to_focused(
    parser: &mut vt100::Parser,
    scroll_offset: &mut usize,
    tracker: &crate::ui::blocks::BlockRegistry,
) {
    let focused = match tracker.focused() {
        Some(b) => b,
        None => {
            *scroll_offset = 0;
            parser.screen_mut().set_scrollback(0);
            return;
        }
    };
    let start_index = match focused {
        crate::ui::blocks::Block::Prompt(pb) => pb.start_index,
        crate::ui::blocks::Block::Tool(tb) => tb.start_index,
        crate::ui::blocks::Block::CmdResponse(cb) => cb.start_index,
        crate::ui::blocks::Block::AiResponse(ab) => ab.start_index,
        crate::ui::blocks::Block::Thinking(tk) => tk.start_index,
    };
    let sb_len = true_scrollback_len(parser) as u64;
    let screen_rows = parser.screen().size().0 as u64;
    // abs_bottom at scroll_offset==0: sb_len + screen_rows
    // To bring start_index to the middle of screen, we want:
    //   abs_top = start_index - screen_rows/2
    //   scroll_offset = sb_len + screen_rows - screen_rows - abs_top
    //                 = sb_len - abs_top
    let half = screen_rows / 2;
    let desired_abs_top = start_index.saturating_sub(half);
    // scroll_offset = how far from the natural bottom we are
    let natural_abs_top = sb_len; // abs_top when scroll_offset==0 is sb_len (cursor at bottom)
    let new_offset = if desired_abs_top >= natural_abs_top {
        0
    } else {
        (natural_abs_top - desired_abs_top) as usize
    };
    *scroll_offset = new_offset;
    parser.screen_mut().set_scrollback(*scroll_offset);
}

/// Format a token count for compact display: 0, 512, 1.2k, 200k, 1.0M.
pub(crate) fn format_tokens(n: usize) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let s = human_format::Formatter::new()
        .with_decimals(1)
        .with_separator("")
        .format(n as f64);
    // "200.0k" -> "200k", "1.2k" unchanged
    s.trim().replace(".0", "")
}

#[cfg(test)]
mod tests {
    use super::{parser_push_notice, rerender_all, NoticeKind};

    fn screen_line(parser: &vt100::Parser, row: u16, cols: u16) -> String {
        let mut line = String::new();
        for col in 0..cols {
            if let Some(cell) = parser.screen().cell(row, col) {
                line.push(cell.contents().chars().next().unwrap_or(' '));
            }
        }
        line.trim_end().to_string()
    }

    #[test]
    fn notice_kind_uses_expected_colors() {
        assert_eq!(NoticeKind::Error.ansi_prefix(), "\x1b[31m");
        assert_eq!(NoticeKind::Warning.ansi_prefix(), "\x1b[33m");
        assert_eq!(NoticeKind::Notice.ansi_prefix(), "\x1b[90m");
    }

    #[test]
    fn parser_push_notice_adds_blank_separator_line() {
        let mut parser = vt100::Parser::new(4, 40, 0);

        parser_push_notice(&mut parser, NoticeKind::Error, "boom");

        assert_eq!(screen_line(&parser, 0, 40), "  boom");
        assert_eq!(screen_line(&parser, 1, 40), "");
        assert_eq!(parser.screen().cursor_position(), (2, 0));
    }

    #[test]
    fn rerender_all_writes_blocks_and_recomputes_positions() {
        use crate::ui::blocks::{BlockRegistry, InputMode};

        let mut parser = vt100::Parser::new(40, 80, 1000);
        let mut tracker = BlockRegistry::new();
        let mut scroll_offset: usize = 0;

        // Add a prompt block
        tracker.record_prompt(
            "hello",
            0,
            InputMode::Ai,
            vec!["\x1b[1mhello\x1b[0m".to_string()],
        );

        // Add a thinking block
        tracker.record_thinking_block(
            "pondering...".to_string(),
            2.0,
            0,
            "\x1b[90mThought for 2s\x1b[0m".to_string(),
        );

        // Add a tool block with rendered lines
        tracker.record_tool_result("Read", "content here", 0);
        tracker.set_last_tool_rendered_lines(vec![
            "\x1b[32mRead(foo.rs)\x1b[0m".to_string(),
            "\x1b[90m  └ Read 1 lines\x1b[0m".to_string(),
        ]);

        // Re-render everything
        rerender_all(&mut parser, &mut tracker, &mut scroll_offset);

        // Verify intro was written
        assert_eq!(screen_line(&parser, 0, 80).trim(), format!("codiv v{} — type 'exit' to quit", crate::VERSION));

        // Verify blocks are present in the parser output
        // (exact content depends on ANSI stripping, but we verify positions are sequential)
        let blocks = tracker.blocks();
        assert_eq!(blocks.len(), 3);

        // Positions should be monotonically increasing
        let positions: Vec<u64> = blocks.iter().map(|b| match b {
            crate::ui::blocks::Block::Prompt(p) => p.start_index,
            crate::ui::blocks::Block::Thinking(t) => t.start_index,
            crate::ui::blocks::Block::Tool(t) => t.start_index,
            crate::ui::blocks::Block::AiResponse(a) => a.start_index,
            crate::ui::blocks::Block::CmdResponse(c) => c.start_index,
        }).collect();
        for i in 1..positions.len() {
            assert!(positions[i] > positions[i - 1], "positions should increase: {:?}", positions);
        }

        assert_eq!(scroll_offset, 0);
    }
}
