use crossterm::event::KeyCode;

use crate::ipc::client::CodivdClient;
use crate::ipc::messages as ipc_messages;

use super::super::state::{PendingSessionPicker, TerminalState, VISIBLE_SESSIONS};
use super::super::utils::{parser_push_notice, NoticeKind};

fn redraw_picker(picker: &PendingSessionPicker, p: &mut vt100::Parser) {
    // Erase all picker lines (must match prompt_lines exactly)
    for _ in 0..picker.prompt_lines {
        p.process(b"\x1b[A\r\x1b[K");
    }
    // Re-render only the visible window of sessions
    let visible_count = picker.sessions.len().min(VISIBLE_SESSIONS);
    let end = picker.viewport_offset + visible_count;
    for i in picker.viewport_offset..end {
        let s = &picker.sessions[i];
        let name = s.name.as_deref().unwrap_or("(unnamed)");
        let time = codiv_common::conversation::relative_time(&s.updated_at);
        let (prefix, color) = if i == picker.selected_index {
            ("\u{203a}", "\x1b[1;37m")
        } else {
            (" ", "\x1b[37m")
        };
        let line = format!(
            "{}  {}[{}] {} \x1b[90m({})\x1b[0m\r\n",
            color,
            prefix,
            i + 1,
            name,
            time
        );
        p.process(line.as_bytes());
    }
    let scroll_hint = if picker.sessions.len() > VISIBLE_SESSIONS {
        " (\u{2191}\u{2193} to scroll)"
    } else {
        ""
    };
    let select_line = format!("\r\nSelect session or Esc to cancel:{}\r\n", scroll_hint);
    p.process(select_line.as_bytes());
}

pub(crate) fn handle_session_picker(
    key_code: KeyCode,
    state: &mut TerminalState,
    parser: &mut vt100::Parser,
    client: &mut Option<CodivdClient>,
) -> bool {
    match key_code {
        KeyCode::Up => {
            if let Some(ref mut picker) = state.pending_session_picker {
                if picker.selected_index > 0 {
                    picker.selected_index -= 1;
                    // Scroll viewport up if selection went above visible window
                    if picker.selected_index < picker.viewport_offset {
                        picker.viewport_offset = picker.selected_index;
                    }
                    redraw_picker(picker, parser);
                }
            }
            true
        }
        KeyCode::Down => {
            if let Some(ref mut picker) = state.pending_session_picker {
                if picker.selected_index < picker.sessions.len() - 1 {
                    picker.selected_index += 1;
                    // Scroll viewport down if selection went below visible window
                    let visible_count = picker.sessions.len().min(VISIBLE_SESSIONS);
                    if picker.selected_index >= picker.viewport_offset + visible_count {
                        picker.viewport_offset = picker.selected_index - visible_count + 1;
                    }
                    redraw_picker(picker, parser);
                }
            }
            true
        }
        KeyCode::Enter => {
            if let Some(picker) = state.pending_session_picker.take() {
                let sid = &picker.sessions[picker.selected_index].id;
                if let Some(ref mut c) = client {
                    if let Some(frame) = ipc_messages::build_load_session(sid) {
                        c.send(&frame);
                    }
                }
                // Clear the picker lines
                for _ in 0..picker.prompt_lines {
                    parser.process(b"\x1b[A\r\x1b[K");
                }
            }
            true
        }
        KeyCode::Esc => {
            if let Some(picker) = state.pending_session_picker.take() {
                for _ in 0..picker.prompt_lines {
                    parser.process(b"\x1b[A\r\x1b[K");
                }
                parser_push_notice(parser, NoticeKind::Notice, "Cancelled.");
            }
            true
        }
        _ => {
            true // consume all other keys
        }
    }
}
