use crossterm::event::KeyCode;

use crate::ipc::client::CodivdClient;
use crate::ipc::messages as ipc_messages;

use super::super::state::{PendingConfirmation, TerminalState};

fn redraw_options(conf: &PendingConfirmation, p: &mut vt100::Parser) {
    let is_native = matches!(
        conf.tool_name.as_str(),
        "read" | "write" | "edit" | "glob" | "grep"
    );
    let options: Vec<&str> = if is_native {
        vec!["1. Yes, allow this action", "2. No, reject"]
    } else if conf.risk == codiv_common::messages::RiskLevel::Critical {
        vec![
            "1. Yes, allow this action",
            "2. No, reject",
            "3. No, and never allow (permanent)",
        ]
    } else {
        vec![
            "1. Yes, allow this action",
            "2. Yes, and always allow (permanent)",
            "3. No, reject",
            "4. No, and never allow (permanent)",
        ]
    };
    // Move cursor up by option_count lines to overwrite them
    for _ in 0..conf.option_count {
        p.process(b"\x1b[A\r\x1b[K");
    }
    // Redraw each option line
    for (i, option) in options.iter().enumerate() {
        let (prefix, color) = if i == conf.selected_index {
            ("\u{203a}", "\x1b[1;37m") // › bold white
        } else {
            (" ", "\x1b[37m") // dim white
        };
        p.process(format!("    {}{} {}\x1b[0m\r\n", color, prefix, option).as_bytes());
    }
}

fn clear_prompt(conf: &PendingConfirmation, p: &mut vt100::Parser) {
    for _ in 0..conf.prompt_lines {
        p.process(b"\x1b[A\r\x1b[K");
    }
}

fn resolve_action(conf: &PendingConfirmation) -> (bool, bool, bool) {
    let is_native = matches!(
        conf.tool_name.as_str(),
        "read" | "write" | "edit" | "glob" | "grep"
    );
    if is_native {
        // Native tools: 0=allow, 1=reject (no permanent options)
        match conf.selected_index {
            0 => (true, false, false),
            1 => (false, false, false),
            _ => (false, false, false),
        }
    } else if conf.risk == codiv_common::messages::RiskLevel::Critical {
        // Critical: 0=allow, 1=reject, 2=never-allow(permanent)
        match conf.selected_index {
            0 => (true, false, false),
            1 => (false, false, false),
            2 => (false, false, true),
            _ => (false, false, false),
        }
    } else {
        // Non-critical bash/non-native: 0=allow, 1=always-allow(permanent), 2=reject, 3=never-allow(permanent)
        match conf.selected_index {
            0 => (true, false, false),
            1 => (true, true, false),
            2 => (false, false, false),
            3 => (false, false, true),
            _ => (false, false, false),
        }
    }
}

fn send_confirmation(
    conf: &PendingConfirmation,
    client: &mut Option<CodivdClient>,
) {
    let (granted, always, never) = resolve_action(conf);
    if let Some(ref mut c) = client {
        if let Some(frame) =
            ipc_messages::build_confirmation(&conf.request_id, granted, always, never, None)
        {
            c.send(&frame);
        }
    }
}

pub(crate) fn handle_confirmation(
    key_code: KeyCode,
    state: &mut TerminalState,
    parser: &mut vt100::Parser,
    client: &mut Option<CodivdClient>,
) -> bool {
    match key_code {
        KeyCode::Up => {
            if let Some(ref mut conf) = state.pending_confirmation {
                conf.selected_index = if conf.selected_index == 0 {
                    conf.option_count - 1
                } else {
                    conf.selected_index - 1
                };
                redraw_options(conf, parser);
            }
            true
        }
        KeyCode::Down => {
            if let Some(ref mut conf) = state.pending_confirmation {
                conf.selected_index = if conf.selected_index >= conf.option_count - 1 {
                    0
                } else {
                    conf.selected_index + 1
                };
                redraw_options(conf, parser);
            }
            true
        }
        KeyCode::Enter => {
            if let Some(conf) = state.pending_confirmation.take() {
                clear_prompt(&conf, parser);
                send_confirmation(&conf, client);
            }
            true
        }
        KeyCode::Char('1') => {
            if let Some(mut conf) = state.pending_confirmation.take() {
                conf.selected_index = 0;
                clear_prompt(&conf, parser);
                send_confirmation(&conf, client);
            }
            true
        }
        KeyCode::Char('2') => {
            if let Some(mut conf) = state.pending_confirmation.take() {
                conf.selected_index = 1;
                clear_prompt(&conf, parser);
                send_confirmation(&conf, client);
            }
            true
        }
        KeyCode::Char('3') => {
            if let Some(mut conf) = state.pending_confirmation.take() {
                conf.selected_index = 2;
                clear_prompt(&conf, parser);
                send_confirmation(&conf, client);
            }
            true
        }
        KeyCode::Char('4') => {
            // Only valid for non-critical (4 options)
            if let Some(ref conf) = state.pending_confirmation {
                if conf.risk != codiv_common::messages::RiskLevel::Critical {
                    if let Some(mut conf) = state.pending_confirmation.take() {
                        conf.selected_index = 3;
                        clear_prompt(&conf, parser);
                        send_confirmation(&conf, client);
                    }
                }
            }
            true
        }
        KeyCode::Esc => {
            // Reject on Escape
            if let Some(conf) = state.pending_confirmation.take() {
                clear_prompt(&conf, parser);
                if let Some(ref mut c) = client {
                    if let Some(frame) =
                        ipc_messages::build_confirmation(&conf.request_id, false, false, false, None)
                    {
                        c.send(&frame);
                    }
                }
            }
            true
        }
        _ => {
            true // consume all other keys while prompt is active
        }
    }
}
