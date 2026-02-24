//! Interactive command passthrough session.
//!
//! Detects commands that require a full TTY (editors, pagers, REPLs, etc.)
//! and proxies stdin/stdout bidirectionally to a PTY master fd while the
//! terminal is in raw mode.

use std::os::fd::{BorrowedFd, RawFd};

use nix::errno::Errno;
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use nix::sys::termios::{self, SetArg, Termios};
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
