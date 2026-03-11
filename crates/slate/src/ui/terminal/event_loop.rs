use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use crate::markdown::MarkdownStream;
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::prelude::CrosstermBackend;
use ratatui::Terminal;

use crate::ipc::client::SlatedClient;
use crate::ipc::messages as ipc_messages;
use crate::shell::bash_coprocess::BashCoprocess;
use crate::shell::command_index::{classify_input, InputAction};
use crate::shell::completion_engine::CompletionEngine;

use crate::ui::blocks::{Block, BlockRegistry, InputMode};
use crate::ui::completion_popup::CompletionPopup;
use crate::ui::input::InputLine;
use crate::ui::selection::TextSelection;
use crate::ui::tool_modal::ToolResultModal;

use super::animation::AnimationState;
use super::daemon;
use super::input as terminal_input;
use super::io as terminal_io;
use super::io::TerminalColors;
use super::render::render_frame;
use slate_common::permissions::PermissionMode;

use super::state::{PendingCommand, PendingConfirmation, PendingSessionPicker, MAX_SCROLLBACK};
use super::utils::scroll_to_focused;
use super::utils::{get_scrollback_line, parser_push_notice, send_agent_request, NoticeKind};
use super::{parser_cols_from_term_width, parser_rows_from_term_height, PROMPT_GUTTER_WIDTH};

