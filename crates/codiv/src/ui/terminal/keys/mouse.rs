use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use super::super::state::TerminalState;
use super::super::{parser_rows_from_term_height, PROMPT_GUTTER_WIDTH};

pub(crate) fn handle_mouse(
    mouse: &MouseEvent,
    parser: &mut vt100::Parser,
    state: &mut TerminalState,
    term_size: Result<ratatui::layout::Size, std::io::Error>,
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
                    let content_left = PROMPT_GUTTER_WIDTH;
                    let click_row = mouse.row;
                    let click_col = mouse.column;
                    if click_row >= anchor && click_row <= prompt_end_row
                        && click_col >= content_left
                    {
                        let row = (click_row - anchor) as usize;
                        let col = (click_col - content_left) as usize;
                        state.input.set_cursor_to_row_col(row, col);
                    } else {
                        state.selection.start_at(mouse.column, mouse.row);
                    }
                } else {
                    state.selection.start_at(mouse.column, mouse.row);
                }
            } else {
                state.selection.start_at(mouse.column, mouse.row);
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            state.selection.update(mouse.column, mouse.row);
        }
        MouseEventKind::Up(MouseButton::Left) => {
            state.selection.finish();
            if state.selection.is_active() {
                let term_area_height =
                    term_size.as_ref().map(|s| parser_rows_from_term_height(s.height)).unwrap_or(1);
                let text = state.selection.extract_text(
                    parser.screen(),
                    0,
                    term_area_height,
                    PROMPT_GUTTER_WIDTH,
                );
                if !text.is_empty() {
                    if let Some(ref mut cb) = state.clipboard {
                        let _ = cb.set_text(text);
                    }
                }
            }
        }
        _ => {
            state.selection.clear();
        }
    }
}
