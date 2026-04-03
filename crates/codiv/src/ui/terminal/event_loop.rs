use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::prelude::CrosstermBackend;
use ratatui::Terminal;

use crate::ipc::client::CodivdClient;
use crate::ipc::daemon_launcher;
use crate::ipc::messages as ipc_messages;
use crate::shell::bash_coprocess::BashCoprocess;

use super::daemon;
use super::input as terminal_input;
use super::io as terminal_io;
use super::render::{current_hint, render_frame};
use super::state::TerminalState;
use crate::ui::theme::Theme;

use super::utils::{finalize_thinking, get_scrollback_line, parser_push_notice, rerender_all, reset_screen, NoticeKind};
use super::{parser_cols_from_term_width, parser_rows_from_term_height};

/// The main event loop. Factored out so cleanup always runs in `run()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn event_loop(
    term: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    parser: &mut vt100::Parser,
    bash: &mut BashCoprocess,
    shutdown: &Arc<std::sync::atomic::AtomicBool>,
    client: &mut Option<CodivdClient>,
    state: &mut TerminalState,
    theme: &Theme,
) -> Result<(), Box<dyn std::error::Error>> {
    // Start background initialization (non-blocking) so the first Tab
    // press is fast without freezing the UI at startup.
    state.completion_engine.start_init(bash);

    // Spawn crossterm reader thread for channelized terminal input.
    let crossterm_rx = terminal_io::spawn_crossterm_reader();
    let tick_rx = terminal_io::spawn_tick_channel(super::animation::TICK_INTERVAL);

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let daemon_connected = client.as_ref().is_some_and(|c| c.is_connected());

        // Check for daemon disconnect.
        if !daemon_connected && client.is_some() {
            *client = None;
            parser_push_notice(parser, NoticeKind::Warning, "daemon disconnected — will retry automatically");
            state.ui.needs_render = true;
        }

        // --- Poll completion engine background init when the coprocess is free ---
        if state.cmd.pending_command.is_none() {
            state.completion_engine.poll_init(bash);
        }

        // --- Detect alternate-screen transitions (vim, etc.) ---
        let in_alt_screen = parser.screen().alternate_screen();
        if in_alt_screen != state.was_alt_screen {
            state.was_alt_screen = in_alt_screen;
            if let Ok(sz) = term.size() {
                let parser_rows = parser_rows_from_term_height(sz.height);
                let parser_cols = if in_alt_screen {
                    sz.width.max(1)
                } else {
                    parser_cols_from_term_width(sz.width)
                };
                parser.screen_mut().set_size(parser_rows, parser_cols);
                bash.resize(parser_rows, parser_cols);
            }
            state.ui.needs_render = true;
        }

        // --- Render only when needed ---
        if state.ui.needs_render {
            render_frame(term, parser, state, daemon_connected, theme)?;
            state.ui.needs_render = false;
        }

        // --- Compute timeout ---
        let timeout = terminal_io::compute_next_timeout(
            &state.cmd.pending_command,
            daemon_connected,
            state.last_heartbeat_sent,
            state.completion_engine.is_ready(),
        );

        // --- Select on all event sources ---
        let no_pty: crossbeam_channel::Receiver<Vec<u8>> = crossbeam_channel::never();
        let pty_rx = if state.cmd.pending_command.is_some() {
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
                    if let Some(ref mut pending) = state.cmd.pending_command {
                        // Capture PTY bytes for CmdResponseBlock (skip alternate screen)
                        if pending.ai_execution_id.is_none() && !parser.screen().alternate_screen() {
                            state.cmd.cmd_output_capture.extend_from_slice(&bytes);
                        }
                        terminal_io::process_pty_bytes(&bytes, pending, parser, bash);
                    }
                    state.ui.needs_render = true;
                }
            }
            recv(crossterm_rx) -> msg => {
                if let Ok(evt) = msg {
                    match &evt {
                        Event::Mouse(mouse) => {
                            super::keys::handle_mouse(
                                mouse,
                                parser,
                                state,
                                term.size(),
                            );
                        }
                        Event::Resize(cols, rows) => {
                            let parser_rows = parser_rows_from_term_height(*rows);
                            let parser_cols = if state.was_alt_screen {
                                (*cols).max(1)
                            } else {
                                parser_cols_from_term_width(*cols)
                            };
                            bash.resize(parser_rows, parser_cols);
                            state.stream.md_stream.set_width(parser_cols);

                            // Guard: skip rerender if modal or streaming is active
                            let has_modal = state.modal.pending_confirmation.is_some()
                                || state.modal.pending_session_picker.is_some();
                            let is_streaming = state.stream.agent_streaming
                                || state.tracker.pending_tool().is_some();
                            if has_modal || is_streaming || state.was_alt_screen {
                                // Just resize the parser without rerendering
                                parser.screen_mut().set_size(parser_rows, parser_cols);
                            } else {
                                // Recreate parser at new size and rerender all blocks
                                *parser = vt100::Parser::new(
                                    parser_rows,
                                    parser_cols,
                                    super::state::MAX_SCROLLBACK,
                                );
                                rerender_all(parser, &mut state.tracker, &mut state.ui.scroll_offset);
                            }
                        }
                        Event::Paste(ref text) => {
                            if state.cmd.pending_command.is_some() {
                                // Forward paste text to the running command's PTY.
                                bash.send_bytes(text.as_bytes());
                            } else if !state.stream.agent_streaming {
                                state.input.insert_paste(text.clone());
                                state.ui.scroll_offset = 0;
                                parser.screen_mut().set_scrollback(0);
                            }
                        }
                        _ => {}
                    }

                    // Process key events (only if not mouse/resize).
                    if let Event::Key(key) = evt {
                        if key.kind == KeyEventKind::Press {
                            // --- Modal key interception (highest priority) ---
                            let mut key_handled = false;
                            if state.tool_result_modal.is_visible() {
                                match key.code {
                                    KeyCode::Esc => state.tool_result_modal.close(),
                                    KeyCode::Up => state.tool_result_modal.scroll_up(1),
                                    KeyCode::Down => state.tool_result_modal.scroll_down(1),
                                    KeyCode::PageUp => state.tool_result_modal.scroll_up(20),
                                    KeyCode::PageDown => state.tool_result_modal.scroll_down(20),
                                    _ => {}
                                }
                                key_handled = true;
                            }

                            // --- Session picker key interception ---
                            if state.modal.pending_session_picker.is_some() && !key_handled {
                                key_handled = super::keys::handle_session_picker(
                                    key.code,
                                    state,
                                    parser,
                                    client,
                                );
                            }

                            // --- Confirmation prompt key interception ---
                            if state.modal.pending_confirmation.is_some() && !key_handled {
                                key_handled = super::keys::handle_confirmation(
                                    key.code,
                                    state,
                                    parser,
                                    client,
                                );
                            }

                            // --- Cancel AI streaming on Escape ---
                            if state.stream.agent_streaming && !key_handled {
                                if key.code == KeyCode::Esc {
                                    if let Some(ref rid) = state.stream.active_request_id {
                                        if let Some(ref mut c) = client {
                                            if let Some(frame) = ipc_messages::build_cancel_request(rid) {
                                                c.send(&frame);
                                            }
                                        }
                                    }
                                    // Finalize thinking block if in progress
                                    finalize_thinking(
                                        parser,
                                        &mut state.stream.thinking_start,
                                        &mut state.stream.thinking_buffer,
                                        &mut state.stream.thinking_scrollback,
                                        &mut state.tracker,
                                        theme.ansi_thinking,
                                    );
                                    // Flush any in-progress AI content
                                    let final_bytes = state.stream.md_stream.finish();
                                    if !final_bytes.is_empty() {
                                        parser.process(&final_bytes);
                                    }
                                    // Close AI response block if one was open
                                    if let Some(start) = state.stream.ai_start_scrollback.take() {
                                        let ai_end = get_scrollback_line(parser);
                                        let line_count = (ai_end.saturating_sub(start)) as u16;
                                        if line_count > 0 {
                                            let lines = std::mem::take(&mut state.stream.ai_rendered_lines);
                                            state.tracker.record_ai_response(start, line_count, lines);
                                        } else {
                                            state.stream.ai_rendered_lines.clear();
                                        }
                                    }
                                    state.stream.agent_streaming = false;
                                    state.stream.active_request_id = None;
                                    state.stream.md_stream.reset();
                                    parser_push_notice(parser, NoticeKind::Notice, "[cancelled]");
                                    key_handled = true;
                                }
                            }

                            // --- Forward keystrokes to PTY when a command is executing ---
                            if state.cmd.pending_command.is_some() {
                                if key.code == KeyCode::Char('c')
                                    && key.modifiers.contains(KeyModifiers::CONTROL)
                                {
                                    // fall through to input handler
                                } else if let Some(bytes) =
                                    terminal_input::key_event_to_bytes(key.code, key.modifiers)
                                {
                                    bash.send_bytes(&bytes);
                                    if let Some(ref mut p) = state.cmd.pending_command {
                                        p.last_activity = Instant::now();
                                    }
                                    key_handled = true;
                                } else {
                                    key_handled = true;
                                }
                            }

                            if !key_handled {
                                if super::keys::handle_completion_popup(key.code, state) {
                                    // handled by completion popup
                                } else if super::keys::handle_block_nav(key.code, key.modifiers, state, parser) {
                                    // handled by block navigation
                                } else {
                                    match super::keys::handle_input_keys(
                                        key.code,
                                        key.modifiers,
                                        term,
                                        parser,
                                        bash,
                                        client,
                                        state,
                                    ) {
                                        super::keys::InputKeyResult::Exit => break,
                                        super::keys::InputKeyResult::Continue => {}
                                    }
                                }
                            }
                        }
                    }

                    state.ui.needs_render = true;
                }
            }
            recv(daemon_rx) -> msg => {
                if let Ok(msg) = msg {
                    // Collect all daemon messages (first + drained) before processing,
                    // to avoid borrow conflicts between daemon_rx and client.
                    let mut daemon_msgs = vec![msg];
                    while let Ok(msg2) = daemon_rx.try_recv() {
                        daemon_msgs.push(msg2);
                    }
                    for dm in daemon_msgs {
                        let md_width = term.size().map(|s| parser_cols_from_term_width(s.width)).unwrap_or(80);
                        let lease_frames = daemon::handle_lease_message(&dm, state);
                        if let Some(ref mut c) = client {
                            for frame in lease_frames { c.send(&frame); }
                        }
                        daemon::handle_daemon_message(dm, parser, state, md_width, theme);
                    }
                    state.ui.needs_render = true;
                }
            }
            recv(tick_rx) -> _ => {
                state.ui.anim.update_active(
                    state.cmd.pending_command.is_some(),
                    state.stream.agent_streaming,
                    state.stream.thinking_start.is_some(),
                    state.tracker.pending_tool().is_some(),
                );
                if state.ui.anim.tick() || current_hint(&state).is_some() {
                    state.ui.needs_render = true;
                }
            }
            default(timeout) => {
                state.ui.needs_render = true;
            }
        }

        // --- Post-select: drain additional PTY data ---
        if state.cmd.pending_command.is_some() {
            let drain: Vec<Vec<u8>> = {
                let pty_rx = bash.pty_receiver();
                let mut collected = Vec::new();
                while let Ok(bytes) = pty_rx.try_recv() {
                    collected.push(bytes);
                }
                collected
            };
            for bytes in &drain {
                if let Some(ref mut pending) = state.cmd.pending_command {
                    if pending.ai_execution_id.is_none() && !parser.screen().alternate_screen() {
                        state.cmd.cmd_output_capture.extend_from_slice(bytes);
                    }
                    terminal_io::process_pty_bytes(bytes, pending, parser, bash);
                }
            }
        }

        // --- Post-select: check command completion ---
        if let Some(ref mut pending) = state.cmd.pending_command {
            if let Some(result) = BashCoprocess::check_complete(
                &pending.accumulated,
                &pending.command,
                &pending.sentinel,
            ) {
                let is_ai_command = pending.ai_execution_id.is_some();
                if !is_ai_command {
                    parser.process(b"\x1b[A\x1b[2K");
                    if result.exit_code != 0 {
                        parser_push_notice(
                            parser,
                            NoticeKind::Error,
                            &format!("exit code: {}", result.exit_code),
                        );
                    }
                }
                state.shell.cwd = bash.capture_cwd();
                state.shell.git_info = bash.capture_git_info();
                if pending.needs_env_refresh {
                    state.shell.cached_env_vars = bash.capture_env();
                }
                if let Some(ref execution_id) = pending.ai_execution_id {
                    // AI-requested command: send CommandCompleted back via lease protocol
                    let lease_id = state.shell.shell_relay.active_lease
                        .as_ref()
                        .map(|l| l.lease_id.clone())
                        .unwrap_or_default();
                    if let Some(ref mut c) = client {
                        if let Some(frame) = ipc_messages::build_command_completed(
                            &lease_id,
                            execution_id,
                            &result.output,
                            result.exit_code,
                            &state.shell.cwd,
                        ) {
                            c.send(&frame);
                        }
                    }
                } else {
                    // User command: send CommandResult as before
                    if let Some(ref mut c) = client {
                        if let Some(frame) = ipc_messages::build_command_result(
                            &pending.command,
                            &result.output,
                            result.exit_code,
                            &state.shell.cwd,
                        ) {
                            c.send(&frame);
                        }
                    }
                }
                if !is_ai_command {
                    let cmd_end = get_scrollback_line(parser);
                    if let Some(start) = state.cmd.cmd_start_scrollback.take() {
                        let line_count = (cmd_end.saturating_sub(start)) as u16;
                        if line_count > 0 {
                            let raw_bytes = std::mem::take(&mut state.cmd.cmd_output_capture);
                            state.tracker.record_cmd_response(
                                &pending.command,
                                start,
                                line_count,
                                result.exit_code,
                                raw_bytes,
                            );
                        } else {
                            state.cmd.cmd_output_capture.clear();
                        }
                    } else {
                        state.cmd.cmd_output_capture.clear();
                    }
                    parser.process(b"\r\n");
                }
                state.cmd.pending_command = None;
                state.ui.scroll_offset = 0;
                parser.screen_mut().set_scrollback(0);
                state.ui.needs_render = true;
            } else if pending.last_activity.elapsed() > std::time::Duration::from_secs(300) {
                bash.send_interrupt();
                bash.drain_for(100);
                parser_push_notice(
                    parser,
                    NoticeKind::Error,
                    "command timed out (no activity for 5m)",
                );
                let cmd_end = get_scrollback_line(parser);
                if let Some(start) = state.cmd.cmd_start_scrollback.take() {
                    let line_count = (cmd_end.saturating_sub(start)) as u16;
                    if line_count > 0 {
                        let raw_bytes = std::mem::take(&mut state.cmd.cmd_output_capture);
                        state.tracker.record_cmd_response(&pending.command, start, line_count, -1, raw_bytes);
                    }
                }
                parser.process(b"\r\n");
                state.cmd.pending_command = None;
                state.ui.scroll_offset = 0;
                parser.screen_mut().set_scrollback(0);
                state.ui.needs_render = true;
            }
        }

        // --- Start leased AI command when the coprocess is free ---
        if state.cmd.pending_command.is_none() {
            // Try to promote a pending lease first.
            daemon::promote_pending_lease(state, client);

            // Check if the active lease has a command ready to execute.
            let cmd_to_start = state.shell.shell_relay.active_lease.as_mut().and_then(|lease| {
                lease.current_command.take().map(|cmd| (lease.lease_id.clone(), cmd))
            });

            if let Some((lease_id, leased_cmd)) = cmd_to_start {
                let needs_env = super::state::command_modifies_env(&leased_cmd.command);
                let wrapped_command = format!(
                    "GIT_PAGER=cat PAGER=cat SYSTEMD_PAGER=cat GIT_TERMINAL_PROMPT=0 GIT_EDITOR=true {}",
                    &leased_cmd.command
                );
                match bash.start_command(&wrapped_command) {
                    Some(sentinel) => {
                        // Send CommandStarted ACK — this is when the daemon starts
                        // the execution timeout timer.
                        if let Some(ref mut c) = client {
                            if let Some(frame) = ipc_messages::build_command_started(
                                &lease_id,
                                &leased_cmd.execution_id,
                            ) {
                                c.send(&frame);
                            }
                        }
                        state.cmd.cmd_start_scrollback = Some(get_scrollback_line(parser));
                        state.cmd.pending_command = Some(super::state::PendingCommand {
                            sentinel,
                            accumulated: String::new(),
                            command: leased_cmd.command,
                            last_activity: Instant::now(),
                            needs_env_refresh: needs_env,
                            ai_execution_id: Some(leased_cmd.execution_id),
                        });
                        state.ui.needs_render = true;
                    }
                    None => {
                        // Failed to start — send CommandFailed back
                        if let Some(ref mut c) = client {
                            if let Some(frame) = ipc_messages::build_command_failed(
                                &lease_id,
                                &leased_cmd.execution_id,
                                "failed to send command to shell",
                                &state.shell.cwd,
                            ) {
                                c.send(&frame);
                            }
                        }
                    }
                }
            }
        }

        // --- Periodic heartbeat every 10s ---
        if let Some(ref mut c) = client {
            if c.is_connected()
                && state.last_heartbeat_sent.elapsed() > std::time::Duration::from_secs(10)
            {
                if let Some(hb) = ipc_messages::build_heartbeat() {
                    c.send(&hb);
                }
                state.last_heartbeat_sent = Instant::now();
            }
        }

        // --- Periodic daemon reconnection every 5s when disconnected ---
        if client.is_none()
            && state.last_reconnect_attempt.elapsed() > std::time::Duration::from_secs(5)
        {
            state.last_reconnect_attempt = Instant::now();
            let socket = daemon_launcher::socket_path();
            if let Some(mut new_client) = CodivdClient::connect(&socket) {
                if let Some(snapshot) =
                    ipc_messages::build_env_snapshot(&state.shell.cached_env_vars, "", &state.shell.cwd)
                {
                    new_client.send(&snapshot);
                }
                if let Some(ref sid) = state.session_id {
                    if let Some(frame) = ipc_messages::build_load_session(sid) {
                        new_client.send(&frame);
                    }
                } else {
                    reset_screen(parser, &mut state.ui.scroll_offset, &mut state.tracker);
                }
                *client = Some(new_client);
                state.last_heartbeat_sent = Instant::now();
                log::info!("reconnected to daemon");
                state.ui.needs_render = true;
            }
        }
    }

    Ok(())
}
