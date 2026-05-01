use crossterm::event::KeyCode;

use super::super::state::TerminalState;

pub(crate) fn handle_completion_popup(
    key_code: KeyCode,
    state: &mut TerminalState,
) -> bool {
    if !state.input.completion_popup.is_visible() {
        return false;
    }

    match key_code {
        KeyCode::Up => {
            state.input.completion_popup.select_prev();
            true
        }
        KeyCode::Down => {
            state.input.completion_popup.select_next();
            true
        }
        KeyCode::Enter | KeyCode::Tab => {
            if let Some((selected, start, end)) = state.input.completion_popup.confirm() {
                let suffix = if std::path::Path::new(&selected).is_dir() {
                    "/"
                } else {
                    " "
                };
                state.input.line.replace_range(start, end, &format!("{}{}", selected, suffix));
            }
            true
        }
        KeyCode::Esc => {
            state.input.completion_popup.dismiss();
            true
        }
        _ => {
            state.input.completion_popup.dismiss();
            false
        }
    }
}
