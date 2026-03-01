//! Main terminal UI — event loop, rendering, and command dispatch.
//!
//! This is the entry point for the ratatui-based terminal interface. It sets
//! up crossterm raw mode, runs a synchronous event loop, and renders a linear
//! scroll-down layout consisting of a VT100 pseudoterminal, an input line,
//! and a status bar.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossbeam_channel;
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use tui_term::widget::{Cursor as PtCursor, PseudoTerminal};

use crate::ipc::client::SlatedClient;
use crate::ipc::messages as ipc_messages;
use crate::shell::bash_coprocess::BashCoprocess;
use crate::shell::command_index::{classify_input, CommandIndex, InputAction};
use crate::shell::completion_engine::CompletionEngine;
use crate::shell::interactive::InteractiveSession;

use super::completion_popup::CompletionPopup;
use super::input::InputLine;
use super::selection::TextSelection;
use crate::VERSION;
use crate::markdown::MarkdownStream;

/// Send an agent request to the daemon. Returns `true` if the request was sent.
fn send_agent_request(
    client: &mut SlatedClient,
    query: &str,
    cwd: &str,
) -> bool {
    let request_id = format!("agent-{}", rand::random::<u64>());
    let context = ipc_messages::SessionContext {
        cwd: cwd.to_string(),
        recent_commands: Vec::new(),
        env_vars: Vec::new(),
    };
    if let Some(frame) = ipc_messages::build_agent_request(query, &request_id, context) {
        client.send(&frame);
        true
    } else {
        false
    }
}

/// Tracks a command that has been submitted to bash but hasn't completed yet.
struct PendingCommand {
    sentinel: String,
    accumulated: String,
    command: String,
    last_activity: Instant,
    display_buf: String,
    last_output: Instant,
}

/// Default scrollback limit (number of lines retained).
const MAX_SCROLLBACK: usize = 10_000;

/// Timeout before flushing incomplete line buffer (for prompts that don't end with \n).
const DISPLAY_FLUSH_MS: u64 = 100;

/// Check whether a line is sentinel protocol noise that should be suppressed.
fn is_sentinel_noise(line: &str, command: &str, sentinel: &str) -> bool {
    let plain = BashCoprocess::strip_ansi(line);
    let trimmed = plain.trim();

    // Lines containing the sentinel marker
    if trimmed.contains(sentinel) {
        return true;
    }

    // Lines containing __SLATE_EXIT
    if trimmed.contains("__SLATE_EXIT") {
        return true;
    }

    // Bare prompt lines
    if trimmed == "$" || trimmed == "$ " {
        return true;
    }

    let without_prompt = trimmed.strip_prefix("$ ").unwrap_or(trimmed);
    let cmd_trimmed = command.trim();

    // Echoed command line
    if !cmd_trimmed.is_empty() && without_prompt == cmd_trimmed {
        return true;
    }

    // Echoed command with sentinel suffix (e.g. "ls; __SLATE_EXIT=$?; ...")
    if !cmd_trimmed.is_empty() && without_prompt.starts_with(cmd_trimmed) {
        let rest = &without_prompt[cmd_trimmed.len()..];
        if rest.starts_with("; __SLATE_EXIT") {
            return true;
        }
    }

    false
}

/// Helper: write styled text to the vt100 parser using ANSI SGR codes.
fn parser_push_styled(parser: &mut vt100::Parser, text: &str, ansi_prefix: &str) {
    let line = format!("{}{}\x1b[0m\r\n", ansi_prefix, text);
    parser.process(line.as_bytes());
}

/// Truncate tool text for display, collapsing newlines and limiting length.
fn truncate_tool_text(text: &str, max_len: usize) -> String {
    // Collapse whitespace/newlines into single spaces for inline display.
    let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.len() <= max_len {
        collapsed
    } else {
        format!("{}...", &collapsed[..max_len])
    }
}

