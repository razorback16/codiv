//! Interactive command passthrough session.
//!
//! Detects commands that require a full TTY (editors, pagers, REPLs, etc.)
//! and proxies stdin/stdout bidirectionally to a PTY master fd while the
//! terminal is in raw mode.

use std::ffi::CString;
use std::os::fd::{AsRawFd, BorrowedFd, RawFd};

use super::bash_coprocess::BashCoprocess;

use nix::errno::Errno;
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use nix::pty::{forkpty, ForkptyResult, Winsize};
use nix::sys::termios::{self, SetArg, Termios};
use nix::sys::wait::{waitpid, WaitPidFlag};
use nix::unistd;

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
        // Get the real terminal size.
        let ws = get_terminal_winsize().unwrap_or(Winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        });

        // Pre-compute all CStrings before fork (no allocation after fork).
        let bash_cstr = CString::new("bash").unwrap();
        let args: Vec<CString> = vec![
            CString::new("bash").unwrap(),
            CString::new("-c").unwrap(),
            CString::new(command).unwrap(),
        ];
        let cwd_cstr = CString::new(cwd).unwrap();
        let env_cstrs: Vec<(CString, CString)> = env
            .iter()
            .filter_map(|(k, v)| {
                Some((CString::new(k.as_str()).ok()?, CString::new(v.as_str()).ok()?))
            })
            .collect();

        // Safety: child immediately execs; only async-signal-safe calls between
        // fork and exec.
        let fork_result = match unsafe { forkpty(&ws, None) } {
            Ok(r) => r,
            Err(_) => return,
        };

        match fork_result {
            ForkptyResult::Parent { child, master } => {
                let master_fd = master.as_raw_fd();

                // Enter the poll loop (this blocks until HUP).
                self.enter(master_fd);

                // Reap the child process.
                let _ = waitpid(child, Some(WaitPidFlag::WNOHANG));

                // master (OwnedFd) is dropped here, closing the PTY.
                drop(master);
            }
            ForkptyResult::Child => {
                // In child: only async-signal-safe operations.
                unsafe {
                    // Clear the environment, then set the snapshot.
                    libc::clearenv();
                    for (k, v) in &env_cstrs {
                        libc::setenv(k.as_ptr(), v.as_ptr(), 1);
                    }
                    // Ensure TERM is set for interactive programs.
                    libc::setenv(
                        b"TERM\0".as_ptr() as *const libc::c_char,
                        b"xterm-256color\0".as_ptr() as *const libc::c_char,
                        1,
                    );
                    // Change to the shell's cwd.
                    libc::chdir(cwd_cstr.as_ptr());
                }

                let _ = nix::unistd::execvp(&bash_cstr, &args);
                unsafe { libc::_exit(127) };
            }
        }
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

    /// Proxy stdin↔master_fd until the sentinel appears in output.
    ///
    /// Same raw-mode poll loop as `enter()`, but instead of breaking on
    /// POLLHUP, we accumulate output and break when the expanded sentinel
    /// is detected. Returns the accumulated tail (last 4KB) for exit code
    /// extraction, or `None` on error.
    pub fn enter_with_sentinel(
        &mut self,
        master_fd: RawFd,
        sentinel: &str,
        rows: u16,
        cols: u16,
    ) -> Option<(String, Option<Vec<u8>>)> {
        let stdin_fd = unsafe { BorrowedFd::borrow_raw(libc::STDIN_FILENO) };
        let master_bfd = unsafe { BorrowedFd::borrow_raw(master_fd) };

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

            // master_fd -> stdout, accumulate for sentinel detection
            if let Some(revents) = poll_fds[1].revents() {
                if revents.contains(PollFlags::POLLIN) {
                    match unistd::read(master_fd, &mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            // Write to stdout for the user to see.
                            let stdout_fd = unsafe { BorrowedFd::borrow_raw(libc::STDOUT_FILENO) };
                            let _ = unistd::write(stdout_fd, &buf[..n]);

                            // Feed shadow parser and snapshot while in alt screen.
                            shadow.process(&buf[..n]);
                            if shadow.screen().alternate_screen() {
                                last_alt_screen = Some(snapshot_rows(shadow.screen(), cols));
                            }

                            // Accumulate for sentinel detection.
                            let chunk = String::from_utf8_lossy(&buf[..n]);
                            tail.push_str(&chunk);
                            // Trim to last TAIL_CAP bytes to bound memory.
                            if tail.len() > TAIL_CAP * 2 {
                                let start = tail.len() - TAIL_CAP;
                                // Find a valid char boundary.
                                let start = tail.ceil_char_boundary(start);
                                tail = tail[start..].to_string();
                            }

                            if BashCoprocess::find_expanded_sentinel(&tail, sentinel).is_some() {
                                found_sentinel = true;
                                break;
                            }
                        }
                    }
                }

                if revents.intersects(PollFlags::POLLHUP | PollFlags::POLLERR) {
                    // Drain remaining output.
                    loop {
                        match unistd::read(master_fd, &mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                let stdout_fd = unsafe { BorrowedFd::borrow_raw(libc::STDOUT_FILENO) };
                                let _ = unistd::write(stdout_fd, &buf[..n]);
                                shadow.process(&buf[..n]);
                                if shadow.screen().alternate_screen() {
                                    last_alt_screen = Some(snapshot_rows(shadow.screen(), cols));
                                }
                                let chunk = String::from_utf8_lossy(&buf[..n]);
                                tail.push_str(&chunk);
                            }
                        }
                    }
                    break;
                }
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

/// Query the real terminal size via ioctl(TIOCGWINSZ).
fn get_terminal_winsize() -> Option<Winsize> {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let ret = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };
    if ret == 0 && ws.ws_row > 0 && ws.ws_col > 0 {
        Some(Winsize {
            ws_row: ws.ws_row,
            ws_col: ws.ws_col,
            ws_xpixel: ws.ws_xpixel,
            ws_ypixel: ws.ws_ypixel,
        })
    } else {
        None
    }
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
