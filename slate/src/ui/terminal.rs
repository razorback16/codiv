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

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
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

/// Run the main terminal UI.
///
/// This function takes ownership of the terminal, enters the alternate screen
/// with raw mode, and runs a synchronous event loop until the user exits.
pub fn run(
    bash: &BashCoprocess,
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

/// The main event loop. Factored out so cleanup always runs in `run()`.
fn event_loop(
    term: &mut Terminal<CrosstermBackend<io::Stdout>>,
    parser: &mut vt100::Parser,
    scroll_offset: &mut usize,
    input: &mut InputLine,
    interactive_session: &mut InteractiveSession,
    bash: &BashCoprocess,
    command_index: &CommandIndex,
    shutdown: &Arc<AtomicBool>,
    cwd: &mut String,
    client: &mut Option<SlatedClient>,
    prompt_is_live: &mut bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut last_heartbeat_sent = Instant::now();
    let mut last_daemon_timestamp: i64 = 0;
    let session_id = format!("slate-{}", std::process::id());
    let mut pending_command: Option<PendingCommand> = None;
    let mut completion_engine = CompletionEngine::new();
    let mut completion_popup = CompletionPopup::new();
    let mut selection = TextSelection::new();
    let mut clipboard = arboard::Clipboard::new().ok();

    // Start background initialization (non-blocking) so the first Tab
    // press is fast without freezing the UI at startup.
    completion_engine.start_init(bash);

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let daemon_connected = client.as_ref().map_or(false, |c| c.is_connected());

        // Check for daemon disconnect.
        if !daemon_connected && client.is_some() {
            *client = None;
        }

        // --- Poll pending command for output ---
        if let Some(ref mut pending) = pending_command {
            let bytes = bash.try_read();
            if !bytes.is_empty() {
                let text = String::from_utf8_lossy(&bytes);
                pending.accumulated.push_str(&text);
                pending.display_buf.push_str(&text);
                pending.last_activity = Instant::now();
                pending.last_output = Instant::now();

                // Process complete lines through the noise filter.
                while let Some(newline_pos) = pending.display_buf.find('\n') {
                    let line = pending.display_buf[..newline_pos].to_string();
                    pending.display_buf = pending.display_buf[newline_pos + 1..].to_string();
                    if !is_sentinel_noise(&line, &pending.command, &pending.sentinel) {
                        let out = format!("{}\n", line);
                        parser.process(out.as_bytes());
                    }
                }
            }

            // Flush incomplete line buffer after timeout (shows prompts like
            // sudo password, apt Y/n that don't end with \n).
            if !pending.display_buf.is_empty()
                && pending.last_output.elapsed() > Duration::from_millis(DISPLAY_FLUSH_MS)
            {
                let buf = std::mem::take(&mut pending.display_buf);
                if !is_sentinel_noise(&buf, &pending.command, &pending.sentinel) {
                    parser.process(buf.as_bytes());
                }
            }

            if let Some(result) = BashCoprocess::check_complete(
                &pending.accumulated,
                &pending.command,
                &pending.sentinel,
            ) {
                // Flush any remaining display buffer.
                if let Some(ref mut p) = pending_command {
                    let buf = std::mem::take(&mut p.display_buf);
                    if !buf.is_empty()
                        && !is_sentinel_noise(&buf, &p.command, &p.sentinel)
                    {
                        parser.process(buf.as_bytes());
                    }
                }
                // Only show exit code if non-zero.
                if result.exit_code != 0 {
                    parser_push_styled(
                        parser,
                        &format!("exit code: {}", result.exit_code),
                        "\x1b[31m",
                    );
                }
                *cwd = bash.capture_cwd();
                pending_command = None;
                *scroll_offset = 0;
                parser.screen_mut().set_scrollback(0);
            } else if pending.last_activity.elapsed() > Duration::from_secs(300) {
                // Timeout after 5 minutes of inactivity (no output or keystrokes).
                bash.send_interrupt();
                bash.drain_for(100);
                parser_push_styled(parser, "command timed out (no activity for 5m)", "\x1b[31m");
                pending_command = None;
                *scroll_offset = 0;
                parser.screen_mut().set_scrollback(0);
            }
        }

        let is_executing = pending_command.is_some();

        // Poll completion engine background init when the coprocess is free.
        if pending_command.is_none() {
            completion_engine.poll_init(bash);
        }

        render_frame(term, parser, input, cwd, daemon_connected, last_daemon_timestamp, *scroll_offset, prompt_is_live, is_executing, &completion_popup, &selection)?;

        // Poll for events with a short timeout (10ms when executing for
        // responsive output polling, 50ms otherwise).
        let poll_ms = if is_executing { 10 } else { 50 };
        if event::poll(Duration::from_millis(poll_ms))? {
            let evt = event::read()?;

            // Handle mouse events (scroll wheel).
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
                    continue;
                }
                Event::Resize(cols, rows) => {
                    let parser_rows = rows.saturating_sub(1).max(1);
                    let parser_cols = (*cols).max(1);
                    parser.screen_mut().set_size(parser_rows, parser_cols);
                    continue;
                }
                _ => {}
            }

            // We only care about key press events (not release/repeat on
            // platforms that report them).
            let key = match evt {
                Event::Key(key) if key.kind == KeyEventKind::Press => key,
                _ => continue,
            };

            // Clear text selection on any keypress.
            selection.clear();

            // --- Forward keystrokes to PTY when a command is executing ---
            // This allows the user to respond to prompts (sudo password,
            // apt Y/n, `read` input, etc.). Ctrl+C falls through to the
            // existing interrupt handler below.
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
                        continue;
                    }
                    (KeyCode::Char(ch), _) => {
                        let mut buf = [0u8; 4];
                        let s = ch.encode_utf8(&mut buf);
                        bash.send_bytes(s.as_bytes());
                        if let Some(ref mut p) = pending_command {
                            p.last_activity = Instant::now();
                        }
                        continue;
                    }
                    (KeyCode::Backspace, _) => {
                        bash.send_bytes(b"\x7f");
                        if let Some(ref mut p) = pending_command {
                            p.last_activity = Instant::now();
                        }
                        continue;
                    }
                    _ => {
                        // Consume other keys silently (arrows/tab don't
                        // make sense for stdin prompts).
                        continue;
                    }
                }
            }

            // --- Completion popup key interception ---
            if completion_popup.is_visible() {
                match key.code {
                    KeyCode::Up => {
                        completion_popup.select_prev();
                        continue;
                    }
                    KeyCode::Down => {
                        completion_popup.select_next();
                        continue;
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
                        continue;
                    }
                    KeyCode::Esc => {
                        completion_popup.dismiss();
                        continue;
                    }
                    _ => {
                        // Any other key dismisses popup and falls through to normal handling.
                        completion_popup.dismiss();
                    }
                }
            }

            match (key.code, key.modifiers) {
                // --- Ctrl combos ---
                (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => {
                    if pending_command.is_some() {
                        // Interrupt the running command via PTY Ctrl-C
                        // (delivers SIGINT to entire foreground process group).
                        bash.send_interrupt();
                        bash.drain_for(100);
                        pending_command = None;
                    } else {
                        // No command running — signal bash directly.
                        let _ = bash.send_signal(nix::sys::signal::Signal::SIGINT);
                    }
                    parser.process(b"^C\r\n");
                    *prompt_is_live = false;
                    input.clear();
                    // Auto-scroll to bottom.
                    *scroll_offset = 0;
                    parser.screen_mut().set_scrollback(0);
                }

                (KeyCode::Char('d'), m)
                    if m.contains(KeyModifiers::CONTROL) && input.content().is_empty() =>
                {
                    // Ctrl+D on empty line -> exit.
                    break;
                }

                // --- Enter: submit input ---
                (KeyCode::Enter, _) if pending_command.is_none() => {
                    let raw_input = input.submit();

                    // Commit the current prompt line (already visible in parser).
                    parser.process(b"\r\n");
                    *prompt_is_live = false;

                    let action = classify_input(&raw_input, command_index);

                    match action {
                        InputAction::Empty => {
                            // Nothing to do — next render cycle writes the prompt.
                        }

                        InputAction::Exit => {
                            break;
                        }

                        InputAction::AiQuery => {
                            parser_push_styled(
                                parser,
                                "AI mode not yet available (Phase 2)",
                                "\x1b[90m",
                            );
                        }

                        InputAction::Interactive => {
                            // Snapshot the coprocess environment before suspending
                            // the TUI, so the interactive command inherits env/cwd.
                            let env = bash.capture_env();
                            let interactive_cwd = cwd.clone();

                            // Suspend the TUI for interactive passthrough.
                            terminal::disable_raw_mode()?;
                            execute!(term.backend_mut(), DisableMouseCapture, LeaveAlternateScreen)?;

                            // Spawn a dedicated PTY for the interactive command.
                            // This forks a new child process sized to the real
                            // terminal dimensions. When the command exits, the
                            // PTY HUPs and spawn_and_enter returns cleanly.
                            interactive_session.spawn_and_enter(
                                &raw_input,
                                &env,
                                &interactive_cwd,
                            );

                            // Reset terminal state before re-entering TUI.
                            // RIS (Reset to Initial State) clears any residual
                            // state left by the interactive program (e.g. vim).
                            let _ = std::io::Write::write_all(
                                &mut std::io::stdout(),
                                b"\x1bc",
                            );
                            let _ = std::io::Write::flush(&mut std::io::stdout());

                            // Resume the TUI.
                            execute!(term.backend_mut(), EnterAlternateScreen, EnableMouseCapture)?;
                            terminal::enable_raw_mode()?;
                            term.clear()?;

                            // Push visible content into scrollback, then clear.
                            // Push newlines from the current cursor position (NOT
                            // from the bottom row). From cursor row C in a screen
                            // of R rows, the first R-C newlines just move the
                            // cursor down without scrolling, then subsequent ones
                            // scroll only the actual content rows into scrollback
                            // — no blank rows end up in scrollback.
                            let visible_rows = parser.screen().size().0;
                            for _ in 0..visible_rows {
                                parser.process(b"\r\n");
                            }
                            parser.process(b"\x1b[2J\x1b[H");

                            // Update cwd — the interactive command may have
                            // changed the directory (e.g. via shell escape in vim).
                            *cwd = bash.capture_cwd();

                            parser_push_styled(
                                parser,
                                "(interactive session ended)",
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

                        InputAction::NotFound(word) => {
                            parser_push_styled(
                                parser,
                                &format!("command not found: {}", word),
                                "\x1b[31m",
                            );
                        }
                    }

                    // Auto-scroll to bottom after command execution.
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
                                let suffix =
                                    if std::path::Path::new(candidate).is_dir() {
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
                                // Complete the common prefix first.
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
                    // Ignore chars with Ctrl/Alt modifiers (except Ctrl combos
                    // already handled above).
                    if modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
                        // Unhandled modifier combo — ignore.
                    } else {
                        input.insert(ch);
                        // Auto-scroll to bottom when user types.
                        *scroll_offset = 0;
                        parser.screen_mut().set_scrollback(0);
                    }
                }

                _ => {}
            }
        }

        // Drain daemon messages.
        if let Some(ref c) = client {
            while let Some(msg) = c.try_recv() {
                match msg {
                    ipc_messages::DaemonMessage::CommandOutput {
                        data,
                        is_stderr,
                    } => {
                        let text = String::from_utf8_lossy(&data).to_string();
                        if is_stderr {
                            parser_push_styled(parser, &text, "\x1b[31m");
                        } else {
                            // Feed daemon output through the parser as-is.
                            parser.process(text.as_bytes());
                            if !text.ends_with('\n') {
                                parser.process(b"\r\n");
                            }
                        }
                    }
                    ipc_messages::DaemonMessage::CommandComplete {
                        request_id,
                        exit_code,
                    } => {
                        if exit_code != 0 {
                            parser_push_styled(
                                parser,
                                &format!(
                                    "[daemon] command {} exit code: {}",
                                    request_id, exit_code
                                ),
                                "\x1b[31m",
                            );
                        }
                    }
                    ipc_messages::DaemonMessage::Error {
                        request_id,
                        message,
                    } => {
                        parser_push_styled(
                            parser,
                            &format!("[daemon] error ({}): {}", request_id, message),
                            "\x1b[31m",
                        );
                    }
                    ipc_messages::DaemonMessage::Heartbeat { timestamp } => {
                        last_daemon_timestamp = timestamp;
                    }
                }
            }
        }

        // Periodic heartbeat every 10s.
        if let Some(ref mut c) = client {
            if c.is_connected() && last_heartbeat_sent.elapsed() > Duration::from_secs(10) {
                let hb = ipc_messages::build_heartbeat(&session_id);
                c.send(&hb);
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
    daemon_timestamp: i64,
    scroll_offset: usize,
    prompt_is_live: &mut bool,
    is_executing: bool,
    completion_popup: &CompletionPopup,
    selection: &TextSelection,
) -> Result<(), Box<dyn std::error::Error>> {
    // Write live prompt into the vt100 parser (only when scrolled to bottom
    // and no command is currently executing).
    if scroll_offset == 0 && !is_executing {
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
    daemon_timestamp: i64,
    is_executing: bool,
    area: Rect,
) {
    let width = area.width as usize;

    let daemon_status = if daemon_connected && daemon_timestamp > 0 {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let age_s = (now_ms - daemon_timestamp) / 1000;
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