/// The main event loop. Factored out so cleanup always runs in `run()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn event_loop(
    term: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    parser: &mut vt100::Parser,
    scroll_offset: &mut usize,
    input: &mut InputLine,
    bash: &mut BashCoprocess,
    shutdown: &Arc<std::sync::atomic::AtomicBool>,
    cwd: &mut String,
    client: &mut Option<SlatedClient>,
    prompt_is_live: &mut bool,
    terminal_colors: &TerminalColors,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut last_heartbeat_sent = Instant::now();
    let mut last_daemon_timestamp: u64 = 0;
    let mut pending_command: Option<PendingCommand> = None;
    let mut completion_engine = CompletionEngine::new();
    let mut completion_popup = CompletionPopup::new();
    let mut selection = TextSelection::new();
    let mut clipboard = arboard::Clipboard::new().ok();
    let mut agent_streaming = false;
    let mut cmd_start_scrollback: Option<u64> = None;
    let mut ai_start_scrollback: Option<u64> = None;
    let mut thinking_buffer = String::new();
    let mut thinking_start: Option<Instant> = None;
    let mut thinking_scrollback: Option<u64> = None;
    let mut git_info = bash.capture_git_info();
    let mut cached_env_vars = bash.capture_env();
    let mut model_alias = String::new();
    let mut context_usage: (usize, usize) = (0, 0);
    let mut md_stream = MarkdownStream::new(
        term.size()
            .map(|s| parser_cols_from_term_width(s.width))
            .unwrap_or(80),
    );
    let mut tracker = BlockRegistry::new();
    let mut tool_result_modal = ToolResultModal::new();
    let mut was_alt_screen = false;
    let mut anim = AnimationState::new();
    let mut input_mode = InputMode::Ai;
    let mut thinking_enabled = false;
    let mut permission_mode = PermissionMode::default();
    let mut pending_confirmation: Option<PendingConfirmation> = None;
    let mut last_permission_outcome: Option<(String, bool, String)> = None; // (tool_name, granted, reason)
    let mut session_id: Option<String> = None;
    let mut session_name: Option<String> = None;
    let mut pending_session_picker: Option<PendingSessionPicker> = None;

    // Start background initialization (non-blocking) so the first Tab
    // press is fast without freezing the UI at startup.
    completion_engine.start_init(bash);

    // Spawn crossterm reader thread for channelized terminal input.
    let crossterm_rx = terminal_io::spawn_crossterm_reader();
    let tick_rx = terminal_io::spawn_tick_channel(super::animation::TICK_INTERVAL);

    let mut needs_render = true;

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let daemon_connected = client.as_ref().is_some_and(|c| c.is_connected());

        // Check for daemon disconnect.
        if !daemon_connected && client.is_some() {
            *client = None;
        }

        // --- Poll completion engine background init when the coprocess is free ---
        if pending_command.is_none() {
            completion_engine.poll_init(bash);
        }

        // --- Detect alternate-screen transitions (vim, etc.) ---
        let in_alt_screen = parser.screen().alternate_screen();
        if in_alt_screen != was_alt_screen {
            was_alt_screen = in_alt_screen;
            if let Ok(sz) = term.size() {
                let parser_rows = parser_rows_from_term_height(sz.height);
                let parser_cols = if in_alt_screen {
                    // Full width — no gutter margin for fullscreen apps.
                    sz.width.max(1)
                } else {
                    parser_cols_from_term_width(sz.width)
                };
                parser.screen_mut().set_size(parser_rows, parser_cols);
                bash.resize(parser_rows, parser_cols);
            }
            needs_render = true;
        }

        // --- Render only when needed ---
        if needs_render {
            let is_executing = pending_command.is_some();
            render_frame(
                term,
                parser,
                input,
                cwd,
                daemon_connected,
                last_daemon_timestamp,
                *scroll_offset,
                prompt_is_live,
                is_executing,
                agent_streaming,
                thinking_start.is_some(),
                &completion_popup,
                &selection,
                git_info.as_ref(),
                &model_alias,
                context_usage,
                &tracker,
                &tool_result_modal,
                &anim,
                input_mode,
                thinking_enabled,
                permission_mode,
                session_name.as_deref(),
            )?;
            needs_render = false;
        }

        // --- Compute timeout ---
        let timeout = terminal_io::compute_next_timeout(
            &pending_command,
            daemon_connected,
            last_heartbeat_sent,
            completion_engine.is_ready(),
        );

        // --- Select on all event sources ---
        // Only select on PTY when a command is pending. When idle, the
        // completion engine's poll_init() reads from the same channel via
        // try_read() to detect its sentinel — consuming PTY bytes here
        // would starve the completion engine and delay initialization.
        let no_pty: crossbeam_channel::Receiver<Vec<u8>> = crossbeam_channel::never();
        let pty_rx = if pending_command.is_some() {
            bash.pty_receiver()
        } else {
            &no_pty
        };
        let no_daemon: crossbeam_channel::Receiver<ipc_messages::DaemonMessage> =
            crossbeam_channel::never();
        let daemon_rx = client.as_ref().map_or(&no_daemon, |c| c.daemon_receiver());

        crossbeam_channel::select! {
            recv(pty_rx) -> msg => {
                if let Ok(bytes) = msg {
                    if let Some(ref mut pending) = pending_command {
                        terminal_io::process_pty_bytes(&bytes, pending, parser, bash, terminal_colors);
                    }
                    needs_render = true;
                }
            }
            recv(crossterm_rx) -> msg => {
                if let Ok(evt) = msg {
                    match &evt {
                        Event::Mouse(mouse) => {
                            match mouse.kind {
                                MouseEventKind::ScrollUp => {
                                    *scroll_offset = scroll_offset.saturating_add(3);
                                    parser.screen_mut().set_scrollback(*scroll_offset);
                                }
                                MouseEventKind::ScrollDown => {
                                    *scroll_offset = scroll_offset.saturating_sub(3);
                                    parser.screen_mut().set_scrollback(*scroll_offset);
                                }
                                MouseEventKind::Down(MouseButton::Left) => {
                                    selection.start_at(mouse.column, mouse.row);
                                }
                                MouseEventKind::Drag(MouseButton::Left) => {
                                    selection.update(mouse.column, mouse.row);
                                }
                                MouseEventKind::Up(MouseButton::Left) => {
                                    selection.finish();
                                    if selection.is_active() {
                                        let term_area_height =
                                            term.size().map(|s| parser_rows_from_term_height(s.height)).unwrap_or(1);
                                        let text = selection.extract_text(
                                            parser.screen(),
                                            0,
                                            term_area_height,
                                            PROMPT_GUTTER_WIDTH,
                                        );
                                        if !text.is_empty() {
                                            if let Some(ref mut cb) = clipboard {
                                                let _ = cb.set_text(text);
                                            }
                                        }
                                    }
                                }
                                _ => {
                                    selection.clear();
                                }
                            }
                        }
                        Event::Resize(cols, rows) => {
                            let parser_rows = parser_rows_from_term_height(*rows);
                            let parser_cols = if was_alt_screen {
                                (*cols).max(1)
                            } else {
                                parser_cols_from_term_width(*cols)
                            };
                            parser.screen_mut().set_size(parser_rows, parser_cols);
                            bash.resize(parser_rows, parser_cols);
                            md_stream.set_width(parser_cols);
                        }
                        _ => {}
                    }

                    // Process key events (only if not mouse/resize).
                    if let Event::Key(key) = evt {
                        if key.kind == KeyEventKind::Press {
                            // Clear text selection on any keypress.
                            selection.clear();

                            // --- Modal key interception (highest priority) ---
                            let mut key_handled = false;
                            if tool_result_modal.is_visible() {
                                match key.code {
                                    KeyCode::Esc => tool_result_modal.close(),
                                    KeyCode::Up => tool_result_modal.scroll_up(1),
                                    KeyCode::Down => tool_result_modal.scroll_down(1),
                                    KeyCode::PageUp => tool_result_modal.scroll_up(20),
                                    KeyCode::PageDown => tool_result_modal.scroll_down(20),
                                    _ => {} // consume everything else
                                }
                                key_handled = true;
                            }

                            // --- Session picker key interception ---
                            if pending_session_picker.is_some() && !key_handled {
                                let redraw_picker = |picker: &PendingSessionPicker, p: &mut vt100::Parser| {
                                    // Erase all picker lines (must match prompt_lines exactly)
                                    for _ in 0..picker.prompt_lines {
                                        p.process(b"\x1b[A\r\x1b[K");
                                    }
                                    // Re-render session entries
                                    for (i, s) in picker.sessions.iter().enumerate() {
                                        let name = s.name.as_deref().unwrap_or("(unnamed)");
                                        let time = slate_common::conversation::relative_time(&s.updated_at);
                                        let (prefix, color) = if i == picker.selected_index {
                                            ("\u{203a}", "\x1b[1;37m")
                                        } else {
                                            (" ", "\x1b[37m")
                                        };
                                        let line = format!("{}  {}[{}] {} \x1b[90m({})\x1b[0m\r\n", color, prefix, i + 1, name, time);
                                        p.process(line.as_bytes());
                                    }
                                    let select_line = format!("\r\nSelect session [1-{}] or Esc to cancel:\r\n", picker.sessions.len());
                                    p.process(select_line.as_bytes());
                                };

                                match key.code {
                                    KeyCode::Up => {
                                        if let Some(ref mut picker) = pending_session_picker {
                                            picker.selected_index = if picker.selected_index == 0 {
                                                picker.sessions.len() - 1
                                            } else {
                                                picker.selected_index - 1
                                            };
                                            redraw_picker(picker, parser);
                                        }
                                        key_handled = true;
                                    }
                                    KeyCode::Down => {
                                        if let Some(ref mut picker) = pending_session_picker {
                                            picker.selected_index = if picker.selected_index >= picker.sessions.len() - 1 {
                                                0
                                            } else {
                                                picker.selected_index + 1
                                            };
                                            redraw_picker(picker, parser);
                                        }
                                        key_handled = true;
                                    }
                                    KeyCode::Enter => {
                                        if let Some(picker) = pending_session_picker.take() {
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
                                        key_handled = true;
                                    }
                                    KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                                        let idx = (c as usize) - ('1' as usize);
                                        if let Some(ref picker) = pending_session_picker {
                                            if idx < picker.sessions.len() {
                                                let sid = picker.sessions[idx].id.clone();
                                                let prompt_lines = picker.prompt_lines;
                                                pending_session_picker = None;
                                                if let Some(ref mut c) = client {
                                                    if let Some(frame) = ipc_messages::build_load_session(&sid) {
                                                        c.send(&frame);
                                                    }
                                                }
                                                for _ in 0..prompt_lines {
                                                    parser.process(b"\x1b[A\r\x1b[K");
                                                }
                                            }
                                        }
                                        key_handled = true;
                                    }
                                    KeyCode::Esc => {
                                        if let Some(picker) = pending_session_picker.take() {
                                            for _ in 0..picker.prompt_lines {
                                                parser.process(b"\x1b[A\r\x1b[K");
                                            }
                                            parser_push_notice(parser, NoticeKind::Notice, "Cancelled.");
                                        }
                                        key_handled = true;
                                    }
                                    _ => {
                                        key_handled = true; // consume all other keys
                                    }
                                }
                            }

                            // --- Confirmation prompt key interception ---
                            if pending_confirmation.is_some() && !key_handled {
                                // Helper: build the options list for the current risk level
                                let redraw_options = |conf: &PendingConfirmation, p: &mut vt100::Parser| {
                                    let options: Vec<&str> = if conf.risk == slate_common::messages::RiskLevel::Critical {
                                        vec!["1. Yes, allow this action", "2. No, reject", "3. No, and never allow (session)"]
                                    } else {
                                        vec!["1. Yes, allow this action", "2. Yes, and always allow (session)", "3. No, reject", "4. No, and never allow (session)"]
                                    };
                                    // Move cursor up by option_count lines to overwrite them
                                    for _ in 0..conf.option_count {
                                        p.process(b"\x1b[A\r\x1b[K");
                                    }
                                    // Redraw each option line
                                    for (i, option) in options.iter().enumerate() {
                                        let (prefix, color) = if i == conf.selected_index {
                                            ("\u{203a}", "\x1b[1;37m")  // › bold white
                                        } else {
                                            (" ", "\x1b[37m")  // dim white
                                        };
                                        p.process(format!("    {}{} {}\x1b[0m\r\n", color, prefix, option).as_bytes());
                                    }
                                };

                                // Helper: clear all prompt lines (risk label + options)
                                let clear_prompt = |conf: &PendingConfirmation, p: &mut vt100::Parser| {
                                    for _ in 0..conf.prompt_lines {
                                        p.process(b"\x1b[A\r\x1b[K");
                                    }
                                };

                                // Helper: resolve selected_index to (granted, always, never) based on risk
                                let resolve_action = |conf: &PendingConfirmation| -> (bool, bool, bool) {
                                    if conf.risk == slate_common::messages::RiskLevel::Critical {
                                        // Critical: 0=allow, 1=reject, 2=never-allow
                                        match conf.selected_index {
                                            0 => (true, false, false),
                                            1 => (false, false, false),
                                            2 => (false, false, true),
                                            _ => (false, false, false),
                                        }
                                    } else {
                                        // Non-critical: 0=allow, 1=always-allow, 2=reject, 3=never-allow
                                        match conf.selected_index {
                                            0 => (true, false, false),
                                            1 => (true, true, false),
                                            2 => (false, false, false),
                                            3 => (false, false, true),
                                            _ => (false, false, false),
                                        }
                                    }
                                };

                                match key.code {
                                    KeyCode::Up => {
                                        if let Some(ref mut conf) = pending_confirmation {
                                            conf.selected_index = if conf.selected_index == 0 {
                                                conf.option_count - 1
                                            } else {
                                                conf.selected_index - 1
                                            };
                                            redraw_options(conf, parser);
                                        }
                                        key_handled = true;
                                    }
                                    KeyCode::Down => {
                                        if let Some(ref mut conf) = pending_confirmation {
                                            conf.selected_index = if conf.selected_index >= conf.option_count - 1 {
                                                0
                                            } else {
                                                conf.selected_index + 1
                                            };
                                            redraw_options(conf, parser);
                                        }
                                        key_handled = true;
                                    }
                                    KeyCode::Enter => {
                                        if let Some(conf) = pending_confirmation.take() {
                                            let (granted, always, never) = resolve_action(&conf);
                                            clear_prompt(&conf, parser);
                                            if let Some(ref mut c) = client {
                                                if let Some(frame) = ipc_messages::build_confirmation(
                                                    &conf.request_id, granted, always, never, None,
                                                ) {
                                                    c.send(&frame);
                                                }
                                            }
                                        }
                                        key_handled = true;
                                    }
                                    KeyCode::Char('1') => {
                                        if let Some(mut conf) = pending_confirmation.take() {
                                            conf.selected_index = 0;
                                            let (granted, always, never) = resolve_action(&conf);
                                            clear_prompt(&conf, parser);
                                            if let Some(ref mut c) = client {
                                                if let Some(frame) = ipc_messages::build_confirmation(
                                                    &conf.request_id, granted, always, never, None,
                                                ) {
                                                    c.send(&frame);
                                                }
                                            }
                                        }
                                        key_handled = true;
                                    }
                                    KeyCode::Char('2') => {
                                        if let Some(mut conf) = pending_confirmation.take() {
                                            conf.selected_index = 1;
                                            let (granted, always, never) = resolve_action(&conf);
                                            clear_prompt(&conf, parser);
                                            if let Some(ref mut c) = client {
                                                if let Some(frame) = ipc_messages::build_confirmation(
                                                    &conf.request_id, granted, always, never, None,
                                                ) {
                                                    c.send(&frame);
                                                }
                                            }
                                        }
                                        key_handled = true;
                                    }
                                    KeyCode::Char('3') => {
                                        if let Some(mut conf) = pending_confirmation.take() {
                                            conf.selected_index = 2;
                                            let (granted, always, never) = resolve_action(&conf);
                                            clear_prompt(&conf, parser);
                                            if let Some(ref mut c) = client {
                                                if let Some(frame) = ipc_messages::build_confirmation(
                                                    &conf.request_id, granted, always, never, None,
                                                ) {
                                                    c.send(&frame);
                                                }
                                            }
                                        }
                                        key_handled = true;
                                    }
                                    KeyCode::Char('4') => {
                                        // Only valid for non-critical (4 options)
                                        if let Some(ref conf) = pending_confirmation {
                                            if conf.risk != slate_common::messages::RiskLevel::Critical {
                                                if let Some(mut conf) = pending_confirmation.take() {
                                                    conf.selected_index = 3;
                                                    let (granted, always, never) = resolve_action(&conf);
                                                    clear_prompt(&conf, parser);
                                                    if let Some(ref mut c) = client {
                                                        if let Some(frame) = ipc_messages::build_confirmation(
                                                            &conf.request_id, granted, always, never, None,
                                                        ) {
                                                            c.send(&frame);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        key_handled = true;
                                    }
                                    KeyCode::Esc => {
                                        // Reject on Escape
                                        if let Some(conf) = pending_confirmation.take() {
                                            clear_prompt(&conf, parser);
                                            if let Some(ref mut c) = client {
                                                if let Some(frame) = ipc_messages::build_confirmation(
                                                    &conf.request_id, false, false, false, None,
                                                ) {
                                                    c.send(&frame);
                                                }
                                            }
                                        }
                                        key_handled = true;
                                    }
                                    _ => {
                                        key_handled = true; // consume all other keys while prompt is active
                                    }
                                }
                            }

                            // --- Forward keystrokes to PTY when a command is executing ---
                            if pending_command.is_some() {
                                // Ctrl+C falls through to the dedicated handler below.
                                if key.code == KeyCode::Char('c')
                                    && key.modifiers.contains(KeyModifiers::CONTROL)
                                {
                                    // fall through
                                } else if let Some(bytes) =
                                    terminal_input::key_event_to_bytes(key.code, key.modifiers)
                                {
                                    bash.send_bytes(&bytes);
                                    if let Some(ref mut p) = pending_command {
                                        p.last_activity = Instant::now();
                                    }
                                    key_handled = true;
                                } else {
                                    key_handled = true;
                                }
                            }

                            if !key_handled {
                                // --- Completion popup key interception ---
                                let mut popup_handled = false;
                                if completion_popup.is_visible() {
                                    match key.code {
                                        KeyCode::Up => {
                                            completion_popup.select_prev();
                                            popup_handled = true;
                                        }
                                        KeyCode::Down => {
                                            completion_popup.select_next();
                                            popup_handled = true;
                                        }
                                        KeyCode::Enter | KeyCode::Tab => {
                                            if let Some((selected, start, end)) =
                                                completion_popup.confirm()
                                            {
                                                let suffix = if std::path::Path::new(&selected).is_dir()
                                                {
                                                    "/"
                                                } else {
                                                    " "
                                                };
                                                input.replace_range(
                                                    start,
                                                    end,
                                                    &format!("{}{}", selected, suffix),
                                                );
                                            }
                                            popup_handled = true;
                                        }
                                        KeyCode::Esc => {
                                            completion_popup.dismiss();
                                            popup_handled = true;
                                        }
                                        _ => {
                                            completion_popup.dismiss();
                                        }
                                    }
                                }

                                if !popup_handled {
                                    // --- Block navigation (Shift+Up/Down) ---
                                    let mut block_handled = false;
                                    match (key.code, key.modifiers) {
                                        (KeyCode::Up, m) if m.contains(KeyModifiers::SHIFT) => {
                                            if tracker.focused_index().is_some() {
                                                tracker.focus_prev();
                                            } else {
                                                tracker.focus_last();
                                            }
                                            scroll_to_focused(parser, scroll_offset, &tracker);
                                            block_handled = true;
                                        }
                                        (KeyCode::Down, m) if m.contains(KeyModifiers::SHIFT) => {
                                            if tracker.focused_index().is_some() {
                                                tracker.focus_next();
                                            }
                                            scroll_to_focused(parser, scroll_offset, &tracker);
                                            block_handled = true;
                                        }
                                        (
                                            KeyCode::Enter,
                                            _,
                                        ) if tracker.focused_index().is_some() && pending_command.is_none() => {
                                            match tracker.focused() {
                                                Some(Block::Tool(tb)) => {
                                                    tool_result_modal.open(
                                                        &tb.header,
                                                        &tb.full_content,
                                                        tb.is_diff,
                                                        tb.id,
                                                    );
                                                }
                                                Some(Block::AiResponse(ab)) if ab.thinking_content.is_some() => {
                                                    let title = format!("Thought for {:.0}s", ab.thinking_duration_secs.unwrap_or(0.0));
                                                    tool_result_modal.open(&title, ab.thinking_content.as_deref().unwrap_or(""), false, ab.id);
                                                }
                                                _ => {}
                                            }
                                            block_handled = true;
                                        }
                                        (KeyCode::Esc, _) if tracker.focused_index().is_some() => {
                                            // Any block type: single Esc returns to live prompt
                                            tracker.unfocus();
                                            block_handled = true;
                                        }
                                        _ => {}
                                    }

                                    if !block_handled {
                                        match (key.code, key.modifiers) {
                                            // --- Ctrl combos ---
                                            (KeyCode::Char('c'), m)
                                                if m.contains(KeyModifiers::CONTROL) =>
                                            {
                                                if pending_command.is_some() {
                                                    bash.send_interrupt();
                                                    bash.drain_for(100);
                                                    pending_command = None;
                                                } else {
                                                    bash.send_interrupt();
                                                }
                                                parser.process(b"^C\r\n");
                                                *prompt_is_live = false;
                                                input.clear();
                                                *scroll_offset = 0;
                                                parser.screen_mut().set_scrollback(0);
                                            }

                                            (
                                                KeyCode::Char('d'),
                                                m,
                                            ) if m.contains(KeyModifiers::CONTROL)
                                                && input.content().is_empty() =>
                                            {
                                                break; // exit
                                            }

                                            // --- Ctrl+T: toggle thinking mode ---
                                            (KeyCode::Char('t'), m)
                                                if m.contains(KeyModifiers::CONTROL) =>
                                            {
                                                thinking_enabled = !thinking_enabled;
                                            }

                                            // --- Ctrl+P: cycle permission mode ---
                                            (KeyCode::Char('p'), m)
                                                if m.contains(KeyModifiers::CONTROL) =>
                                            {
                                                permission_mode = permission_mode.next();
                                                if let Some(ref mut c) = client {
                                                    if let Some(frame) =
                                                        ipc_messages::build_set_permission_mode(
                                                            permission_mode,
                                                        )
                                                    {
                                                        c.send(&frame);
                                                    }
                                                }
                                            }

                                            // --- Enter: submit input ---
                                            (KeyCode::Enter, _) if pending_command.is_none() => {
                                                let raw_input = input.submit();

                                                // Empty input — spring back, don't create a prompt block
                                                if raw_input.trim().is_empty() {
                                                    *scroll_offset = 0;
                                                    parser.screen_mut().set_scrollback(0);
                                                } else {
                                                    // Prompt text already on screen from last render_frame.
                                                    // Record position, then advance cursor past it.
                                                    let scrollback_line = get_scrollback_line(parser);
                                                    parser.process(b"\r\n\r\n");
                                                    tracker.record_prompt(&raw_input, scrollback_line, input_mode);
                                                    *prompt_is_live = false;
                                                    let action = classify_input(&raw_input);

                                                    match action {
                                                        InputAction::Empty => {}

                                                        InputAction::Exit => {
                                                            break;
                                                        }

                                                        InputAction::Clear => {
                                                            let term_size = term.size()?;
                                                            let rows = parser_rows_from_term_height(term_size.height);
                                                            let cols = parser_cols_from_term_width(term_size.width);
                                                            *parser = vt100::Parser::new(
                                                                rows,
                                                                cols,
                                                                MAX_SCROLLBACK,
                                                            );
                                                            *scroll_offset = 0;
                                                            *prompt_is_live = false;
                                                            tracker.clear();
                                                            tool_result_modal.close();
                                                        }

                                                        InputAction::Reset => {
                                                            let term_size = term.size()?;
                                                            let rows = parser_rows_from_term_height(term_size.height);
                                                            let cols = parser_cols_from_term_width(term_size.width);
                                                            *parser = vt100::Parser::new(
                                                                rows,
                                                                cols,
                                                                MAX_SCROLLBACK,
                                                            );
                                                            *scroll_offset = 0;
                                                            *prompt_is_live = false;
                                                            input.clear_history();
                                                            completion_engine = CompletionEngine::new();
                                                            completion_engine.start_init(bash);
                                                            completion_popup.dismiss();
                                                            selection.clear();
                                                            tracker.clear();
                                                            tool_result_modal.close();
                                                            parser_push_notice(
                                                                parser,
                                                                NoticeKind::Notice,
                                                                &format!(
                                                                    "slate v{} — type 'exit' to quit",
                                                                    crate::VERSION
                                                                ),
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
                                                            match input_mode {
                                                                InputMode::Command => {
                                                                    match bash.start_command(&raw_input) {
                                                                        Some(sentinel) => {
                                                                            cmd_start_scrollback = Some(get_scrollback_line(parser));
                                                                            pending_command = Some(PendingCommand {
                                                                                sentinel,
                                                                                accumulated: String::new(),
                                                                                command: raw_input.clone(),
                                                                                last_activity: Instant::now(),
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
                                                                        if send_agent_request(c, &raw_input, cwd, thinking_enabled, &cached_env_vars) {
                                                                            agent_streaming = true;
                                                                        }
                                                                    } else {
                                                                        parser_push_notice(
                                                                            parser,
                                                                            NoticeKind::Error,
                                                                            "AI mode not available (daemon not connected)",
                                                                        );
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }

                                                    *scroll_offset = 0;
                                                    parser.screen_mut().set_scrollback(0);
                                                }
                                            }

                                            // --- Tab: mode switch or completion ---
                                            (KeyCode::Tab, _) if pending_command.is_none() => {
                                                if input.content().is_empty() {
                                                    // Toggle mode
                                                    input_mode = match input_mode {
                                                        InputMode::Command => InputMode::Ai,
                                                        InputMode::Ai => InputMode::Command,
                                                    };
                                                } else if input_mode == InputMode::Command {
                                                    // Existing tab-completion logic
                                                    let line = input.content().to_string();
                                                    let cursor = input.cursor_byte_offset();
                                                    if let Some(result) =
                                                        completion_engine.complete(bash, &line, cursor)
                                                    {
                                                        match result.candidates.len() {
                                                            0 => {}
                                                            1 => {
                                                                let candidate = &result.candidates[0];
                                                                let suffix = if
                                                                    std::path::Path::new(candidate).is_dir()
                                                                {
                                                                    "/"
                                                                } else {
                                                                    " "
                                                                };
                                                                input.replace_range(
                                                                    result.replace_start,
                                                                    result.replace_end,
                                                                    &format!("{}{}", candidate, suffix),
                                                                );
                                                            }
                                                            _ => {
                                                                let common = terminal_input::longest_common_prefix(
                                                                    &result.candidates,
                                                                );
                                                                let prefix =
                                                                    &line[result.replace_start..result.replace_end];
                                                                if common.len() > prefix.len() {
                                                                    input.replace_range(
                                                                        result.replace_start,
                                                                        result.replace_end,
                                                                        &common,
                                                                    );
                                                                }
                                                                completion_popup.open(
                                                                    result.candidates,
                                                                    result.replace_start,
                                                                    result.replace_end,
                                                                );
                                                            }
                                                        }
                                                    }
                                                }
                                                // In AI mode with non-empty input: no-op
                                            }

                                            // --- History navigation ---
                                            (KeyCode::Up, _) => {
                                                input.history_up();
                                            }
                                            (KeyCode::Down, _) => {
                                                input.history_down();
                                            }

                                            // --- Cursor movement ---
                                            (KeyCode::Left, _) => {
                                                input.move_left();
                                            }
                                            (KeyCode::Right, _) => {
                                                input.move_right();
                                            }
                                            (KeyCode::Home, _) => {
                                                input.home();
                                            }
                                            (KeyCode::End, _) => {
                                                input.end();
                                            }

                                            // --- Editing ---
                                            (KeyCode::Backspace, _) => {
                                                input.backspace();
                                            }
                                            (KeyCode::Delete, _) => {
                                                input.delete();
                                            }

                                            // --- Scrolling ---
                                            (KeyCode::PageUp, _) => {
                                                *scroll_offset = scroll_offset.saturating_add(10);
                                                parser.screen_mut().set_scrollback(*scroll_offset);
                                            }
                                            (KeyCode::PageDown, _) => {
                                                *scroll_offset = scroll_offset.saturating_sub(10);
                                                parser.screen_mut().set_scrollback(*scroll_offset);
                                            }

                                            // --- Regular character input ---
                                            (KeyCode::Char(ch), modifiers) => {
                                                if modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                                                {
                                                    // Unhandled modifier combo — ignore.
                                                } else {
                                                    input.insert(ch);
                                                    *scroll_offset = 0;
                                                    parser.screen_mut().set_scrollback(0);
                                                }
                                            }

                                            _ => {}
                                        }
                                    } // if !block_handled
                                }
                            }
                        }
                    }

                    needs_render = true;
                }
            }
            recv(daemon_rx) -> msg => {
                if let Ok(msg) = msg {
                    daemon::handle_daemon_message(
                        msg,
                        parser,
                        &mut md_stream,
                        &mut agent_streaming,
                        &mut last_daemon_timestamp,
                        &mut model_alias,
                        &mut context_usage,
                        cwd,
                        &mut tracker,
                        &mut ai_start_scrollback,
                        &mut thinking_buffer,
                        &mut thinking_start,
                        &mut thinking_scrollback,
                        &mut pending_confirmation,
                        &mut permission_mode,
                        &mut last_permission_outcome,
                        &mut session_id,
                        &mut session_name,
                        &mut pending_session_picker,
                        scroll_offset,
                        term.size().map(|s| parser_cols_from_term_width(s.width)).unwrap_or(80),
                    );
                    // Drain any additional daemon messages that arrived.
                    while let Ok(msg2) = daemon_rx.try_recv() {
                        daemon::handle_daemon_message(
                            msg2,
                            parser,
                            &mut md_stream,
                            &mut agent_streaming,
                            &mut last_daemon_timestamp,
                            &mut model_alias,
                            &mut context_usage,
                            cwd,
                            &mut tracker,
                            &mut ai_start_scrollback,
                            &mut thinking_buffer,
                            &mut thinking_start,
                            &mut thinking_scrollback,
                            &mut pending_confirmation,
                            &mut permission_mode,
                            &mut last_permission_outcome,
                            &mut session_id,
                            &mut session_name,
                            &mut pending_session_picker,
                            scroll_offset,
                            term.size().map(|s| parser_cols_from_term_width(s.width)).unwrap_or(80),
                        );
                    }
                    needs_render = true;
                }
            }
            recv(tick_rx) -> _ => {
                anim.update_active(pending_command.is_some(), agent_streaming, thinking_start.is_some(), tracker.pending_tool().is_some());
                if anim.tick() {
                    needs_render = true;
                }
            }
            default(timeout) => {
                // Timer fired — handle periodic tasks.
                needs_render = true;
            }
        }

        // --- Post-select: drain additional PTY data ---
        if pending_command.is_some() {
            let drain: Vec<Vec<u8>> = {
                let pty_rx = bash.pty_receiver();
                let mut collected = Vec::new();
                while let Ok(bytes) = pty_rx.try_recv() {
                    collected.push(bytes);
                }
                collected
            };
            for bytes in &drain {
                if let Some(ref mut pending) = pending_command {
                    terminal_io::process_pty_bytes(bytes, pending, parser, bash, terminal_colors);
                }
            }
        }

        // --- Post-select: check command completion ---
        if let Some(ref mut pending) = pending_command {
            if let Some(result) = BashCoprocess::check_complete(
                &pending.accumulated,
                &pending.command,
                &pending.sentinel,
            ) {
                // Erase the sentinel result line from the parser display.
                parser.process(b"\x1b[A\x1b[2K");
                if result.exit_code != 0 {
                    parser_push_notice(
                        parser,
                        NoticeKind::Error,
                        &format!("exit code: {}", result.exit_code),
                    );
                }
                *cwd = bash.capture_cwd();
                git_info = bash.capture_git_info();
                cached_env_vars = bash.capture_env();
                if let Some(ref mut c) = client {
                    if let Some(frame) = ipc_messages::build_command_result(
                        &pending.command,
                        &result.output,
                        result.exit_code,
                        cwd,
                    ) {
                        c.send(&frame);
                    }
                }
                let cmd_end = get_scrollback_line(parser);
                if let Some(start) = cmd_start_scrollback.take() {
                    let line_count = (cmd_end.saturating_sub(start)) as u16;
                    if line_count > 0 {
                        tracker.record_cmd_response(
                            &pending.command,
                            start,
                            line_count,
                            result.exit_code,
                        );
                    }
                }
                parser.process(b"\r\n"); // blank separator after command output
                pending_command = None;
                *scroll_offset = 0;
                parser.screen_mut().set_scrollback(0);
                needs_render = true;
            } else if pending.last_activity.elapsed() > std::time::Duration::from_secs(300) {
                bash.send_interrupt();
                bash.drain_for(100);
                parser_push_notice(
                    parser,
                    NoticeKind::Error,
                    "command timed out (no activity for 5m)",
                );
                let cmd_end = get_scrollback_line(parser);
                if let Some(start) = cmd_start_scrollback.take() {
                    let line_count = (cmd_end.saturating_sub(start)) as u16;
                    if line_count > 0 {
                        tracker.record_cmd_response(&pending.command, start, line_count, -1);
                    }
                }
                parser.process(b"\r\n"); // blank separator after timed-out command
                pending_command = None;
                *scroll_offset = 0;
                parser.screen_mut().set_scrollback(0);
                needs_render = true;
            }
        }

        // --- Periodic heartbeat every 10s ---
        if let Some(ref mut c) = client {
            if c.is_connected()
                && last_heartbeat_sent.elapsed() > std::time::Duration::from_secs(10)
            {
                if let Some(hb) = ipc_messages::build_heartbeat() {
                    c.send(&hb);
                }
                last_heartbeat_sent = Instant::now();
            }
        }
    }

    Ok(())
}
