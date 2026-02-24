//! Main terminal UI — event loop, rendering, and command dispatch.
//!
//! This is the entry point for the ratatui-based terminal interface. It sets
//! up crossterm raw mode, runs a synchronous event loop, and renders a linear
//! scroll-down layout consisting of a VT100 pseudoterminal, an input line,
//! and a status bar.

use std::io;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind};
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
use crate::shell::interactive::InteractiveSession;

use super::input::InputLine;

/// Default scrollback limit (number of lines retained).
const MAX_SCROLLBACK: usize = 10_000;

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
    // Parser rows = total height - 1 (prompt) - 1 (status bar)
    let parser_rows = term_size.height.saturating_sub(2).max(1);
    let parser_cols = term_size.width.max(1);
    let mut parser = vt100::Parser::new(parser_rows, parser_cols, MAX_SCROLLBACK);
    let mut scroll_offset: usize = 0;

    let mut input = InputLine::new();
    let mut interactive_session = InteractiveSession::new();
    let mut cwd = initial_cwd;
    let mut client = client;

    // Welcome message.
    parser_push_styled(&mut parser, "slate v0.1.0 — type 'exit' to quit", "\x1b[90m");

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
) -> Result<(), Box<dyn std::error::Error>> {
    let mut last_heartbeat_sent = Instant::now();
    let mut last_daemon_timestamp: i64 = 0;
    let session_id = format!("slate-{}", std::process::id());

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let daemon_connected = client.as_ref().map_or(false, |c| c.is_connected());

        // Check for daemon disconnect.
        if !daemon_connected && client.is_some() {
            *client = None;
        }

        render_frame(term, parser, input, cwd, daemon_connected, last_daemon_timestamp)?;

        // Poll for events with 50ms timeout.
        if event::poll(Duration::from_millis(50))? {
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
                        _ => {}
                    }
                    continue;
                }
                Event::Resize(cols, rows) => {
                    let parser_rows = rows.saturating_sub(2).max(1);
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

            match (key.code, key.modifiers) {
                // --- Ctrl combos ---
                (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => {
                    // Send SIGINT to the bash child process group.
                    let _ = bash.send_signal(nix::sys::signal::Signal::SIGINT);
                    parser_push_styled(parser, "^C", "\x1b[90m");
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
                (KeyCode::Enter, _) => {
                    let raw_input = input.submit();
                    let action = classify_input(&raw_input, command_index);

                    match action {
                        InputAction::Empty => {
                            // Just push another prompt line.
                            parser_push_styled(
                                parser,
                                &format!("{}$ ", cwd),
                                "\x1b[1;32m",
                            );
                        }

                        InputAction::Exit => {
                            break;
                        }

                        InputAction::AiQuery => {
                            parser_push_styled(
                                parser,
                                &format!("{}$ {}", cwd, raw_input),
                                "\x1b[1;36m",
                            );
                            parser_push_styled(
                                parser,
                                "AI mode not yet available (Phase 2)",
                                "\x1b[90m",
                            );
                        }

                        InputAction::Interactive => {
                            parser_push_styled(
                                parser,
                                &format!("{}$ {}", cwd, raw_input),
                                "\x1b[1;36m",
                            );

                            // Suspend the TUI for interactive passthrough.
                            terminal::disable_raw_mode()?;
                            execute!(term.backend_mut(), DisableMouseCapture, LeaveAlternateScreen)?;

                            // Write the command to bash and enter interactive
                            // session on its PTY.
                            let cmd_with_newline = format!("{}\n", raw_input);
                            let master_raw_fd = bash.master_fd().as_raw_fd();

                            // Write the command to the PTY so bash executes it.
                            let _ =
                                nix::unistd::write(bash.master_fd(), cmd_with_newline.as_bytes());

                            interactive_session.enter(master_raw_fd);

                            // Resume the TUI.
                            execute!(term.backend_mut(), EnterAlternateScreen, EnableMouseCapture)?;
                            terminal::enable_raw_mode()?;
                            term.clear()?;

                            // Update cwd — the interactive command may have
                            // changed the directory.
                            *cwd = bash.capture_cwd();

                            parser_push_styled(
                                parser,
                                "(interactive session ended)",
                                "\x1b[90m",
                            );
                        }

                        InputAction::Execute => {
                            parser_push_styled(
                                parser,
                                &format!("{}$ {}", cwd, raw_input),
                                "\x1b[1;36m",
                            );

                            let result = bash.execute(&raw_input, 30_000);

                            if !result.output.is_empty() {
                                // Feed raw output (with ANSI codes) directly
                                // to the vt100 parser. It handles all escape
                                // sequences correctly.
                                parser.process(result.output.as_bytes());
                                // Ensure output ends with a newline so the
                                // next line starts at column 0.
                                if !result.output.ends_with('\n') {
                                    parser.process(b"\r\n");
                                }
                            }

                            if result.exit_code != 0 {
                                parser_push_styled(
                                    parser,
                                    &format!("exit code: {}", result.exit_code),
                                    "\x1b[31m",
                                );
                            }

                            // Update cwd — the command may have been `cd`.
                            *cwd = bash.capture_cwd();
                        }

                        InputAction::NotFound(word) => {
                            parser_push_styled(
                                parser,
                                &format!("{}$ {}", cwd, raw_input),
                                "\x1b[1;36m",
                            );
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
                (KeyCode::Tab, _) => {
                    let content = input.content().to_string();
                    let word = content
                        .split_whitespace()
                        .last()
                        .unwrap_or("")
                        .to_string();

                    let completions = command_index.complete(&word);
                    input.complete(&completions);
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
                        request_id: _,
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
    parser: &vt100::Parser,
    input: &InputLine,
    cwd: &str,
    daemon_connected: bool,
    daemon_timestamp: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    term.draw(|frame| {
        let area = frame.area();

        // Layout: terminal area (flexible) | prompt (1) | status bar (1)
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),    // PseudoTerminal
                Constraint::Length(1), // Active prompt + input
                Constraint::Length(1), // Status bar
            ])
            .split(area);

        let term_area = chunks[0];
        let prompt_area = chunks[1];
        let status_area = chunks[2];

        // --- Render pseudoterminal ---
        let pseudo_term = PseudoTerminal::new(parser.screen())
            .cursor(PtCursor::default().visibility(false));
        frame.render_widget(pseudo_term, term_area);

        // --- Render active prompt + input ---
        render_prompt(frame, input, cwd, prompt_area);

        // --- Render status bar ---
        render_status_bar(frame, cwd, daemon_connected, daemon_timestamp, status_area);
    })?;

    Ok(())
}

/// Render the active prompt line with cursor.
fn render_prompt(frame: &mut Frame, input: &InputLine, cwd: &str, area: Rect) {
    let prompt = format!("{}$ ", cwd);
    let prompt_span = Span::styled(
        prompt.clone(),
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    );
    let content_span = Span::raw(input.content().to_string());
    let line = Line::from(vec![prompt_span, content_span]);
    frame.render_widget(Paragraph::new(line), area);

    // Position the cursor.
    let cursor_x = area.x + prompt.len() as u16 + input.cursor_position() as u16;
    let cursor_x = cursor_x.min(area.x + area.width.saturating_sub(1));
    frame.set_cursor_position((cursor_x, area.y));
}

/// Render the status bar at the bottom of the screen.
fn render_status_bar(
    frame: &mut Frame,
    cwd: &str,
    daemon_connected: bool,
    daemon_timestamp: i64,
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

    let right = format!(" {} | slate v0.1.0 ", daemon_status);
    let left = format!(" {} ", cwd);

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
