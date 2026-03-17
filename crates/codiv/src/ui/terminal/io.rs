use std::io::{Read, Write};
use std::time::{Duration, Instant};

use crossbeam_channel;
use crossterm::event;

use super::state::PendingCommand;
use crate::shell::bash_coprocess::BashCoprocess;

/// Cached terminal foreground/background colors queried from the host terminal.
pub struct TerminalColors {
    pub fg: String, // e.g. "rgb:cccc/cccc/cccc"
    pub bg: String, // e.g. "rgb:1a1a/1a1a/2e2e"
}

impl Default for TerminalColors {
    fn default() -> Self {
        Self {
            fg: "rgb:cccc/cccc/cccc".to_string(),
            bg: "rgb:1a1a/1a1a/2e2e".to_string(),
        }
    }
}

/// Query the host terminal for foreground and background colors.
///
/// This must be called BEFORE crossterm raw mode is entered, because
/// crossterm's raw mode handling may interfere with reading responses.
pub fn query_terminal_colors() -> TerminalColors {
    let mut colors = TerminalColors::default();

    // We need raw mode briefly to read terminal responses.
    if crossterm::terminal::enable_raw_mode().is_err() {
        return colors;
    }

    let mut stdout = std::io::stdout();

    // Query foreground (OSC 10) and background (OSC 11)
    let _ = stdout.write_all(b"\x1b]10;?\x1b\\");
    let _ = stdout.write_all(b"\x1b]11;?\x1b\\");
    let _ = stdout.flush();

    // Read responses using non-blocking I/O on stdin fd 0.
    // We can't use crossterm::event::poll() here because it consumes bytes
    // internally and won't expose raw OSC responses.
    use std::os::unix::io::AsRawFd;
    let stdin = std::io::stdin();
    let fd = stdin.as_raw_fd();

    // Set stdin to non-blocking mode
    let old_flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if old_flags >= 0 {
        unsafe { libc::fcntl(fd, libc::F_SETFL, old_flags | libc::O_NONBLOCK) };
    }

    let mut buf = Vec::with_capacity(256);
    let mut tmp = [0u8; 64];
    let deadline = Instant::now() + Duration::from_millis(200);

    // Poll stdin until timeout or we have both responses.
    while Instant::now() < deadline {
        match stdin.lock().read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                // Check if we have both complete responses
                let s = String::from_utf8_lossy(&buf);
                if parse_osc_color_response(&s, "10").is_some()
                    && parse_osc_color_response(&s, "11").is_some()
                {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => break,
        }
    }

    // Restore blocking mode
    if old_flags >= 0 {
        unsafe { libc::fcntl(fd, libc::F_SETFL, old_flags) };
    }

    let _ = crossterm::terminal::disable_raw_mode();

    // Parse responses from buffer
    let response = String::from_utf8_lossy(&buf);
    if let Some(fg) = parse_osc_color_response(&response, "10") {
        colors.fg = fg;
    }
    if let Some(bg) = parse_osc_color_response(&response, "11") {
        colors.bg = bg;
    }

    colors
}

/// Parse an OSC color response like `\x1b]10;rgb:RRRR/GGGG/BBBB\x1b\\`
fn parse_osc_color_response(data: &str, osc_num: &str) -> Option<String> {
    let prefix = format!("\x1b]{};", osc_num);
    if let Some(start) = data.find(&prefix) {
        let after = &data[start + prefix.len()..];
        // Find the string terminator: either ESC \ or BEL
        let end = after.find("\x1b\\").or_else(|| after.find('\x07'));
        if let Some(end_pos) = end {
            let color = &after[..end_pos];
            if !color.is_empty() {
                return Some(color.to_string());
            }
        }
    }
    None
}

pub(crate) fn process_pty_bytes(
    bytes: &[u8],
    pending: &mut PendingCommand,
    parser: &mut vt100::Parser,
    bash: &mut BashCoprocess,
) {
    // Only render PTY output for user-initiated commands.
    // AI-relayed commands are invisible — the AI summarizes results in its response.
    if pending.ai_execution_id.is_none() {
        parser.process(bytes);
    }
    let text = String::from_utf8_lossy(bytes);
    pending.accumulated.push_str(&text);
    pending.last_activity = Instant::now();

    if pending.ai_execution_id.is_none() {
        respond_to_terminal_queries(bytes, bash, parser);
    }
}

