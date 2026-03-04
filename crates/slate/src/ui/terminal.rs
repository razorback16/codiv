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
use crate::shell::bash_coprocess::{BashCoprocess, GitInfo};
use crate::shell::command_index::{classify_input, CommandIndex, InputAction};
use crate::shell::completion_engine::CompletionEngine;

use super::blocks::{Block, BlockRegistry, ToolResultAction};
use super::completion_popup::CompletionPopup;
use super::input::InputLine;
use super::selection::TextSelection;
use super::tool_modal::ToolResultModal;
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
}

/// Default scrollback limit (number of lines retained).
const MAX_SCROLLBACK: usize = 10_000;

/// Helper: write styled text to the vt100 parser using ANSI SGR codes.
fn parser_push_styled(parser: &mut vt100::Parser, text: &str, ansi_prefix: &str) {
    let line = format!("  {}{}\x1b[0m\r\n", ansi_prefix, text);
    parser.process(line.as_bytes());
}

/// Format a token count for compact display: 0, 512, 1.2k, 200k, 1.0M.
fn format_tokens(n: usize) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let s = human_format::Formatter::new()
        .with_decimals(1)
        .with_separator("")
        .format(n as f64);
    // "200.0k" -> "200k", "1.2k" unchanged
    s.trim().replace(".0", "")
}

