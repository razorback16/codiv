//! Interactive command passthrough session.
//!
//! Detects commands that require a full TTY (editors, pagers, REPLs, etc.)
//! and proxies stdin/stdout bidirectionally to a PTY master fd while the
//! terminal is in raw mode.

use std::io::Write;
use std::os::fd::{BorrowedFd, RawFd};
use crossbeam_channel::Receiver;

use super::bash_coprocess::BashCoprocess;

use nix::errno::Errno;
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use nix::sys::termios::{self, SetArg, Termios};
use nix::unistd;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};

/// An interactive passthrough session that proxies raw I/O between the
/// user's terminal and a PTY master file descriptor.
pub struct InteractiveSession {
    saved_termios: Option<Termios>,
    in_session: bool,
}

impl InteractiveSession {
    /// Create a new `InteractiveSession`.
    pub fn new() -> Self {
        Self {
            saved_termios: None,
            in_session: false,
        }
    }

    /// Spawn a dedicated PTY for an interactive command and enter the session.
    ///
    /// Unlike `enter()` which reuses an existing PTY, this forks a new child
    /// process with its own PTY sized to the real terminal dimensions. The
    /// child execs the command via bash, and when it exits the PTY HUPs,
    /// cleanly ending the session.
    pub fn spawn_and_enter(
        &mut self,
        command: &str,
        env: &[(String, String)],
        cwd: &str,
    ) {
        log::debug!("spawn_and_enter: cmd={:?} cwd={}", command, cwd);
        // Get real terminal size via crossterm.
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));

        let pty_system = native_pty_system();
        let pair = match pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        }) {
            Ok(p) => p,
            Err(_) => return,
        };

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "bash".into());
        let mut cmd = CommandBuilder::new(&shell);
        cmd.args(["-c", command]);
        cmd.cwd(cwd);
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.env("TERM", "xterm-256color");

        let mut child = match pair.slave.spawn_command(cmd) {
            Ok(c) => c,
            Err(_) => return,
        };
        drop(pair.slave);

        // Get the raw fd for the poll loop.
        let master_fd = match pair.master.as_raw_fd() {
            Some(fd) => fd,
            None => return,
        };

        log::debug!("spawn_and_enter: entering poll loop ({}x{})", cols, rows);
        // Enter the poll loop (blocks until HUP).
        self.enter(master_fd);

        // Reap the child.
        let _ = child.wait();
        log::debug!("spawn_and_enter: child reaped");
        drop(pair.master);
    }

    /// Enter the interactive session.
    ///
    /// Saves the current terminal attributes, switches stdin to raw mode,
    /// then runs a bidirectional poll loop that proxies data between stdin
    /// and `master_fd`. Returns when the remote side hangs up, an
    /// unrecoverable error occurs, or the session is otherwise terminated.
    ///
    /// Terminal settings are always restored before this method returns.
    pub fn enter(&mut self, master_fd: RawFd) {
        // SAFETY: We use STDIN_FILENO / STDOUT_FILENO which are always valid
        // while the process is alive. The BorrowedFd lifetimes are scoped to
        // this function body which is shorter than the process lifetime.
        let stdin_fd = unsafe { BorrowedFd::borrow_raw(libc::STDIN_FILENO) };
        let stdout_fd = unsafe { BorrowedFd::borrow_raw(libc::STDOUT_FILENO) };
        let master_bfd = unsafe { BorrowedFd::borrow_raw(master_fd) };

        // Save current termios and switch to raw mode.
        match termios::tcgetattr(stdin_fd) {
            Ok(attrs) => {
                self.saved_termios = Some(attrs.clone());
                let mut raw = attrs;
                termios::cfmakeraw(&mut raw);
                let _ = termios::tcsetattr(stdin_fd, SetArg::TCSANOW, &raw);
            }
            Err(_) => {
                // If we cannot get terminal attributes (e.g. stdin is not a
                // TTY), proceed without raw mode.
            }
        }

        self.in_session = true;

        let mut buf = [0u8; 4096];

        loop {
            if !self.in_session {
                break;
            }

            let mut poll_fds = [
                PollFd::new(stdin_fd, PollFlags::POLLIN),
                PollFd::new(master_bfd, PollFlags::POLLIN),
            ];

            let ret = poll(&mut poll_fds, PollTimeout::from(100u16));
            match ret {
                Err(Errno::EINTR) => continue,
                Err(_) => break,
                Ok(_) => {}
            }

            // stdin -> master_fd
            if let Some(revents) = poll_fds[0].revents() {
                if revents.contains(PollFlags::POLLIN) {
                    match unistd::read(libc::STDIN_FILENO, &mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if !write_all(master_fd, &buf[..n]) {
                                break;
                            }
                        }
                    }
                }
            }

            // master_fd -> stdout
            if let Some(revents) = poll_fds[1].revents() {
                if revents.contains(PollFlags::POLLIN) {
                    match unistd::read(master_fd, &mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if !write_all(libc::STDOUT_FILENO, &buf[..n]) {
                                break;
                            }
                        }
                    }
                }

                // master_fd hung up or errored — drain remaining output.
                if revents.intersects(PollFlags::POLLHUP | PollFlags::POLLERR) {
                    loop {
                        match unistd::read(master_fd, &mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                let _ = unistd::write(stdout_fd, &buf[..n]);
                            }
                        }
                    }
                    break;
                }
            }
        }

        self.exit();
    }

    /// Proxy stdin↔PTY until the sentinel appears in output.
    ///
    /// Reads PTY output from the reader thread's mpsc channel (not from
    /// the raw fd) to avoid a race condition with dual readers. Polls
    /// only stdin for input events; PTY data is drained from the channel
    /// on each iteration.
    ///
    /// Returns the accumulated tail (last 4KB) for exit code extraction,
    /// or `None` on error.
    pub fn enter_with_sentinel(
        &mut self,
        reader_rx: &Receiver<Vec<u8>>,
        writer: &mut dyn Write,
        sentinel: &str,
        rows: u16,
        cols: u16,
    ) -> Option<(String, Option<Vec<u8>>)> {
        let stdin_fd = unsafe { BorrowedFd::borrow_raw(libc::STDIN_FILENO) };

        // Save current termios and switch to raw mode.
        match termios::tcgetattr(stdin_fd) {
            Ok(attrs) => {
                self.saved_termios = Some(attrs.clone());
                let mut raw = attrs;
                termios::cfmakeraw(&mut raw);
                let _ = termios::tcsetattr(stdin_fd, SetArg::TCSANOW, &raw);
            }
            Err(_) => {}
        }

        self.in_session = true;

        let mut buf = [0u8; 4096];
        // Keep only the last 4KB of accumulated output to bound memory.
        const TAIL_CAP: usize = 4096;
        let mut tail = String::with_capacity(TAIL_CAP + 512);
        let mut found_sentinel = false;

        // Shadow parser to track alternate screen state and capture content.
        let mut shadow = vt100::Parser::new(rows, cols, 0);
        let mut last_alt_screen: Option<Vec<u8>> = None;

        loop {
            if !self.in_session {
                break;
            }

            // Poll only stdin (short timeout so we also check the channel frequently).
            let mut poll_fds = [
                PollFd::new(stdin_fd, PollFlags::POLLIN),
            ];

            let ret = poll(&mut poll_fds, PollTimeout::from(10u16));
            match ret {
                Err(Errno::EINTR) => {}
                Err(_) => break,
                Ok(_) => {}
            }

            // stdin -> PTY writer
            if let Some(revents) = poll_fds[0].revents() {
                if revents.contains(PollFlags::POLLIN) {
                    match unistd::read(libc::STDIN_FILENO, &mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if writer.write_all(&buf[..n]).is_err() {
                                break;
                            }
                        }
                    }
                }
            }

            // Drain all available PTY data from the reader thread's channel.
            loop {
                match reader_rx.try_recv() {
                    Ok(data) => {
                        // Write to stdout for the user to see.
                        let stdout_bfd = unsafe { BorrowedFd::borrow_raw(libc::STDOUT_FILENO) };
                        let _ = unistd::write(stdout_bfd, &data);

                        // Feed shadow parser and snapshot while in alt screen.
                        shadow.process(&data);
                        if shadow.screen().alternate_screen() {
                            last_alt_screen = Some(snapshot_rows(shadow.screen(), cols));
                        }

                        // Accumulate for sentinel detection.
                        let chunk = String::from_utf8_lossy(&data);
                        tail.push_str(&chunk);
                        // Trim to last TAIL_CAP bytes to bound memory.
                        if tail.len() > TAIL_CAP * 2 {
                            let start = tail.len() - TAIL_CAP;
                            let start = tail.ceil_char_boundary(start);
                            tail = tail[start..].to_string();
                        }
                    }
                    Err(_) => break,
                }
            }

            // Check for sentinel after draining all available data.
            // Strip \n for the check to handle sentinel wrapping on narrow terminals.
            let check_buf = tail.replace('\n', "");
            if BashCoprocess::find_expanded_sentinel(&check_buf, sentinel).is_some() {
                found_sentinel = true;
                break;
            }
        }

        // Restore terminal — but don't write cleanup escape sequences
        // (the TUI caller will handle re-entering alternate screen).
        if let Some(ref saved) = self.saved_termios.take() {
            let stdin_fd = unsafe { BorrowedFd::borrow_raw(libc::STDIN_FILENO) };
            let _ = termios::tcsetattr(stdin_fd, SetArg::TCSANOW, saved);
        }
        self.in_session = false;

        if found_sentinel { Some((tail, last_alt_screen)) } else { None }
    }

    /// Restore the saved terminal attributes and mark the session as inactive.
    pub fn exit(&mut self) {
        // Write terminal cleanup sequences before restoring termios.
        // These handle cases where the interactive program left the
        // terminal in an unexpected state.
        let stdout_fd = unsafe { BorrowedFd::borrow_raw(libc::STDOUT_FILENO) };
        let _ = nix::unistd::write(stdout_fd, b"\x1b[?1049l"); // exit alt screen
        let _ = nix::unistd::write(stdout_fd, b"\x1b[?25h");   // show cursor
        let _ = nix::unistd::write(stdout_fd, b"\x1b[0m");     // reset SGR

        if let Some(ref saved) = self.saved_termios.take() {
            let stdin_fd = unsafe { BorrowedFd::borrow_raw(libc::STDIN_FILENO) };
            let _ = termios::tcsetattr(stdin_fd, SetArg::TCSANOW, saved);
        }
        self.in_session = false;
    }
}