/// Scan raw bytes for terminal capability queries and send appropriate responses.
///
/// Only responds to DSR (cursor position). OSC color queries are intentionally
/// ignored — responding via PTY master is racy and causes garbled output.
fn respond_to_terminal_queries(
    bytes: &[u8],
    bash: &mut BashCoprocess,
    parser: &vt100::Parser,
) {
    if find_subsequence(bytes, b"\x1b[6n").is_some() {
        let (row, col) = parser.screen().cursor_position();
        let response = format!("\x1b[{};{}R", row + 1, col + 1);
        bash.send_bytes(response.as_bytes());
    }
}

/// Find a subsequence in a byte slice.
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Check if bytes contain an OSC query (prefix followed eventually by ST or BEL).
#[cfg(test)]
fn contains_osc_query(bytes: &[u8], prefix: &[u8]) -> bool {
    if let Some(pos) = find_subsequence(bytes, prefix) {
        let after = &bytes[pos + prefix.len()..];
        // Look for string terminator: ESC \ or BEL
        find_subsequence(after, b"\x1b\\").is_some() || after.contains(&0x07)
    } else {
        false
    }
}

/// Spawn a background thread that blocks on `crossterm::event::read()` and
/// forwards events through a crossbeam channel. This lets the main loop
/// `select!` on terminal input alongside PTY and daemon channels.
pub(crate) fn spawn_crossterm_reader() -> crossbeam_channel::Receiver<event::Event> {
    let (tx, rx) = crossbeam_channel::unbounded();
    std::thread::Builder::new()
        .name("crossterm-reader".into())
        .spawn(move || loop {
            match event::poll(Duration::from_millis(100)) {
                Ok(true) => match event::read() {
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
pub(crate) fn compute_next_timeout(
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

/// Spawn a background thread that sends tick events at a fixed interval.
pub(crate) fn spawn_tick_channel(interval: Duration) -> crossbeam_channel::Receiver<()> {
    let (tx, rx) = crossbeam_channel::unbounded();
    std::thread::Builder::new()
        .name("tick-timer".into())
        .spawn(move || loop {
            std::thread::sleep(interval);
            if tx.send(()).is_err() {
                break;
            }
        })
        .expect("spawn tick timer");
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_osc_color_st_terminator() {
        let data = "\x1b]10;rgb:cccc/cccc/cccc\x1b\\";
        assert_eq!(
            parse_osc_color_response(data, "10"),
            Some("rgb:cccc/cccc/cccc".to_string())
        );
    }

    #[test]
    fn parse_osc_color_bel_terminator() {
        let data = "\x1b]11;rgb:1a1a/1a1a/2e2e\x07";
        assert_eq!(
            parse_osc_color_response(data, "11"),
            Some("rgb:1a1a/1a1a/2e2e".to_string())
        );
    }

    #[test]
    fn parse_osc_color_both_in_buffer() {
        let data = "\x1b]10;rgb:ffff/ffff/ffff\x1b\\\x1b]11;rgb:0000/0000/0000\x1b\\";
        assert_eq!(
            parse_osc_color_response(data, "10"),
            Some("rgb:ffff/ffff/ffff".to_string())
        );
        assert_eq!(
            parse_osc_color_response(data, "11"),
            Some("rgb:0000/0000/0000".to_string())
        );
    }

    #[test]
    fn parse_osc_color_not_found() {
        assert_eq!(parse_osc_color_response("", "10"), None);
        assert_eq!(parse_osc_color_response("garbage", "10"), None);
        // Incomplete — no terminator
        assert_eq!(
            parse_osc_color_response("\x1b]10;rgb:cccc/cccc/cccc", "10"),
            None
        );
    }

    #[test]
    fn find_subsequence_found() {
        assert_eq!(find_subsequence(b"\x1b[6n", b"\x1b[6n"), Some(0));
        assert_eq!(find_subsequence(b"abc\x1b[6ndef", b"\x1b[6n"), Some(3));
    }

    #[test]
    fn find_subsequence_not_found() {
        assert_eq!(find_subsequence(b"hello", b"\x1b[6n"), None);
        assert_eq!(find_subsequence(b"", b"\x1b[6n"), None);
    }

    #[test]
    fn contains_osc_query_with_st() {
        let bytes = b"\x1b]10;?\x1b\\";
        assert!(contains_osc_query(bytes, b"\x1b]10;?"));
    }

    #[test]
    fn contains_osc_query_with_bel() {
        let bytes = b"\x1b]11;?\x07";
        assert!(contains_osc_query(bytes, b"\x1b]11;?"));
    }

    #[test]
    fn contains_osc_query_no_terminator() {
        let bytes = b"\x1b]10;?";
        assert!(!contains_osc_query(bytes, b"\x1b]10;?"));
    }

    #[test]
    fn contains_osc_query_not_present() {
        assert!(!contains_osc_query(b"hello", b"\x1b]10;?"));
    }
}
