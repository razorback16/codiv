use std::time::Instant;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use super::super::state::TerminalState;

pub(crate) fn handle_mouse(
    mouse: &MouseEvent,
    parser: &mut vt100::Parser,
    state: &mut TerminalState,
    _term_size: Result<ratatui::layout::Size, std::io::Error>,
) {
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            state.scroll_offset = state.scroll_offset.saturating_add(3);
            parser.screen_mut().set_scrollback(state.scroll_offset);
            state.scroll_offset = parser.screen().scrollback();
        }
        MouseEventKind::ScrollDown => {
            state.scroll_offset = state.scroll_offset.saturating_sub(3);
            parser.screen_mut().set_scrollback(state.scroll_offset);
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if state.prompt_is_live && state.scroll_offset == 0 {
                if let Some(anchor) = state.prompt_anchor_row {
                    let prompt_line_count = state.input.line_count() as u16;
                    let prompt_end_row = anchor + prompt_line_count - 1;
                    let content_left = super::super::PROMPT_GUTTER_WIDTH;
                    let click_row = mouse.row;
                    let click_col = mouse.column;
                    if click_row >= anchor && click_row <= prompt_end_row
                        && click_col >= content_left
                    {
                        let row = (click_row - anchor) as usize;
                        let col = (click_col - content_left) as usize;
                        state.input.set_cursor_to_row_col(row, col);
                    }
                }
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            state.last_mouse_drag = Some(Instant::now());
        }
        _ => {}
    }
}