/// Run the main terminal UI.
///
/// This function takes ownership of the terminal, enters the alternate screen
/// with raw mode, and runs a synchronous event loop until the user exits.
pub fn run(
    bash: &mut BashCoprocess,
    command_index: &CommandIndex,
    shutdown: Arc<AtomicBool>,
    initial_cwd: String,
    client: Option<SlatedClient>,
) -> Result<(), Box<dyn std::error::Error>> {
    // --- Terminal setup ---
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut term = Terminal::new(backend)?;

    // --- State ---
    let term_size = term.size()?;
    // Parser rows = total height - 1 (status bar)
    let parser_rows = term_size.height.saturating_sub(1).max(1);
    let parser_cols = term_size.width.max(1);
    let mut parser = vt100::Parser::new(parser_rows, parser_cols, MAX_SCROLLBACK);
    let mut scroll_offset: usize = 0;

    let mut input = InputLine::new();
    let mut interactive_session = InteractiveSession::new();
    let mut cwd = initial_cwd;
    let mut client = client;
    let mut prompt_is_live = false;

    // Welcome message.
    parser_push_styled(&mut parser, &format!("slate v{} — type 'exit' to quit", VERSION), "\x1b[90m");

    // --- Event loop ---
    let result = event_loop(
        &mut term,
        &mut parser,
        &mut scroll_offset,
        &mut input,
        &mut interactive_session,
        bash,
        command_index,
        &shutdown,
        &mut cwd,
        &mut client,
        &mut prompt_is_live,
    );

    // --- Cleanup (always runs) ---
    terminal::disable_raw_mode()?;
    execute!(term.backend_mut(), DisableMouseCapture, LeaveAlternateScreen)?;
    term.show_cursor()?;

    result
}

