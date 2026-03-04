use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::prelude::CrosstermBackend;
use ratatui::Terminal;
use crate::markdown::MarkdownStream;

use crate::ipc::client::SlatedClient;
use crate::ipc::messages as ipc_messages;
use crate::shell::bash_coprocess::BashCoprocess;
use crate::shell::command_index::{classify_input, CommandIndex, InputAction};
use crate::shell::completion_engine::CompletionEngine;

use crate::ui::blocks::{Block, BlockRegistry};
use crate::ui::completion_popup::CompletionPopup;
use crate::ui::input::InputLine;
use crate::ui::selection::TextSelection;
use crate::ui::tool_modal::ToolResultModal;

use super::daemon;
use super::input as terminal_input;
use super::io as terminal_io;
use super::state::{PendingCommand, MAX_SCROLLBACK};
use super::utils::{get_scrollback_line, parser_push_styled, send_agent_request};
use super::utils::scroll_to_focused;
use super::render::render_frame;

/// The main event loop. Factored out so cleanup always runs in `run()`.
pub(crate) fn event_loop(
    term: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    parser: &mut vt100::Parser,
    scroll_offset: &mut usize,
    input: &mut InputLine,
    bash: &mut BashCoprocess,
    command_index: &CommandIndex,
    shutdown: &Arc<std::sync::atomic::AtomicBool>,
    cwd: &mut String,
    client: &mut Option<SlatedClient>,
    prompt_is_live: &mut bool,
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
    let mut git_info = bash.capture_git_info();
    let mut model_alias = String::new();
    let mut context_usage: (usize, usize) = (0, 0);
    let mut md_stream = MarkdownStream::new(
        term.size()
            .map(|s| s.width.saturating_sub(2).max(1))
            .unwrap_or(80),
    );
    let mut tracker = BlockRegistry::new();
    let mut tool_result_modal = ToolResultModal::new();

    // Start background initialization (non-blocking) so the first Tab
    // press is fast without freezing the UI at startup.
    completion_engine.start_init(bash);

    // Spawn crossterm reader thread for channelized terminal input.
    let crossterm_rx = terminal_io::spawn_crossterm_reader();

    let mut needs_render = true;

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let daemon_connected = client.as_ref().map_or(false, |c| c.is_connected());

        // Check for daemon disconnect.
        if !daemon_connected && client.is_some() {
            *client = None;
        }

        // --- Poll completion engine background init when the coprocess is free ---
        if pending_command.is_none() {
            completion_engine.poll_init(bash);
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
                &completion_popup,
                &selection,
                git_info.as_ref(),
                &model_alias,
                context_usage,
                &tracker,
                &tool_result_modal,
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
        let no_daemon: crossbeam_channel::Receiver<ipc_messages::DaemonMessage> = crossbeam_channel::never();
        let daemon_rx = client
            .as_ref()
            .map_or(&no_daemon, |c| c.daemon_receiver());

        crossbeam_channel::select! {
            recv(pty_rx) -> msg => {
                if let Ok(bytes) = msg {
                    if let Some(ref mut pending) = pending_command {
                        terminal_io::process_pty_bytes(&bytes, pending, parser);
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
                                            term.size().map(|s| s.height.saturating_sub(1).max(1)).unwrap_or(1);
                                        let text = selection.extract_text(
                                            parser.screen(),
                                            0,
                                            term_area_height,
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
                            let parser_rows = rows.saturating_sub(1).max(1);
                            let parser_cols = (*cols).saturating_sub(2).max(1);
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
                                            if let Some(Block::Tool(tb)) = tracker.focused() {
                                                tool_result_modal.open(
                                                    &tb.header,
                                                    &tb.full_content,
                                                    tb.is_diff,
                                                    tb.id,
                                                );
                                            }
                                            // PromptBlock: future — re-run
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
                                                    tracker.record_prompt(&raw_input, scrollback_line);
                                                    *prompt_is_live = false;
                                                    let action = classify_input(&raw_input, command_index);

                                                    match action {
                                                        InputAction::Empty => {}

                                                        InputAction::Exit => {
                                                            break;
                                                        }

                                                        InputAction::AiQuery => {
                                                            if let Some(ref mut c) = client {
                                                                let query = raw_input
                                                                    .trim_start()
                                                                    .strip_prefix('?')
                                                                    .unwrap_or(&raw_input);
                                                                if send_agent_request(c, query, &cwd) {
                                                                    agent_streaming = true;
                                                                }
                                                            } else {
                                                                parser_push_styled(
                                                                    parser,
                                                                    "AI mode not available (daemon not connected)",
                                                                    "\x1b[31m",
                                                                );
                                                            }
                                                        }

                                                        InputAction::Clear => {
                                                            let term_size = term.size()?;
                                                            let rows =
                                                                term_size.height.saturating_sub(1).max(1);
                                                            let cols =
                                                                term_size.width.saturating_sub(2).max(1);
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
                                                            let rows =
                                                                term_size.height.saturating_sub(1).max(1);
                                                            let cols =
                                                                term_size.width.saturating_sub(2).max(1);
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
                                                            parser_push_styled(
                                                                parser,
                                                                &format!(
                                                                    "slate v{} — type 'exit' to quit",
                                                                    crate::VERSION
                                                                ),
                                                                "\x1b[90m",
                                                            );
                                                            parser.process(b"\r\n\r\n"); // header line + blank line after
                                                        }

                                                        InputAction::Execute => {
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
                                                                    parser_push_styled(
                                                                        parser,
                                                                        "failed to send command to shell",
                                                                        "\x1b[31m",
                                                                    );
                                                                }
                                                            }
                                                        }

                                                        InputAction::NotFound(ref word) => {
                                                            if let Some(ref mut c) = client {
                                                                if send_agent_request(c, &raw_input, &cwd) {
                                                                    agent_streaming = true;
                                                                }
                                                            } else {
                                                                parser_push_styled(
                                                                    parser,
                                                                    &format!(
                                                                        "command not found: {}",
                                                                        word
                                                                    ),
                                                                    "\x1b[31m",
                                                                );
                                                            }
                                                        }
                                                    }

                                                    *scroll_offset = 0;
                                                    parser.screen_mut().set_scrollback(0);
                                                }
                                            }

                                            // --- Tab: completion ---
                                            (KeyCode::Tab, _) if pending_command.is_none() => {
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
                        );
                    }
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
            let pty_rx = bash.pty_receiver();
                    while let Ok(bytes) = pty_rx.try_recv() {
                if let Some(ref mut pending) = pending_command {
                    terminal_io::process_pty_bytes(&bytes, pending, parser);
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
                    parser_push_styled(
                        parser,
                        &format!("exit code: {}", result.exit_code),
                        "\x1b[31m",
                    );
                }
                *cwd = bash.capture_cwd();
                git_info = bash.capture_git_info();
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
                        tracker.record_cmd_response(&pending.command, start, line_count, result.exit_code);
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
                parser_push_styled(parser, "command timed out (no activity for 5m)", "\x1b[31m");
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
            if c.is_connected() && last_heartbeat_sent.elapsed() > std::time::Duration::from_secs(10) {
                if let Some(hb) = ipc_messages::build_heartbeat() {
                    c.send(&hb);
                }
                last_heartbeat_sent = Instant::now();
            }
        }
    }

    Ok(())
}