/// Convert a crossterm key event into raw terminal bytes for PTY forwarding.
///
/// Handles the full range of keys: printable characters (with Ctrl/Alt
/// modifiers), navigation keys, function keys, and editing keys. Returns
/// `None` only for keys that have no byte representation (e.g. bare
/// modifier presses).
fn key_event_to_bytes(code: KeyCode, modifiers: KeyModifiers) -> Option<Vec<u8>> {
    match code {
        KeyCode::Char(ch) => {
            if modifiers.contains(KeyModifiers::CONTROL) {
                // Ctrl+letter → 0x01..0x1A
                let ctrl = (ch.to_ascii_lowercase() as u8).wrapping_sub(b'a').wrapping_add(1);
                if ctrl <= 26 { Some(vec![ctrl]) } else { None }
            } else if modifiers.contains(KeyModifiers::ALT) {
                let mut buf = vec![0x1b]; // ESC prefix
                let mut tmp = [0u8; 4];
                buf.extend_from_slice(ch.encode_utf8(&mut tmp).as_bytes());
                Some(buf)
            } else {
                let mut tmp = [0u8; 4];
                let s = ch.encode_utf8(&mut tmp);
                Some(s.as_bytes().to_vec())
            }
        }
        KeyCode::Enter     => Some(b"\r".to_vec()),
        KeyCode::Backspace => Some(b"\x7f".to_vec()),
        KeyCode::Tab       => Some(b"\t".to_vec()),
        KeyCode::BackTab   => Some(b"\x1b[Z".to_vec()),
        KeyCode::Esc       => Some(b"\x1b".to_vec()),
        KeyCode::Up        => Some(b"\x1b[A".to_vec()),
        KeyCode::Down      => Some(b"\x1b[B".to_vec()),
        KeyCode::Right     => Some(b"\x1b[C".to_vec()),
        KeyCode::Left      => Some(b"\x1b[D".to_vec()),
        KeyCode::Home      => Some(b"\x1b[H".to_vec()),
        KeyCode::End       => Some(b"\x1b[F".to_vec()),
        KeyCode::PageUp    => Some(b"\x1b[5~".to_vec()),
        KeyCode::PageDown  => Some(b"\x1b[6~".to_vec()),
        KeyCode::Insert    => Some(b"\x1b[2~".to_vec()),
        KeyCode::Delete    => Some(b"\x1b[3~".to_vec()),
        KeyCode::F(n) => {
            let s = match n {
                1  => "\x1bOP",    2  => "\x1bOQ",
                3  => "\x1bOR",    4  => "\x1bOS",
                5  => "\x1b[15~",  6  => "\x1b[17~",
                7  => "\x1b[18~",  8  => "\x1b[19~",
                9  => "\x1b[20~",  10 => "\x1b[21~",
                11 => "\x1b[23~",  12 => "\x1b[24~",
                _ => return None,
            };
            Some(s.as_bytes().to_vec())
        }
        _ => None,
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
    let mut cwd = initial_cwd;
    let mut client = client;
    let mut prompt_is_live = false;

    // Welcome message.
    parser.process(format!("\x1b[90mslate v{} — type 'exit' to quit\x1b[0m\r\n\r\n", VERSION).as_bytes());

    // --- Event loop ---
    let result = event_loop(
        &mut term,
        &mut parser,
        &mut scroll_offset,
        &mut input,
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
fn spawn_crossterm_reader() -> crossbeam_channel::Receiver<crossterm::event::Event> {
    let (tx, rx) = crossbeam_channel::unbounded();
    std::thread::Builder::new()
        .name("crossterm-reader".into())
        .spawn(move || loop {
            match crossterm::event::poll(Duration::from_millis(100)) {
                Ok(true) => match crossterm::event::read() {
                    Ok(evt) => {
                        if tx.send(evt).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                },
                Ok(false) => {}
                Err(_) => break,
            }
        })
        .expect("spawn crossterm reader");
    rx
}

/// Compute the minimum timeout until the next timer-driven action.
///
/// Returns the shortest of:
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

/// Process PTY output bytes for a pending command: feed raw bytes directly
/// to the VT100 parser and accumulate for sentinel detection.
fn process_pty_bytes(bytes: &[u8], pending: &mut PendingCommand, parser: &mut vt100::Parser) {
    parser.process(bytes);
    let text = String::from_utf8_lossy(bytes);
    pending.accumulated.push_str(&text);
    pending.last_activity = Instant::now();
}

/// Get the absolute scrollback line number (scrollback_len + cursor_row).
fn get_scrollback_line(parser: &vt100::Parser) -> u64 {
    let screen = parser.screen();
    let (cursor_row, _) = screen.cursor_position();
    let scrollback_len = screen.scrollback() as u64;
    scrollback_len + cursor_row as u64
}

/// Process a single daemon message, updating parser state.
fn handle_daemon_message(
    msg: ipc_messages::DaemonMessage,
    parser: &mut vt100::Parser,
    md_stream: &mut MarkdownStream,
    agent_streaming: &mut bool,
    ai_separator_emitted: &mut bool,
    last_daemon_timestamp: &mut u64,
    model_alias: &mut String,
    context_usage: &mut (usize, usize),
    _cwd: &str,
    tracker: &mut BlockRegistry,
    ai_start_scrollback: &mut Option<u64>,
) {
    match msg {
        ipc_messages::DaemonMessage::AgentStreamChunk {
            request_id: _,
            chunk,
        } => {
            match chunk {
                ipc_messages::StreamChunk::Text(t) => {
                    if !*ai_separator_emitted {
                        parser.process(b"\r\n");
                        *ai_separator_emitted = true;
                        *ai_start_scrollback = Some(get_scrollback_line(parser));
                    }
                    if let Some(ansi) = md_stream.push(&t) {
                        parser.process(&ansi);
                    }
                }
                ipc_messages::StreamChunk::Reasoning(t) => {
                    if !*ai_separator_emitted {
                        parser.process(b"\r\n");
                        *ai_separator_emitted = true;
                        *ai_start_scrollback = Some(get_scrollback_line(parser));
                    }
                    let t = t.replace('\n', "\r\n");
                    parser.process(t.as_bytes());
                }
                ipc_messages::StreamChunk::ToolCall { name, arguments } => {
                    // Flush any buffered markdown text so it appears before the tool call.
                    let pending = md_stream.finish();
                    if !pending.is_empty() {
                        parser.process(&pending);
                        parser.process(b"\r\n");
                    }
                    md_stream.reset();
                    tracker.record_tool_call(&name, &arguments);
                }
                ipc_messages::StreamChunk::ToolResult { name, result } => {
                    {
                        let will_merge = tracker.will_merge_edit(&name);
                        if !will_merge {
                            parser.process(b"\r\n");
                        }
                        let scrollback_line = get_scrollback_line(parser);
                        match tracker.record_tool_result(&name, &result, scrollback_line) {
                            ToolResultAction::Merged => {
                                // Block merged into previous — update the summary
                                // line in the VT100 buffer by moving cursor up and
                                // rewriting the line.
                                let summary = tracker
                                    .last_tool_block_mut()
                                    .map(|tb| tb.summary.clone());
                                if let Some(summary) = summary {
                                    parser.process(b"\x1b[A\r\x1b[K");
                                    let line = format!("  \x1b[32m{}\x1b[0m\r\n", summary);
                                    parser.process(line.as_bytes());
                                }
                            }
                            ToolResultAction::Summary { header, summary } => {
                                // Write 2-line block to VT100.
                                // Header: bold green ● ToolName(args)
                                let header_line = format!("  \x1b[1m\x1b[32m{}\x1b[0m\r\n", header);
                                parser.process(header_line.as_bytes());
                                // Summary: colored based on tool
                                let is_bash_error = name.eq_ignore_ascii_case("bash")
                                    && !summary.contains("exit 0");
                                let color = if is_bash_error { "\x1b[31m" } else { "\x1b[32m" };
                                let summary_line = format!("  {}{}\x1b[0m\r\n", color, summary);
                                parser.process(summary_line.as_bytes());
                            }
                        }
                    }
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
                let ai_end = get_scrollback_line(parser);
                if let Some(start) = ai_start_scrollback.take() {
                    let line_count = (ai_end.saturating_sub(start)) as u16;
                    if line_count > 0 {
                        tracker.record_ai_response(start, line_count);
                    }
                }
                parser.process(b"\r\n"); // blank separator after AI response
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
        ipc_messages::DaemonMessage::AgentMeta {
            model_alias: alias,
            total_tokens,
            context_window,
        } => {
            *model_alias = alias;
            *context_usage = (total_tokens, context_window);
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
    let mut ai_separator_emitted = false;
    let mut cmd_start_scrollback: Option<u64> = None;
    let mut ai_start_scrollback: Option<u64> = None;
    let mut git_info: Option<GitInfo> = bash.capture_git_info();
    let mut model_alias = String::new();
    let mut context_usage: (usize, usize) = (0, 0);
    let mut md_stream = MarkdownStream::new(term.size().map(|s| s.width).unwrap_or(80));
    let mut tracker = BlockRegistry::new();
    let mut tool_result_modal = ToolResultModal::new();

    // Start background initialization (non-blocking) so the first Tab
    // press is fast without freezing the UI at startup.
    completion_engine.start_init(bash);

    // Spawn crossterm reader thread for channelized terminal input.
    let crossterm_rx = spawn_crossterm_reader();

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
            render_frame(term, parser, input, cwd, daemon_connected, last_daemon_timestamp, *scroll_offset, prompt_is_live, is_executing, agent_streaming, &completion_popup, &selection, git_info.as_ref(), &model_alias, context_usage, &tracker, &tool_result_modal)?;
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
                                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                                    // fall through
                                } else if let Some(bytes) = key_event_to_bytes(key.code, key.modifiers) {
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
                                    // --- Block navigation (Shift+Up/Down) ---
                                    let mut block_handled = false;
                                    match (key.code, key.modifiers) {
                                        (KeyCode::Up, m) if m.contains(KeyModifiers::SHIFT) => {
                                            if tracker.focused_index().is_some() {
                                                tracker.focus_prev();
                                            } else {
                                                tracker.focus_last();
                                            }
                                            block_handled = true;
                                        }
                                        (KeyCode::Down, m) if m.contains(KeyModifiers::SHIFT) => {
                                            if tracker.focused_index().is_some() {
                                                tracker.focus_next();
                                            }
                                            block_handled = true;
                                        }
                                        (KeyCode::Enter, _) if tracker.focused_index().is_some() && pending_command.is_none() => {
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

                                            // Empty input — spring back, don't create a prompt block
                                            if raw_input.trim().is_empty() {
                                                *scroll_offset = 0;
                                                parser.screen_mut().set_scrollback(0);
                                            } else {
                                                // Prompt text already on screen from last render_frame.
                                                // Record position, then advance cursor past it.
                                                let scrollback_line = get_scrollback_line(parser);
                                                parser.process(b"\r\n");
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
                                                            let query = raw_input.trim_start().strip_prefix('?').unwrap_or(&raw_input);
                                                            if send_agent_request(c, query, &cwd) {
                                                                agent_streaming = true;
                                                                ai_separator_emitted = false;
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
                                                        let rows = term_size.height.saturating_sub(1).max(1);
                                                        let cols = term_size.width.max(1);
                                                        *parser = vt100::Parser::new(rows, cols, MAX_SCROLLBACK);
                                                        *scroll_offset = 0;
                                                        *prompt_is_live = false;
                                                        tracker.clear();
                                                        tool_result_modal.close();
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
                                                        tracker.clear();
                                                        tool_result_modal.close();
                                                        parser_push_styled(
                                                            parser,
                                                            &format!("slate v{} — type 'exit' to quit", crate::VERSION),
                                                            "\x1b[90m",
                                                        );
                                                        parser.process(b"\r\n"); // blank separator line after header
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
                                                                ai_separator_emitted = false;
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
                    handle_daemon_message(msg, parser, &mut md_stream, &mut agent_streaming, &mut ai_separator_emitted, &mut last_daemon_timestamp, &mut model_alias, &mut context_usage, cwd, &mut tracker, &mut ai_start_scrollback);
                    // Drain any additional daemon messages that arrived.
                    while let Ok(msg2) = daemon_rx.try_recv() {
                        handle_daemon_message(msg2, parser, &mut md_stream, &mut agent_streaming, &mut ai_separator_emitted, &mut last_daemon_timestamp, &mut model_alias, &mut context_usage, cwd, &mut tracker, &mut ai_start_scrollback);
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
            } else if pending.last_activity.elapsed() > Duration::from_secs(300) {
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
    git_info: Option<&GitInfo>,
    model_alias: &str,
    context_usage: (usize, usize),
    tracker: &BlockRegistry,
    tool_result_modal: &ToolResultModal,
) -> Result<(), Box<dyn std::error::Error>> {
    // Write live prompt into the vt100 parser (only when scrolled to bottom
    // and no command is currently executing or agent streaming, and not in alt screen).
    let in_alt_screen = parser.screen().alternate_screen();
    if scroll_offset == 0 && !is_executing && !agent_streaming && !in_alt_screen {
        let input_text = input.content();

        // Write the input text at column 2, leaving cols 0-1 for the `> ` overlay.
        parser.process(
            format!("\r\x1b[K  {}", input_text).as_bytes(),
        );

        // Position cursor: move back from end if cursor isn't at end of input.
        let target_col = input.cursor_position();
        let current_col = input_text.len();
        if current_col > target_col {
            parser.process(format!("\x1b[{}D", current_col - target_col).as_bytes());
        }

        *prompt_is_live = true;
    }

    term.draw(|frame| {
        let area = frame.area();

        if in_alt_screen {
            // Fullscreen: render PseudoTerminal over the entire area (no status bar).
            let pseudo_term = PseudoTerminal::new(parser.screen())
                .cursor(PtCursor::default().visibility(true));
            frame.render_widget(pseudo_term, area);
        } else {
            // Normal: terminal area + status bar.
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

            // --- Render `>` prompt gutter for all visible prompt blocks ---
            {
                let scrollback_len = parser.screen().scrollback() as u64;
                let screen_rows = parser.screen().size().0 as u64;
                let abs_bottom = scrollback_len + screen_rows - scroll_offset as u64;
                let abs_top = abs_bottom.saturating_sub(screen_rows);
                let buf = frame.buffer_mut();
                for block in tracker.blocks() {
                    if let Block::Prompt(pb) = block {
                        if pb.scrollback_line >= abs_top && pb.scrollback_line < abs_bottom {
                            let row = (pb.scrollback_line - abs_top) as u16 + term_area.top();
                            if row < term_area.bottom() {
                                buf[(term_area.left(), row)].set_char('>').set_fg(Color::Cyan);
                            }
                        }
                    }
                }
                // Also draw `>` for the live (in-progress) prompt
                if *prompt_is_live && scroll_offset == 0 {
                    let (cursor_row, _) = parser.screen().cursor_position();
                    let row = term_area.top() + cursor_row;
                    if row < term_area.bottom() {
                        buf[(term_area.left(), row)].set_char('>').set_fg(Color::Cyan);
                    }
                }
            }

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
            render_status_bar(frame, cwd, daemon_connected, daemon_timestamp, is_executing, status_area, git_info, model_alias, context_usage);

            // --- Render completion popup ---
            if completion_popup.is_visible() {
                let (cursor_row, _cursor_col) = parser.screen().cursor_position();
                let anchor_x = input.cursor_position() as u16;
                let anchor_y = term_area.top() + cursor_row;
                completion_popup.render(frame, anchor_x, anchor_y);
            }

            // --- Render block selection overlay ---
            if let Some(focused) = tracker.focused() {
                let (scrollback_line, line_count) = match focused {
                    Block::Tool(tb) => (tb.scrollback_line, tb.line_count),
                    Block::Prompt(pb) => (pb.scrollback_line, 1),
                    Block::CmdResponse(cb) => (cb.scrollback_line, cb.line_count),
                    Block::AiResponse(ab) => (ab.scrollback_line, ab.line_count),
                };
                let scrollback_len = parser.screen().scrollback() as u64;
                let screen_rows = parser.screen().size().0 as u64;
                let abs_bottom = scrollback_len + screen_rows - scroll_offset as u64;
                let abs_top = abs_bottom.saturating_sub(screen_rows);
                if scrollback_line >= abs_top && scrollback_line < abs_bottom {
                    let screen_row = (scrollback_line - abs_top) as u16 + term_area.top();
                    let top_rule = screen_row.saturating_sub(1);
                    let bottom_rule = screen_row + line_count;
                    let buf = frame.buffer_mut();
                    for row in [top_rule, bottom_rule] {
                        if row >= term_area.top() && row < term_area.bottom() {
                            let is_blank = (term_area.left()..term_area.right())
                                .all(|col| {
                                    let ch = buf[(col, row)].symbol();
                                    ch == " " || ch == ""
                                });
                            if is_blank {
                                for col in term_area.left()..term_area.right() {
                                    let cell = &mut buf[(col, row)];
                                    cell.set_char('\u{2500}');
                                    cell.set_fg(Color::DarkGray);
                                }
                            }
                        }
                    }
                    // Show "(press Enter to expand)" hint for ToolBlocks
                    if let Block::Tool(_) = focused {
                        let hint = " (press Enter to expand)";
                        let hint_row = screen_row + 1; // summary line
                        if hint_row >= term_area.top() && hint_row < term_area.bottom() {
                            // Find end of existing text
                            let buf = frame.buffer_mut();
                            let mut text_end = term_area.left();
                            for col in term_area.left()..term_area.right() {
                                let ch = buf[(col, hint_row)].symbol();
                                if ch != " " && ch != "" {
                                    text_end = col + 1;
                                }
                            }
                            for (i, ch) in hint.chars().enumerate() {
                                let col = text_end + i as u16;
                                if col < term_area.right() {
                                    buf[(col, hint_row)]
                                        .set_char(ch)
                                        .set_fg(Color::DarkGray);
                                }
                            }
                        }
                    }
                }
            } else if *prompt_is_live && scroll_offset == 0 {
                // Live prompt is implicitly selected — draw ───── rules
                let (cursor_row, _) = parser.screen().cursor_position();
                let screen_row = term_area.top() + cursor_row;
                let top_rule = screen_row.saturating_sub(1);
                let bottom_rule = screen_row + 1;
                let buf = frame.buffer_mut();
                for row in [top_rule, bottom_rule] {
                    if row >= term_area.top() && row < term_area.bottom() {
                        let is_blank = (term_area.left()..term_area.right())
                            .all(|col| {
                                let ch = buf[(col, row)].symbol();
                                ch == " " || ch == ""
                            });
                        if is_blank {
                            for col in term_area.left()..term_area.right() {
                                buf[(col, row)].set_char('\u{2500}').set_fg(Color::DarkGray);
                            }
                        }
                    }
                }
            }

            // --- Render tool result modal ---
            if tool_result_modal.is_visible() {
                tool_result_modal.render(frame, area);
            }
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
    git_info: Option<&GitInfo>,
    model_alias: &str,
    context_usage: (usize, usize),
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
    let model_part = if !model_alias.is_empty() {
        format!("{} {}/{} | ", model_alias, format_tokens(context_usage.0), format_tokens(context_usage.1))
    } else {
        String::new()
    };
    let right = format!(" {}{} | slate v{} ", model_part, daemon_status, VERSION);
    let left = match git_info {
        Some(info) => {
            let branch_part = format!("({})", info.branch);
            let stats_part = if info.files_changed > 0 {
                format!(" ~{} +{} -{}", info.files_changed, info.insertions, info.deletions)
            } else {
                String::new()
            };
            format!(" {} {}{}{} ", cwd, branch_part, stats_part, running_indicator)
        }
        None => format!(" {}{} ", cwd, running_indicator),
    };

    // Pad the middle so right-side text is right-aligned.
    let pad = width.saturating_sub(left.len() + right.len());
    let bar = format!("{}{}{}", left, " ".repeat(pad), right);

    let paragraph = Paragraph::new(Line::from(Span::styled(
        bar,
        Style::default()
            .add_modifier(Modifier::REVERSED | Modifier::BOLD),
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
