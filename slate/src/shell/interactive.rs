//! Interactive command passthrough session.
//!
//! Detects commands that require a full TTY (editors, pagers, REPLs, etc.)
//! and proxies stdin/stdout bidirectionally to a PTY master fd while the
//! terminal is in raw mode.

use std::collections::HashSet;
use std::os::fd::{BorrowedFd, RawFd};
use std::sync::LazyLock;

use nix::errno::Errno;
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use nix::sys::termios::{self, SetArg, Termios};
use nix::unistd;

/// Commands that require interactive passthrough.
#[allow(dead_code)]
static INTERACTIVE_COMMANDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    HashSet::from([
        "vim", "vi", "nvim", "nano", "emacs",
        "htop", "top",
        "less", "more", "man",
        "ssh", "tmux", "screen",
        "python", "python3", "node", "irb", "ghci",
    ])
});

/// An interactive passthrough session that proxies raw I/O between the
/// user's terminal and a PTY master file descriptor.
pub struct InteractiveSession {
    saved_termios: Option<Termios>,
    in_session: bool,
}

#[allow(dead_code)]
impl InteractiveSession {
    /// Create a new `InteractiveSession`.
    pub fn new() -> Self {
        Self {
            saved_termios: None,
            in_session: false,
        }
    }

    /// Returns `true` if `command` begins with a program name that requires
    /// interactive passthrough (e.g. vim, python3, ssh).
    ///
    /// The first whitespace-delimited token is extracted and any leading path
    /// components are stripped before checking against the known set.
    pub fn needs_passthrough(command: &str) -> bool {
        let first_word = match command.split_whitespace().next() {
            Some(w) => w,
            None => return false,
        };
        // Strip leading path: "/usr/bin/vim" -> "vim"
        let basename = match first_word.rfind('/') {
            Some(pos) => &first_word[pos + 1..],
            None => first_word,
        };
        if basename.is_empty() {
            return false;
        }
        INTERACTIVE_COMMANDS.contains(basename)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_needs_passthrough_vim() {
        assert!(InteractiveSession::needs_passthrough("vim file.txt"));
    }

    #[test]
    fn test_needs_passthrough_with_path() {
        assert!(InteractiveSession::needs_passthrough("/usr/bin/vim file.txt"));
    }

    #[test]
    fn test_needs_passthrough_ls() {
        assert!(!InteractiveSession::needs_passthrough("ls -la"));
    }

    #[test]
    fn test_needs_passthrough_empty() {
        assert!(!InteractiveSession::needs_passthrough(""));
    }

    #[test]
    fn test_needs_passthrough_all_interactive() {
        let commands = [
            "vim", "vi", "nvim", "nano", "emacs",
            "htop", "top",
            "less", "more", "man",
            "ssh", "tmux", "screen",
            "python", "python3", "node", "irb", "ghci",
        ];
        for cmd in &commands {
            assert!(
                InteractiveSession::needs_passthrough(cmd),
                "{cmd} should need passthrough"
            );
        }
    }
}
