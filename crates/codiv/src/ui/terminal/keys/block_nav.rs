use crossterm::event::{KeyCode, KeyModifiers};

use crate::ui::blocks::Block;

use super::super::state::TerminalState;
use super::super::utils::scroll_to_focused;

pub(crate) fn handle_block_nav(
    key_code: KeyCode,
    modifiers: KeyModifiers,
    state: &mut TerminalState,
    parser: &mut vt100::Parser,
) -> bool {
    match (key_code, modifiers) {
        (KeyCode::Up, m) if m.contains(KeyModifiers::SHIFT) => {
            if state.tracker.focused_index().is_some() {
                state.tracker.focus_prev();
            } else {
                state.tracker.focus_last();
            }
            scroll_to_focused(parser, &mut state.ui.scroll_offset, &state.tracker);
            true
        }
        (KeyCode::Down, m) if m.contains(KeyModifiers::SHIFT) => {
            if state.tracker.focused_index().is_some() {
                state.tracker.focus_next();
            }
            scroll_to_focused(parser, &mut state.ui.scroll_offset, &state.tracker);
            true
        }
        (KeyCode::Enter, _)
            if state.tracker.focused_index().is_some() && state.cmd.pending_command.is_none() =>
        {
            match state.tracker.focused() {
                Some(Block::Tool(tb)) => {
                    state.tool_result_modal.open(&tb.header, &tb.full_content, tb.is_diff, tb.id);
                }
                Some(Block::Thinking(tk)) => {
                    let title = format!("Thought for {:.0}s", tk.duration_secs);
                    state.tool_result_modal.open(&title, &tk.content, false, tk.id);
                }
                _ => {}
            }
            true
        }
        (KeyCode::Esc, _) if state.tracker.focused_index().is_some() => {
            // Any block type: single Esc returns to live prompt
            state.tracker.unfocus();
            true
        }
        _ => false,
    }
}