/// Spawn a background thread that blocks on `crossterm::event::read()` and
/// forwards events through a crossbeam channel. This lets the main loop
/// `select!` on terminal input alongside PTY and daemon channels.
fn spawn_crossterm_reader(
    paused: Arc<AtomicBool>,
) -> crossbeam_channel::Receiver<crossterm::event::Event> {
    let (tx, rx) = crossbeam_channel::unbounded();
    std::thread::Builder::new()
        .name("crossterm-reader".into())
        .spawn(move || loop {
            // When paused (interactive session active), spin on poll() with a
            // short timeout instead of blocking in read(). This prevents the
            // reader thread from stealing keystrokes meant for vim/less/etc.
            if paused.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            // Use poll+read instead of a bare read() so we periodically check
            // the pause flag even when no events arrive.
            match crossterm::event::poll(Duration::from_millis(100)) {
                Ok(true) => match crossterm::event::read() {
                    Ok(evt) => {
                        if tx.send(evt).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                },
                Ok(false) => {} // timeout, loop back to check pause flag
                Err(_) => break,
            }
        })
        .expect("spawn crossterm reader");
    rx
}

/// Compute the minimum timeout until the next timer-driven action.
///
/// Returns the shortest of:
/// - Display flush remaining (100ms after last output, only when pending command has buffered incomplete line)
/// - Heartbeat remaining (10s interval, only when daemon connected)
/// - Activity timeout remaining (300s, only when a command is pending)
/// - Fallback ceiling of 60s (guarantees we wake up periodically)
fn compute_next_timeout(
    pending_command: &Option<PendingCommand>,
    daemon_connected: bool,
    last_heartbeat_sent: Instant,
    completion_engine_ready: bool,
) -> Duration {
    let mut timeout = Duration::from_secs(60); // fallback ceiling

    // While the completion engine is initializing (sourcing bash-completion),
    // poll frequently so poll_init() can drain PTY output via try_read().
    if !completion_engine_ready && pending_command.is_none() {
        timeout = timeout.min(Duration::from_millis(50));
    }

    if let Some(ref pending) = pending_command {
        // Display flush: 100ms after last output when buffer is non-empty.
        if !pending.display_buf.is_empty() {
            let elapsed = pending.last_output.elapsed();
            let flush_deadline = Duration::from_millis(DISPLAY_FLUSH_MS);
            if elapsed < flush_deadline {
                timeout = timeout.min(flush_deadline - elapsed);
            } else {
                // Already past deadline — wake immediately.
                return Duration::ZERO;
            }
        }

        // Activity timeout: 300s since last activity.
        let activity_deadline = Duration::from_secs(300);
        let activity_elapsed = pending.last_activity.elapsed();
        if activity_elapsed < activity_deadline {
            timeout = timeout.min(activity_deadline - activity_elapsed);
        } else {
            return Duration::ZERO;
        }
    }

    // Heartbeat: every 10s when connected.
    if daemon_connected {
        let hb_deadline = Duration::from_secs(10);
        let hb_elapsed = last_heartbeat_sent.elapsed();
        if hb_elapsed < hb_deadline {
            timeout = timeout.min(hb_deadline - hb_elapsed);
        } else {
            return Duration::ZERO;
        }
    }

    timeout
}

/// Process PTY output bytes for a pending command: accumulate, filter noise,
/// and feed complete lines to the VT100 parser.
fn process_pty_bytes(bytes: &[u8], pending: &mut PendingCommand, parser: &mut vt100::Parser) {
    let text = String::from_utf8_lossy(bytes);
    pending.accumulated.push_str(&text);
    pending.display_buf.push_str(&text);
    pending.last_activity = Instant::now();
    pending.last_output = Instant::now();

    while let Some(newline_pos) = pending.display_buf.find('\n') {
        let line = pending.display_buf[..newline_pos].to_string();
        pending.display_buf = pending.display_buf[newline_pos + 1..].to_string();
        if !is_sentinel_noise(&line, &pending.command, &pending.sentinel) {
            let out = format!("{}\n", line);
            parser.process(out.as_bytes());
        }
        if parser.screen().alternate_screen() {
            break;
        }
    }
}

/// Process a single daemon message, updating parser state.
fn handle_daemon_message(
    msg: ipc_messages::DaemonMessage,
    parser: &mut vt100::Parser,
    md_stream: &mut MarkdownStream,
    agent_streaming: &mut bool,
    last_daemon_timestamp: &mut u64,
) {
    match msg {
        ipc_messages::DaemonMessage::AgentStreamChunk {
            request_id: _,
            chunk,
        } => {
            match chunk {
                ipc_messages::StreamChunk::Text(t) => {
                    if let Some(ansi) = md_stream.push(&t) {
                        parser.process(&ansi);
                    }
                }
                ipc_messages::StreamChunk::Reasoning(t) => {
                    let t = t.replace('\n', "\r\n");
                    parser.process(t.as_bytes());
                }
                ipc_messages::StreamChunk::ToolCall { name, arguments } => {
                    let summary = truncate_tool_text(&arguments, 120);
                    parser_push_styled(
                        parser,
                        &format!("  {} {}", name, summary),
                        "\x1b[36m",
                    );
                }
                ipc_messages::StreamChunk::ToolResult { name, result } => {
                    let lines: Vec<&str> = result.lines().collect();
                    let max_lines = 20;
                    let max_line_len = 200;
                    let truncated_lines = lines.len() > max_lines;
                    let display_lines = &lines[..lines.len().min(max_lines)];
                    let mut output = format!("  {} result:", name);
                    for line in display_lines {
                        if line.len() > max_line_len {
                            output.push_str(&format!("\r\n    {}...", &line[..max_line_len]));
                        } else {
                            output.push_str(&format!("\r\n    {}", line));
                        }
                    }
                    if truncated_lines {
                        output.push_str(&format!(
                            "\r\n    ... ({} more lines)",
                            lines.len() - max_lines
                        ));
                    }
                    let line = format!("\x1b[2;32m{}\x1b[0m\r\n", output);
                    parser.process(line.as_bytes());
                }
            };
        }
        ipc_messages::DaemonMessage::AgentComplete {
            request_id: _,
            summary: _,
        } => {
            if *agent_streaming {
                let final_bytes = md_stream.finish();
                if !final_bytes.is_empty() {
                    parser.process(&final_bytes);
                }
                parser.process(b"\r\n");
                md_stream.reset();
                *agent_streaming = false;
            }
        }
        ipc_messages::DaemonMessage::ConfirmationRequest {
            request_id,
            description,
            risk: _,
        } => {
            parser_push_styled(
                parser,
                &format!("[daemon] confirm ({}): {}", request_id, description),
                "\x1b[33m",
            );
        }
        ipc_messages::DaemonMessage::Error {
            request_id,
            message,
        } => {
            *agent_streaming = false;
            md_stream.reset();
            parser_push_styled(
                parser,
                &format!("[daemon] error ({}): {}", request_id, message),
                "\x1b[31m",
            );
        }
        ipc_messages::DaemonMessage::Heartbeat { timestamp } => {
            *last_daemon_timestamp = timestamp;
        }
    }
}

/// The main event loop. Factored out so cleanup always runs in `run()`.
fn event_loop(
    term: &mut Terminal<CrosstermBackend<io::Stdout>>,
    parser: &mut vt100::Parser,
    scroll_offset: &mut usize,
    input: &mut InputLine,
    interactive_session: &mut InteractiveSession,
    bash: &mut BashCoprocess,
    command_index: &CommandIndex,
    shutdown: &Arc<AtomicBool>,
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
    let mut md_stream = MarkdownStream::new(term.size().map(|s| s.width).unwrap_or(80));

    // Start background initialization (non-blocking) so the first Tab
    // press is fast without freezing the UI at startup.
    completion_engine.start_init(bash);

    // Spawn crossterm reader thread for channelized terminal input.
    // The pause flag stops it from reading stdin during interactive sessions
    // (vim, less, etc.) so it doesn't steal keystrokes.
    let crossterm_paused = Arc::new(AtomicBool::new(false));
    let crossterm_rx = spawn_crossterm_reader(crossterm_paused.clone());

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
            render_frame(term, parser, input, cwd, daemon_connected, last_daemon_timestamp, *scroll_offset, prompt_is_live, is_executing, agent_streaming, &completion_popup, &selection)?;
            needs_render = false;
        }

        // --- Compute timeout ---
        let timeout = compute_next_timeout(&pending_command, daemon_connected, last_heartbeat_sent, completion_engine.is_ready());

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
        let daemon_rx = client.as_ref().map_or(&no_daemon, |c| c.daemon_receiver());

        crossbeam_channel::select! {
            recv(pty_rx) -> msg => {
                if let Ok(bytes) = msg {
                    if let Some(ref mut pending) = pending_command {
                        process_pty_bytes(&bytes, pending, parser);
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
                                        let term_area_height = term.size().map(|s| s.height.saturating_sub(1).max(1)).unwrap_or(1);
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
                            let parser_cols = (*cols).max(1);
                            parser.screen_mut().set_size(parser_rows, parser_cols);
                            bash.resize(parser_rows);
                            md_stream.set_width(*cols);
                        }
                        _ => {}
                    }

                    // Process key events (only if not mouse/resize).
                    if let Event::Key(key) = evt {
                        if key.kind == KeyEventKind::Press {
                            // Clear text selection on any keypress.
                            selection.clear();

                            // --- Forward keystrokes to PTY when a command is executing ---
                            let mut key_handled = false;
                            if pending_command.is_some() {
                                match (key.code, key.modifiers) {
                                    (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => {
                                        // Fall through to existing Ctrl+C handler below.
                                    }
                                    (KeyCode::Enter, _) => {
                                        bash.send_bytes(b"\r");
                                        if let Some(ref mut p) = pending_command {
                                            p.last_activity = Instant::now();
                                        }
                                        key_handled = true;
                                    }
                                    (KeyCode::Char(ch), _) => {
                                        let mut buf = [0u8; 4];
                                        let s = ch.encode_utf8(&mut buf);
                                        bash.send_bytes(s.as_bytes());
                                        if let Some(ref mut p) = pending_command {
                                            p.last_activity = Instant::now();
                                        }
                                        key_handled = true;
                                    }
                                    (KeyCode::Backspace, _) => {
                                        bash.send_bytes(b"\x7f");
                                        if let Some(ref mut p) = pending_command {
                                            p.last_activity = Instant::now();
                                        }
                                        key_handled = true;
                                    }
                                    _ => {
                                        key_handled = true;
                                    }
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
                                            if let Some((selected, start, end)) = completion_popup.confirm() {
                                                let suffix = if std::path::Path::new(&selected).is_dir() {
                                                    "/"
                                                } else {
                                                    " "
                                                };
                                                input.replace_range(start, end, &format!("{}{}", selected, suffix));
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
                                    match (key.code, key.modifiers) {
                                        // --- Ctrl combos ---
                                        (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => {
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

                                        (KeyCode::Char('d'), m)
                                            if m.contains(KeyModifiers::CONTROL) && input.content().is_empty() =>
                                        {
                                            break; // exit
                                        }

                                        // --- Enter: submit input ---
                                        (KeyCode::Enter, _) if pending_command.is_none() => {
                                            let raw_input = input.submit();
                                            parser.process(b"\r\n");
                                            *prompt_is_live = false;
                                            let action = classify_input(&raw_input, command_index);

                                            match action {
                                                InputAction::Empty => {}

                                                InputAction::Exit => {
                                                    break;
                                                }

                                                InputAction::AiQuery => {
                                                    if let Some(ref mut c) = client {
                                                        let query = raw_input.trim_start().strip_prefix('?').unwrap_or(&raw_input);
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

                                                InputAction::Interactive => {
                                                    let env = bash.capture_env();
                                                    let interactive_cwd = cwd.clone();
                                                    // Pause crossterm reader so it doesn't steal keystrokes from the interactive program.
                                                    crossterm_paused.store(true, Ordering::Relaxed);
                                                    terminal::disable_raw_mode()?;
                                                    execute!(term.backend_mut(), DisableMouseCapture, LeaveAlternateScreen)?;
                                                    interactive_session.spawn_and_enter(
                                                        &raw_input,
                                                        &env,
                                                        &interactive_cwd,
                                                    );
                                                    let _ = std::io::Write::write_all(
                                                        &mut std::io::stdout(),
                                                        b"\x1b[2J\x1b[H",
                                                    );
                                                    let _ = std::io::Write::flush(&mut std::io::stdout());
                                                    execute!(term.backend_mut(), EnterAlternateScreen, EnableMouseCapture)?;
                                                    terminal::enable_raw_mode()?;
                                                    term.clear()?;
                                                    // Resume crossterm reader and drain any stale events.
                                                    crossterm_paused.store(false, Ordering::Relaxed);
                                                    while crossterm_rx.try_recv().is_ok() {}
                                                    if parser.screen().alternate_screen() {
                                                        parser.process(b"\x1b[?1049l");
                                                        parser.process(b"\x1b[?25h");
                                                    } else {
                                                        let visible_rows = parser.screen().size().0;
                                                        for _ in 0..visible_rows {
                                                            parser.process(b"\r\n");
                                                        }
                                                        parser.process(b"\x1b[2J\x1b[H");
                                                    }
                                                    *cwd = bash.capture_cwd();
                                                    parser_push_styled(
                                                        parser,
                                                        "(interactive session ended)",
                                                        "\x1b[90m",
                                                    );
                                                }

                                                InputAction::Clear => {
                                                    let term_size = term.size()?;
                                                    let rows = term_size.height.saturating_sub(1).max(1);
                                                    let cols = term_size.width.max(1);
                                                    *parser = vt100::Parser::new(rows, cols, MAX_SCROLLBACK);
                                                    *scroll_offset = 0;
                                                    *prompt_is_live = false;
                                                }

                                                InputAction::Reset => {
                                                    let term_size = term.size()?;
                                                    let rows = term_size.height.saturating_sub(1).max(1);
                                                    let cols = term_size.width.max(1);
                                                    *parser = vt100::Parser::new(rows, cols, MAX_SCROLLBACK);
                                                    *scroll_offset = 0;
                                                    *prompt_is_live = false;
                                                    input.clear_history();
                                                    completion_engine = CompletionEngine::new();
                                                    completion_engine.start_init(bash);
                                                    completion_popup.dismiss();
                                                    selection.clear();
                                                    parser_push_styled(
                                                        parser,
                                                        &format!("slate v{} — type 'exit' to quit", crate::VERSION),
                                                        "\x1b[90m",
                                                    );
                                                }

                                                InputAction::Execute => {
                                                    match bash.start_command(&raw_input) {
                                                        Some(sentinel) => {
                                                            pending_command = Some(PendingCommand {
                                                                sentinel,
                                                                accumulated: String::new(),
                                                                command: raw_input.clone(),
                                                                last_activity: Instant::now(),
                                                                display_buf: String::new(),
                                                                last_output: Instant::now(),
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
                                                            &format!("command not found: {}", word),
                                                            "\x1b[31m",
                                                        );
                                                    }
                                                }
                                            }

                                            *scroll_offset = 0;
                                            parser.screen_mut().set_scrollback(0);
                                        }

                                        // --- Tab: completion ---
                                        (KeyCode::Tab, _) if pending_command.is_none() => {
                                            let line = input.content().to_string();
                                            let cursor = input.cursor_byte_offset();
                                            if let Some(result) = completion_engine.complete(bash, &line, cursor) {
                                                match result.candidates.len() {
                                                    0 => {}
                                                    1 => {
                                                        let candidate = &result.candidates[0];
                                                        let suffix = if std::path::Path::new(candidate).is_dir() { "/" } else { " " };
                                                        input.replace_range(
                                                            result.replace_start,
                                                            result.replace_end,
                                                            &format!("{}{}", candidate, suffix),
                                                        );
                                                    }
                                                    _ => {
                                                        let common = longest_common_prefix(&result.candidates);
                                                        let prefix = &line[result.replace_start..result.replace_end];
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
                                        (KeyCode::Up, _) => { input.history_up(); }
                                        (KeyCode::Down, _) => { input.history_down(); }

                                        // --- Cursor movement ---
                                        (KeyCode::Left, _) => { input.move_left(); }
                                        (KeyCode::Right, _) => { input.move_right(); }
                                        (KeyCode::Home, _) => { input.home(); }
                                        (KeyCode::End, _) => { input.end(); }

                                        // --- Editing ---
                                        (KeyCode::Backspace, _) => { input.backspace(); }
                                        (KeyCode::Delete, _) => { input.delete(); }

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
                                            if modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
                                                // Unhandled modifier combo — ignore.
                                            } else {
                                                input.insert(ch);
                                                *scroll_offset = 0;
                                                parser.screen_mut().set_scrollback(0);
                                            }
                                        }

                                        _ => {}
                                    }
                                }
                            }
                        }
                    }

                    needs_render = true;
                }
            }
            recv(daemon_rx) -> msg => {
                if let Ok(msg) = msg {
                    handle_daemon_message(msg, parser, &mut md_stream, &mut agent_streaming, &mut last_daemon_timestamp);
                    // Drain any additional daemon messages that arrived.
                    while let Ok(msg2) = daemon_rx.try_recv() {
                        handle_daemon_message(msg2, parser, &mut md_stream, &mut agent_streaming, &mut last_daemon_timestamp);
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
                    process_pty_bytes(&bytes, pending, parser);
                }
            }
        }

        // --- Post-select: handle alternate screen detection ---
        if parser.screen().alternate_screen() && pending_command.is_some() {
            let pending = pending_command.take().unwrap();
            let real_size = term.size()?;
            bash.resize_full(real_size.height, real_size.width);
            // Pause crossterm reader so it doesn't steal keystrokes from the interactive program.
            crossterm_paused.store(true, Ordering::Relaxed);
            terminal::disable_raw_mode()?;
            execute!(term.backend_mut(), DisableMouseCapture, LeaveAlternateScreen)?;
            let (reader_rx, writer) = bash.reader_and_writer();
            let accumulated = interactive_session.enter_with_sentinel(
                reader_rx,
                writer,
                &pending.sentinel,
                real_size.height,
                real_size.width,
            );
            let rows = real_size.height.saturating_sub(1).max(1);
            bash.resize_full(rows, 500);
            let _ = std::io::Write::write_all(
                &mut std::io::stdout(),
                b"\x1b[2J\x1b[H",
            );
            let _ = std::io::Write::flush(&mut std::io::stdout());
            execute!(term.backend_mut(), EnterAlternateScreen, EnableMouseCapture)?;
            terminal::enable_raw_mode()?;
            term.clear()?;
            // Resume crossterm reader and drain any stale events.
            crossterm_paused.store(false, Ordering::Relaxed);
            while crossterm_rx.try_recv().is_ok() {}
            let cols = real_size.width.max(1);
            parser.screen_mut().set_size(rows, cols);
            parser.process(b"\x1b[?1049l");
            parser.process(b"\x1b[?25h");
            let mut alt_cmd_output = String::new();
            let mut alt_cmd_exit: i32 = 0;
            if let Some((acc, captured_screen)) = accumulated {
                if let Some(screen_bytes) = captured_screen {
                    parser.process(&screen_bytes);
                    parser.process(b"\r\n");
                }
                if let Some(result) = BashCoprocess::check_complete(
                    &acc,
                    &pending.command,
                    &pending.sentinel,
                ) {
                    if result.exit_code != 0 {
                        parser_push_styled(
                            parser,
                            &format!("exit code: {}", result.exit_code),
                            "\x1b[31m",
                        );
                    }
                    alt_cmd_output = result.output;
                    alt_cmd_exit = result.exit_code;
                }
            }
            *cwd = bash.capture_cwd();
            if let Some(ref mut c) = client {
                if let Some(frame) = ipc_messages::build_command_result(
                    &pending.command,
                    &alt_cmd_output,
                    alt_cmd_exit,
                    cwd,
                ) {
                    c.send(&frame);
                }
            }
            *scroll_offset = 0;
            parser.screen_mut().set_scrollback(0);
            needs_render = true;
        }

        // --- Post-select: check command completion ---
        if let Some(ref mut pending) = pending_command {
            // Flush incomplete line buffer after timeout.
            if !pending.display_buf.is_empty()
                && pending.last_output.elapsed() > Duration::from_millis(DISPLAY_FLUSH_MS)
            {
                let buf = std::mem::take(&mut pending.display_buf);
                if !is_sentinel_noise(&buf, &pending.command, &pending.sentinel) {
                    parser.process(buf.as_bytes());
                }
                needs_render = true;
            }

            if let Some(result) = BashCoprocess::check_complete(
                &pending.accumulated,
                &pending.command,
                &pending.sentinel,
            ) {
                let buf = std::mem::take(&mut pending.display_buf);
                if !buf.is_empty()
                    && !is_sentinel_noise(&buf, &pending.command, &pending.sentinel)
                {
                    parser.process(buf.as_bytes());
                }
                if result.exit_code != 0 {
                    parser_push_styled(
                        parser,
                        &format!("exit code: {}", result.exit_code),
                        "\x1b[31m",
                    );
                }
                *cwd = bash.capture_cwd();
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
                pending_command = None;
                *scroll_offset = 0;
                parser.screen_mut().set_scrollback(0);
                needs_render = true;
            } else if pending.last_activity.elapsed() > Duration::from_secs(300) {
                bash.send_interrupt();
                bash.drain_for(100);
                parser_push_styled(parser, "command timed out (no activity for 5m)", "\x1b[31m");
                pending_command = None;
                *scroll_offset = 0;
                parser.screen_mut().set_scrollback(0);
                needs_render = true;
            }
        }

        // --- Periodic heartbeat every 10s ---
        if let Some(ref mut c) = client {
            if c.is_connected() && last_heartbeat_sent.elapsed() > Duration::from_secs(10) {
                if let Some(hb) = ipc_messages::build_heartbeat() {
                    c.send(&hb);
                }
                last_heartbeat_sent = Instant::now();
            }
        }
    }

    Ok(())
}

/// Render one frame of the terminal UI.
fn render_frame(
    term: &mut Terminal<CrosstermBackend<io::Stdout>>,
    parser: &mut vt100::Parser,
    input: &InputLine,
    cwd: &str,
    daemon_connected: bool,
    daemon_timestamp: u64,
    scroll_offset: usize,
    prompt_is_live: &mut bool,
    is_executing: bool,
    agent_streaming: bool,
    completion_popup: &CompletionPopup,
    selection: &TextSelection,
) -> Result<(), Box<dyn std::error::Error>> {
    // Write live prompt into the vt100 parser (only when scrolled to bottom
    // and no command is currently executing or agent streaming).
    if scroll_offset == 0 && !is_executing && !agent_streaming {
        let prompt_text = format!("{}$ ", cwd);
        let input_text = input.content();

        // Clear current line, write prompt + input with styling.
        parser.process(
            format!("\r\x1b[K\x1b[1;32m{}\x1b[0m{}", prompt_text, input_text).as_bytes(),
        );

        // Position cursor: move back from end if cursor isn't at end of input.
        let target_col = prompt_text.len() + input.cursor_position();
        let current_col = prompt_text.len() + input_text.len();
        if current_col > target_col {
            parser.process(format!("\x1b[{}D", current_col - target_col).as_bytes());
        }

        *prompt_is_live = true;
    }

    term.draw(|frame| {
        let area = frame.area();

        // Layout: terminal area (flexible) | status bar (1)
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),    // PseudoTerminal (now includes prompt)
                Constraint::Length(1), // Status bar
            ])
            .split(area);

        let term_area = chunks[0];
        let status_area = chunks[1];

        // --- Render pseudoterminal ---
        let cursor_visible = scroll_offset == 0;
        let pseudo_term = PseudoTerminal::new(parser.screen())
            .cursor(PtCursor::default().visibility(cursor_visible));
        frame.render_widget(pseudo_term, term_area);

        // --- Render selection highlight ---
        if selection.is_active() {
            let ((sc, sr), (ec, er)) = selection.normalized_range();
            let buf = frame.buffer_mut();
            for row in sr..=er {
                if row < term_area.top() || row >= term_area.bottom() {
                    continue;
                }
                let col_start = if row == sr { sc } else { 0 };
                let col_end = if row == er { ec } else { term_area.right().saturating_sub(1) };
                for col in col_start..=col_end {
                    if col >= term_area.right() {
                        break;
                    }
                    let cell = &mut buf[(col, row)];
                    let fg = cell.fg;
                    let bg = cell.bg;
                    cell.fg = if bg == Color::Reset { Color::Black } else { bg };
                    cell.bg = if fg == Color::Reset { Color::White } else { fg };
                }
            }
        }

        // --- Render status bar ---
        render_status_bar(frame, cwd, daemon_connected, daemon_timestamp, is_executing, status_area);

        // --- Render completion popup ---
        if completion_popup.is_visible() {
            let (cursor_row, _cursor_col) = parser.screen().cursor_position();
            let prompt_len = cwd.len() + 2; // "{cwd}$ "
            let anchor_x = (prompt_len + input.cursor_position()) as u16;
            let anchor_y = term_area.top() + cursor_row;
            completion_popup.render(frame, anchor_x, anchor_y);
        }
    })?;

    Ok(())
}

/// Render the status bar at the bottom of the screen.
fn render_status_bar(
    frame: &mut Frame,
    cwd: &str,
    daemon_connected: bool,
    daemon_timestamp: u64,
    is_executing: bool,
    area: Rect,
) {
    let width = area.width as usize;

    let daemon_status = if daemon_connected && daemon_timestamp > 0 {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let age_s = now_ms.saturating_sub(daemon_timestamp) / 1000;
        if age_s < 30 {
            "daemon: connected"
        } else {
            "daemon: stale"
        }
    } else if daemon_connected {
        "daemon: connected"
    } else {
        "daemon: offline"
    };

    let running_indicator = if is_executing { " [running]" } else { "" };
    let right = format!(" {} | slate v{} ", daemon_status, VERSION);
    let left = format!(" {}{} ", cwd, running_indicator);

    // Pad the middle so right-side text is right-aligned.
    let pad = width.saturating_sub(left.len() + right.len());
    let bar = format!("{}{}{}", left, " ".repeat(pad), right);

    let paragraph = Paragraph::new(Line::from(Span::styled(
        bar,
        Style::default()
            .fg(Color::Black)
            .bg(Color::White)
            .add_modifier(Modifier::BOLD),
    )));

    frame.render_widget(paragraph, area);
}

/// Find the longest common prefix of a list of strings.
fn longest_common_prefix(strings: &[String]) -> String {
    if strings.is_empty() {
        return String::new();
    }
    let mut prefix = strings[0].clone();
    for s in &strings[1..] {
        let shared: String = prefix
            .chars()
            .zip(s.chars())
            .take_while(|(a, b)| a == b)
            .map(|(a, _)| a)
            .collect();
        prefix = shared;
    }
    prefix
}