impl Drop for InteractiveSession {
    fn drop(&mut self) {
        self.exit();
    }
}

/// Snapshot visible rows from the shadow screen using per-row formatted output.
///
/// Uses `rows_formatted()` instead of `contents_formatted()` to avoid absolute
/// cursor positioning sequences. Trailing blank rows are trimmed. Rows are
/// joined with `\r\n` so the output flows naturally as scrollback text.
fn snapshot_rows(screen: &vt100::Screen, cols: u16) -> Vec<u8> {
    let rows: Vec<Vec<u8>> = screen.rows_formatted(0, cols).collect();
    let plain: Vec<String> = screen.rows(0, cols).collect();

    // Find the last non-blank row.
    let last_non_blank = plain
        .iter()
        .rposition(|r| !r.trim().is_empty())
        .map(|i| i + 1)
        .unwrap_or(0);

    let mut buf = Vec::new();
    for (i, row) in rows[..last_non_blank].iter().enumerate() {
        buf.extend_from_slice(row);
        if i + 1 < last_non_blank {
            buf.extend_from_slice(b"\r\n");
        }
    }
    buf
}

/// Write the entire buffer to `fd`, retrying on `EINTR`.
/// Returns `true` on success, `false` on any other error.
fn write_all(fd: RawFd, data: &[u8]) -> bool {
    let mut written = 0usize;
    while written < data.len() {
        let bfd = unsafe { BorrowedFd::borrow_raw(fd) };
        match unistd::write(bfd, &data[written..]) {
            Ok(n) => written += n,
            Err(Errno::EINTR) => continue,
            Err(_) => return false,
        }
    }
    true
}
