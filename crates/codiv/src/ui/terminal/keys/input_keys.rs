use std::time::Instant;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::prelude::CrosstermBackend;
use ratatui::Terminal;

use crate::ipc::client::CodivdClient;
use crate::ipc::daemon_launcher;
use crate::ipc::messages as ipc_messages;
use crate::shell::bash_coprocess::BashCoprocess;
use crate::shell::command_index::{classify_input, complete_slash_command, InputAction};
use crate::shell::completion_engine::CompletionEngine;
use crate::ui::blocks::InputMode;

use super::super::input as terminal_input;
use super::super::render::HINT_INPUT_THRESHOLD;
use super::super::state::{PendingCommand, TerminalState, MAX_SCROLLBACK};
use super::super::utils::{
    get_scrollback_line, parser_push_notice, reset_screen, send_agent_request, true_scrollback_len,
    NoticeKind,
};
use super::super::{parser_cols_from_term_width, parser_rows_from_term_height};

/// Result of handling an input key.
pub(crate) enum InputKeyResult {
    /// Key handled normally; continue the event loop.
    Continue,
    /// The user wants to exit (typed "exit" or Ctrl+D).
    Exit,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_input_keys(
    key_code: KeyCode,
    modifiers: KeyModifiers,
    term: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    parser: &mut vt100::Parser,
    bash: &mut BashCoprocess,
    client: &mut Option<CodivdClient>,
    state: &mut TerminalState,
) -> InputKeyResult {
    match (key_code, modifiers) {
        // --- Ctrl combos ---
        (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => {
            if state.pending_command.is_some() {
                bash.send_interrupt();
                bash.drain_for(100);
                state.pending_command = None;
            } else {
                bash.send_interrupt();
            }
            parser.process(b"^C\r\n");
            state.prompt_is_live = false;
            state.prompt_anchor_row = None;
            state.input.clear();
            state.scroll_offset = 0;
            parser.screen_mut().set_scrollback(0);
        }

        (KeyCode::Char('d'), m)
            if m.contains(KeyModifiers::CONTROL) && state.input.content().is_empty() =>
        {
            return InputKeyResult::Exit;
        }

        // --- Ctrl+T: toggle thinking mode ---
        (KeyCode::Char('t'), m) if m.contains(KeyModifiers::CONTROL) => {
            state.thinking_enabled = !state.thinking_enabled;
            state.hint_shown_at = Some(Instant::now());
            state.hint_seed = state.hint_seed.wrapping_mul(1103515245).wrapping_add(12345);
        }

        // --- Shift+Tab (BackTab): cycle permission mode ---
        (KeyCode::BackTab, _) => {
            state.permission_mode = state.permission_mode.next();
            state.hint_shown_at = Some(Instant::now());
            state.hint_seed = std::time::Instant::now().elapsed().subsec_nanos();
            if let Some(ref mut c) = client {
                if let Some(frame) = ipc_messages::build_set_permission_mode(state.permission_mode) {
                    c.send(&frame);
                }
            }
        }

        // --- Shift+Enter: insert newline (AI mode) ---
        // Terminals send Shift+Enter as Ctrl+J (linefeed).
        (KeyCode::Char('j'), m)
            if m.contains(KeyModifiers::CONTROL)
                && state.input_mode == InputMode::Ai
                && state.pending_command.is_none() =>
        {
            state.input.insert_newline();
            state.scroll_offset = 0;
            parser.screen_mut().set_scrollback(0);
        }

        // --- Enter: submit input ---
        (KeyCode::Enter, _) if state.pending_command.is_none() => {
            let raw_input = state.input.submit();

            // Empty input — spring back, don't create a prompt block
            if raw_input.trim().is_empty() {
                state.scroll_offset = 0;
                parser.screen_mut().set_scrollback(0);
            } else {
                // Prompt text already on screen from last render_frame.
                // Use the anchor row to compute the correct start position.
                let line_count = raw_input.split('\n').count().max(1);

                // Compute scrollback_line from the anchor (first row of prompt).
                let scrollback_line = if let Some(anchor) = state.prompt_anchor_row {
                    let sb_len = true_scrollback_len(parser) as u64;
                    sb_len + anchor as u64
                } else {
                    get_scrollback_line(parser)
                };

                // Move VT100 cursor to just past the last prompt line, then advance.
                if let Some(anchor) = state.prompt_anchor_row {
                    let last_prompt_row = anchor as u16 + line_count as u16;
                    // Position cursor at the row after the last prompt line
                    parser.process(
                        format!("\x1b[{};1H", last_prompt_row + 1).as_bytes(),
                    );
                }
                // One blank line separator after the prompt
                parser.process(b"\r\n");

                // Build rendered lines for the prompt block.
                let rendered_lines: Vec<String> = raw_input
                    .split('\n')
                    .map(|line| format!("\x1b[1m{}\x1b[0m", line))
                    .collect();
                state.tracker.record_prompt(&raw_input, scrollback_line, state.input_mode, rendered_lines);
                state.prompt_is_live = false;
                state.prompt_anchor_row = None;
                let action = classify_input(&raw_input);

                match action {
                    InputAction::Empty => {}

                    InputAction::Exit => {
                        return InputKeyResult::Exit;
                    }

                    InputAction::Clear => {
                        reset_screen(parser, &mut state.scroll_offset, &mut state.tracker);
                        state.prompt_is_live = false;
                        state.tool_result_modal.close();

                        // Tell the daemon to start a fresh session
                        if let Some(ref mut c) = client {
                            if let Some(frame) = ipc_messages::build_new_session() {
                                c.send(&frame);
                            }
                        }
                    }

                    InputAction::Reset => {
                        let term_size = term
                            .size()
                            .expect("failed to get terminal size during reset");
                        let rows = parser_rows_from_term_height(term_size.height);
                        let cols = parser_cols_from_term_width(term_size.width);
                        *parser = vt100::Parser::new(rows, cols, MAX_SCROLLBACK);
                        state.scroll_offset = 0;
                        state.prompt_is_live = false;
                        state.input.clear_history();
                        state.completion_engine = CompletionEngine::new();
                        state.completion_engine.start_init(bash);
                        state.completion_popup.dismiss();
                        state.tracker.clear();
                        state.tool_result_modal.close();
                        parser_push_notice(
                            parser,
                            NoticeKind::Notice,
                            &format!("codiv v{} — type 'exit' to quit", crate::VERSION),
                        );
                    }

                    InputAction::Sessions => {
                        if let Some(ref mut c) = client {
                            if let Some(frame) = ipc_messages::build_list_sessions() {
                                c.send(&frame);
                            }
                        } else {
                            parser_push_notice(
                                parser,
                                NoticeKind::Error,
                                "Sessions not available (daemon not connected)",
                            );
                        }
                    }

                    InputAction::UnknownCommand(ref cmd) => {
                        parser_push_notice(
                            parser,
                            NoticeKind::Error,
                            &format!("Unknown command: {}", cmd),
                        );
                    }

                    InputAction::Submit => {
                        match state.input_mode {
                            InputMode::Command => {
                                match bash.start_command(&raw_input) {
                                    Some(sentinel) => {
                                        state.cmd_start_scrollback =
                                            Some(get_scrollback_line(parser));
                                        state.cmd_output_capture.clear();
                                        state.pending_command = Some(PendingCommand {
                                            sentinel,
                                            accumulated: String::new(),
                                            command: raw_input.clone(),
                                            last_activity: Instant::now(),
                                            needs_env_refresh:
                                                super::super::state::command_modifies_env(
                                                    &raw_input,
                                                ),
                                            ai_execution_id: None,
                                        });
                                    }
                                    None => {
                                        parser_push_notice(
                                            parser,
                                            NoticeKind::Error,
                                            "failed to send command to shell",
                                        );
                                    }
                                }
                            }
                            InputMode::Ai => {
                                if let Some(ref mut c) = client {
                                    if let Some(rid) = send_agent_request(
                                        c,
                                        &raw_input,
                                        &state.cwd,
                                        state.thinking_enabled,
                                        &state.cached_env_vars,
                                    ) {
                                        state.agent_streaming = true;
                                        state.active_request_id = Some(rid);
                                    }
                                } else {
                                    let hint = daemon_launcher::daemon_start_hint();
                                    parser_push_notice(
                                        parser,
                                        NoticeKind::Error,
                                        &format!(
                                            "AI mode not available — daemon is not running. {}",
                                            hint
                                        ),
                                    );
                                }
                            }
                        }
                    }
                }

                state.scroll_offset = 0;
                parser.screen_mut().set_scrollback(0);
            }
        }

        // --- Tab: mode switch or completion ---
        (KeyCode::Tab, _) if state.pending_command.is_none() => {
            if state.input.content().is_empty() {
                // Toggle mode
                state.input_mode = match state.input_mode {
                    InputMode::Command => {
                        if client.is_none() {
                            let hint = daemon_launcher::daemon_start_hint();
                            parser_push_notice(
                                parser,
                                NoticeKind::Error,
                                &format!(
                                    "AI mode not available — daemon is not running. {}",
                                    hint
                                ),
                            );
                        }
                        InputMode::Ai
                    }
                    InputMode::Ai => InputMode::Command,
                };
                state.hint_shown_at = Some(Instant::now());
                state.hint_seed = state.hint_seed.wrapping_mul(1103515245).wrapping_add(12345);
            } else if state.input_mode == InputMode::Command {
                // Existing tab-completion logic
                let line = state.input.content().to_string();
                let cursor = state.input.cursor_byte_offset();
                if let Some(result) = state.completion_engine.complete(bash, &line, cursor) {
                    match result.candidates.len() {
                        0 => {}
                        1 => {
                            let candidate = &result.candidates[0];
                            let suffix =
                                if std::path::Path::new(candidate).is_dir() {
                                    "/"
                                } else {
                                    " "
                                };
                            state.input.replace_range(
                                result.replace_start,
                                result.replace_end,
                                &format!("{}{}", candidate, suffix),
                            );
                        }
                        _ => {
                            let common =
                                terminal_input::longest_common_prefix(&result.candidates);
                            let prefix = &line[result.replace_start..result.replace_end];
                            if common.len() > prefix.len() {
                                state.input.replace_range(
                                    result.replace_start,
                                    result.replace_end,
                                    &common,
                                );
                            }
                            state.completion_popup.open(
                                result.candidates,
                                result.replace_start,
                                result.replace_end,
                            );
                        }
                    }
                }
            } else if state.input_mode == InputMode::Ai {
                let line = state.input.content().to_string();
                if line.starts_with('/') {
                    let candidates = complete_slash_command(&line);
                    match candidates.len() {
                        0 => {}
                        1 => {
                            state.input.replace_range(
                                0,
                                line.len(),
                                &format!("{} ", candidates[0]),
                            );
                        }
                        _ => {
                            let common = terminal_input::longest_common_prefix(&candidates);
                            if common.len() > line.len() {
                                state.input.replace_range(0, line.len(), &common);
                            }
                            state.completion_popup.open(candidates, 0, line.len());
                        }
                    }
                }
            }
        }

        // --- History navigation ---
        (KeyCode::Up, _) => {
            if state.input_mode == InputMode::Ai && !state.input.move_up() {
                state.input.history_up();
            } else if state.input_mode != InputMode::Ai {
                state.input.history_up();
            }
        }
        (KeyCode::Down, _) => {
            if state.input_mode == InputMode::Ai && !state.input.move_down() {
                state.input.history_down();
            } else if state.input_mode != InputMode::Ai {
                state.input.history_down();
            }
        }

        // --- Cursor movement ---
        (KeyCode::Left, _) => {
            state.input.move_left();
        }
        (KeyCode::Right, _) => {
            state.input.move_right();
        }
        (KeyCode::Home, _) => {
            state.input.home();
        }
        (KeyCode::End, _) => {
            state.input.end();
        }

        // --- Editing ---
        (KeyCode::Backspace, _) => {
            state.input.backspace();
        }
        (KeyCode::Delete, _) => {
            state.input.delete();
        }

        // --- Scrolling ---
        (KeyCode::PageUp, _) => {
            state.scroll_offset = state.scroll_offset.saturating_add(10);
            parser.screen_mut().set_scrollback(state.scroll_offset);
            state.scroll_offset = parser.screen().scrollback();
        }
        (KeyCode::PageDown, _) => {
            state.scroll_offset = state.scroll_offset.saturating_sub(10);
            parser.screen_mut().set_scrollback(state.scroll_offset);
        }

        // --- Regular character input ---
        (KeyCode::Char(ch), modifiers) => {
            if modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
                // Unhandled modifier combo — ignore.
            } else if ch == '!' && state.input.content().is_empty() && state.input_mode == InputMode::Ai {
                // Quick switch: '!' on empty input switches to Command mode
                state.input_mode = InputMode::Command;
                state.hint_shown_at = Some(Instant::now());
                state.hint_seed = state.hint_seed.wrapping_mul(1103515245).wrapping_add(12345);
            } else if ch == '?' && state.input.content().is_empty() && state.input_mode == InputMode::Command {
                // Quick switch: '?' on empty input switches to AI mode
                if client.is_none() {
                    let hint = daemon_launcher::daemon_start_hint();
                    parser_push_notice(
                        parser,
                        NoticeKind::Error,
                        &format!(
                            "AI mode not available — daemon is not running. {}",
                            hint
                        ),
                    );
                }
                state.input_mode = InputMode::Ai;
                state.hint_shown_at = Some(Instant::now());
                state.hint_seed = state.hint_seed.wrapping_mul(1103515245).wrapping_add(12345);
            } else {
                let was_below = state.input.content().len() < HINT_INPUT_THRESHOLD;
                state.input.insert(ch);
                if was_below && state.input.content().len() >= HINT_INPUT_THRESHOLD {
                    state.hint_shown_at = Some(Instant::now());
                    state.hint_seed = state.hint_seed.wrapping_mul(1103515245).wrapping_add(12345);
                }
                state.scroll_offset = 0;
                parser.screen_mut().set_scrollback(0);
            }
        }

        _ => {}
    }

    InputKeyResult::Continue
}
