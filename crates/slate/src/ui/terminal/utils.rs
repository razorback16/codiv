use crate::ipc::client::SlatedClient;
use crate::ipc::messages as ipc_messages;

/// Send an agent request to the daemon. Returns `true` if the request was sent.
pub(crate) fn send_agent_request(
    client: &mut SlatedClient,
    query: &str,
    cwd: &str,
) -> bool {
    let request_id = format!("agent-{}", rand::random::<u64>());
    let context = ipc_messages::SessionContext {
        cwd: cwd.to_string(),
        recent_commands: Vec::new(),
        env_vars: Vec::new(),
    };
    if let Some(frame) = ipc_messages::build_agent_request(query, &request_id, context) {
        client.send(&frame);
        true
    } else {
        false
    }
}

/// Helper: write styled text to the vt100 parser using ANSI SGR codes.
pub(crate) fn parser_push_styled(parser: &mut vt100::Parser, text: &str, ansi_prefix: &str) {
    let line = format!("  {}{}\x1b[0m\r\n", ansi_prefix, text);
    parser.process(line.as_bytes());
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
    let scrollback_line = match focused {
        crate::ui::blocks::Block::Prompt(pb) => pb.scrollback_line,
        crate::ui::blocks::Block::Tool(tb) => tb.scrollback_line,
        crate::ui::blocks::Block::CmdResponse(cb) => cb.scrollback_line,
        crate::ui::blocks::Block::AiResponse(ab) => ab.scrollback_line,
        crate::ui::blocks::Block::Thinking(tb) => tb.scrollback_line,
    };
    let sb_len = true_scrollback_len(parser) as u64;
    let screen_rows = parser.screen().size().0 as u64;
    // abs_bottom at scroll_offset==0: sb_len + screen_rows
    // To bring scrollback_line to the middle of screen, we want:
    //   abs_top = scrollback_line - screen_rows/2
    //   scroll_offset = sb_len + screen_rows - screen_rows - abs_top
    //                 = sb_len - abs_top
    let half = screen_rows / 2;
    let desired_abs_top = scrollback_line.saturating_sub(half);
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
