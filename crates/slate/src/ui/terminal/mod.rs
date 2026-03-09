//! Main terminal UI — event loop, rendering, and command dispatch.
//!
//! This is the entry point for the ratatui-based terminal interface. It sets
//! up crossterm raw mode, runs a synchronous event loop, and renders a linear
//! scroll-down layout consisting of a VT100 pseudoterminal, an input line,
//! and a status bar.

pub(super) mod animation;
mod daemon;
mod event_loop;
mod input;
pub(crate) mod io;
mod render;
mod state;
mod utils;

use self::state::MAX_SCROLLBACK;
pub(crate) use event_loop::event_loop;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::ipc::client::SlatedClient;
use crate::shell::bash_coprocess::BashCoprocess;
use crate::ui::input::InputLine;
use crate::VERSION;

pub use self::io::TerminalColors;

pub(super) const STATUS_BAR_HEIGHT: u16 = 1;
pub(super) const PROMPT_GUTTER_WIDTH: u16 = 2;

#[inline]
pub(super) fn parser_rows_from_term_height(term_height: u16) -> u16 {
    term_height.saturating_sub(STATUS_BAR_HEIGHT).max(1)
}

#[inline]
pub(super) fn parser_cols_from_term_width(term_width: u16) -> u16 {
    term_width.saturating_sub(PROMPT_GUTTER_WIDTH).max(1)
}

/// Run the main terminal UI.
///
/// This function takes ownership of the terminal, enters the alternate screen
/// with raw mode, and runs a synchronous event loop until the user exits.
pub fn run(
    bash: &mut BashCoprocess,
    shutdown: Arc<AtomicBool>,
    initial_cwd: String,
    client: Option<SlatedClient>,
    terminal_colors: TerminalColors,
) -> Result<(), Box<dyn std::error::Error>> {
    // --- Terminal setup ---
    terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut term = Terminal::new(backend)?;

    // --- State ---
    let term_size = term.size()?;
    let parser_rows = parser_rows_from_term_height(term_size.height);
    let parser_cols = parser_cols_from_term_width(term_size.width);
    let mut parser = vt100::Parser::new(parser_rows, parser_cols, MAX_SCROLLBACK);
    let mut scroll_offset: usize = 0;

    let mut input = InputLine::new();
    let mut cwd = initial_cwd;
    let mut client = client;
    let mut prompt_is_live = false;

    // Welcome message.
    parser.process(
        format!(
            "\x1b[90mslate v{} — type 'exit' to quit\x1b[0m\r\n\r\n",
            VERSION
        )
        .as_bytes(),
    );

    // --- Event loop ---
    let result = event_loop(
        &mut term,
        &mut parser,
        &mut scroll_offset,
        &mut input,
        bash,
        &shutdown,
        &mut cwd,
        &mut client,
        &mut prompt_is_live,
        &terminal_colors,
    );

    // --- Cleanup (always runs) ---
    terminal::disable_raw_mode()?;
    execute!(
        term.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    term.show_cursor()?;

    result
}
