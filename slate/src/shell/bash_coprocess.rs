//! Bash co-process management via PTY.
//!
//! Faithful Rust port of the C++ `BashCoprocess` class. Spawns a bash shell
//! via `forkpty`, executes commands using a sentinel protocol, and provides
//! helpers for capturing cwd and environment variables.

use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use nix::pty::{forkpty, ForkptyResult, Winsize};
use nix::sys::signal::{kill, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag};
use nix::unistd::{write as nix_write, Pid};
use rand::Rng;
use std::ffi::CString;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::time::Instant;

/// Result of executing a command in the bash co-process.
#[derive(Debug, Clone)]
pub struct CommandResult {
    pub output: String,
    pub exit_code: i32,
}

/// A bash co-process that communicates over a PTY using a sentinel protocol.
pub struct BashCoprocess {
    master_fd: OwnedFd,
    child_pid: Pid,
}

#[allow(dead_code)]
impl BashCoprocess {
    /// Spawn a new bash co-process.
    ///
    /// Creates a PTY, forks, and execs `bash --noediting --norc --noprofile -i`
    /// in the child. Drains the initial prompt output in the parent.
    pub fn spawn(cols: u16, rows: u16) -> std::io::Result<Self> {
        let ws = Winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };

        // Safety: we handle the fork child by immediately exec-ing bash.
        let fork_result = unsafe { forkpty(&ws, None) }
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        match fork_result {
            ForkptyResult::Parent { child, master } => {
                let coprocess = BashCoprocess {
                    master_fd: master,
                    child_pid: child,
                };
                // Drain the initial prompt output
                coprocess.drain_initial_output();
                Ok(coprocess)
            }
            ForkptyResult::Child => {
                // In the child process: set up environment and exec bash
                unsafe {
                    libc::setenv(
                        b"PS1\0".as_ptr() as *const libc::c_char,
                        b"$ \0".as_ptr() as *const libc::c_char,
                        1,
                    );
                    libc::unsetenv(b"PROMPT_COMMAND\0".as_ptr() as *const libc::c_char);
                    libc::setenv(
                        b"HISTFILE\0".as_ptr() as *const libc::c_char,
                        b"/dev/null\0".as_ptr() as *const libc::c_char,
                        1,
                    );
                    // Disable pagers — the slate scroll buffer acts as the pager.
                    // Empty string makes git skip the pager entirely (isatty still
                    // returns true, so color.ui=auto produces colors naturally).
                    libc::setenv(
                        b"GIT_PAGER\0".as_ptr() as *const libc::c_char,
                        b"\0".as_ptr() as *const libc::c_char,
                        1,
                    );
                    libc::setenv(
                        b"PAGER\0".as_ptr() as *const libc::c_char,
                        b"\0".as_ptr() as *const libc::c_char,
                        1,
                    );
                    libc::setenv(
                        b"TERM\0".as_ptr() as *const libc::c_char,
                        b"xterm-256color\0".as_ptr() as *const libc::c_char,
                        1,
                    );
                }

                let bash = CString::new("bash").unwrap();
                let args: Vec<CString> = vec![
                    CString::new("bash").unwrap(),
                    CString::new("--noediting").unwrap(),
                    CString::new("--norc").unwrap(),
                    CString::new("--noprofile").unwrap(),
                    CString::new("-i").unwrap(),
                ];
                let _ = nix::unistd::execvp(&bash, &args);
                // If execvp returns, it failed
                unsafe { libc::_exit(127) };
            }
        }
    }

    /// Get the master file descriptor.
    pub fn master_fd(&self) -> &OwnedFd {
        &self.master_fd
    }

    /// Get the child PID.
    pub fn child_pid(&self) -> Pid {
        self.child_pid
    }

    /// Check if the child process is still alive.
    pub fn is_alive(&self) -> bool {
        matches!(
            waitpid(self.child_pid, Some(WaitPidFlag::WNOHANG)),
            Ok(nix::sys::wait::WaitStatus::StillAlive)
        )
    }

    /// Send a signal to the child process.
    pub fn send_signal(&self, sig: Signal) -> nix::Result<()> {
        kill(self.child_pid, sig)
    }

    /// Execute a command in the bash co-process and return its output and exit code.
    pub fn execute(&self, command: &str, timeout_ms: i32) -> CommandResult {
        if self.master_fd.as_raw_fd() < 0 {
            return CommandResult {
                output: String::new(),
                exit_code: -1,
            };
        }

        let sentinel = Self::generate_sentinel();
        let full_cmd = format!(
            "{}; __SLATE_EXIT=$?; echo \"{}${{__SLATE_EXIT}}__\"\n",
            command, sentinel
        );

        if !self.write_all(full_cmd.as_bytes()) {
            return CommandResult {
                output: String::new(),
                exit_code: -1,
            };
        }

        let raw = self.read_until_sentinel(&sentinel, timeout_ms);

        let mut exit_code: i32 = -1;
        if let Some(pos) = Self::find_expanded_sentinel(&raw, &sentinel) {
            let code_start = pos + sentinel.len();
            if let Some(rest) = raw.get(code_start..) {
                if let Some(code_end) = rest.find("__") {
                    let code_str = &rest[..code_end];
                    if let Ok(code) = code_str.parse::<i32>() {
                        exit_code = code;
                    }
                }
            }
        }

        let output = Self::clean_output(&raw, command, &sentinel);
        CommandResult { output, exit_code }
    }

    /// Convenience: execute with default 30s timeout.
    pub fn execute_default(&self, command: &str) -> CommandResult {
        self.execute(command, 30000)
    }

    /// Capture the current working directory of the shell.
    pub fn capture_cwd(&self) -> String {
        let result = self.execute_default("pwd");
        result.output.trim().to_string()
    }

    /// Capture the current environment variables of the shell.
    pub fn capture_env(&self) -> Vec<(String, String)> {
        let result = self.execute_default("env");
        let mut env_vars = Vec::new();
        for line in result.output.lines() {
            if let Some(eq_pos) = line.find('=') {
                let key = &line[..eq_pos];
                let value = &line[eq_pos + 1..];
                // Skip empty keys or lines that don't look like env vars
                if !key.is_empty() && !key.contains(' ') {
                    env_vars.push((key.to_string(), value.to_string()));
                }
            }
        }
        env_vars
    }

    // --- Private helpers ---

    /// Generate a unique sentinel string like `__SLATE_SENTINEL_abcd1234efgh5678_`
    fn generate_sentinel() -> String {
        let mut rng = rand::thread_rng();
        let a: u32 = rng.gen();
        let b: u32 = rng.gen();
        format!("__SLATE_SENTINEL_{:08x}{:08x}_", a, b)
    }

    /// Find the position of the "expanded" sentinel in the output.
    ///
    /// The expanded sentinel is followed by a digit (the exit code), as opposed
    /// to the echoed command line which contains `${__SLATE_EXIT}` literally.
    fn find_expanded_sentinel(text: &str, sentinel: &str) -> Option<usize> {
        let mut search_from = 0;
        loop {
            match text[search_from..].find(sentinel) {
                None => return None,
                Some(relative_pos) => {
                    let pos = search_from + relative_pos;
                    let after = pos + sentinel.len();
                    if after < text.len() {
                        let ch = text.as_bytes()[after];
                        if ch.is_ascii_digit() {
                            return Some(pos);
                        }
                    }
                    search_from = pos + sentinel.len();
                }
            }
        }
    }

    /// Write all bytes to the master fd.
    fn write_all(&self, data: &[u8]) -> bool {
        let mut written = 0;
        while written < data.len() {
            match nix_write(&self.master_fd, &data[written..]) {
                Ok(n) => written += n,
                Err(_) => return false,
            }
        }
        true
    }

    /// Read from the PTY until the expanded sentinel is found or timeout expires.
    fn read_until_sentinel(&self, sentinel: &str, timeout_ms: i32) -> String {
        let mut accumulated = String::new();
        let deadline = Instant::now() + std::time::Duration::from_millis(timeout_ms as u64);

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }

            let poll_timeout =
                PollTimeout::try_from(remaining.as_millis() as i32).unwrap_or(PollTimeout::NONE);

            let borrowed_fd = self.master_fd.as_fd();
            let mut poll_fds = [PollFd::new(borrowed_fd, PollFlags::POLLIN)];

            match poll(&mut poll_fds, poll_timeout) {
                Ok(n) if n > 0 => {
                    if let Some(revents) = poll_fds[0].revents() {
                        if revents.contains(PollFlags::POLLIN) {
                            let mut buf = [0u8; 4096];
                            match nix::unistd::read(self.master_fd.as_raw_fd(), &mut buf) {
                                Ok(n) if n > 0 => {
                                    let chunk = String::from_utf8_lossy(&buf[..n]);
                                    accumulated.push_str(&chunk);
                                    // Check if expanded sentinel is present
                                    if Self::find_expanded_sentinel(&accumulated, sentinel)
                                        .is_some()
                                    {
                                        break;
                                    }
                                }
                                _ => break,
                            }
                        } else if revents.contains(PollFlags::POLLHUP)
                            || revents.contains(PollFlags::POLLERR)
                        {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }

        accumulated
    }

    /// Drain initial prompt output after spawning.
    fn drain_initial_output(&self) {
        for _ in 0..20 {
            let borrowed_fd = self.master_fd.as_fd();
            let mut poll_fds = [PollFd::new(borrowed_fd, PollFlags::POLLIN)];
            match poll(
                &mut poll_fds,
                PollTimeout::try_from(200i32).unwrap_or(PollTimeout::NONE),
            ) {
                Ok(n) if n > 0 => {
                    if let Some(revents) = poll_fds[0].revents() {
                        if revents.contains(PollFlags::POLLIN) {
                            let mut buf = [0u8; 4096];
                            let _ = nix::unistd::read(self.master_fd.as_raw_fd(), &mut buf);
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }
    }

    /// Strip ANSI escape sequences and carriage returns from text.
    fn strip_ansi(text: &str) -> String {
        let mut result = String::with_capacity(text.len());
        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' {
                // CSI sequence: ESC [ ... final_byte
                if chars.peek() == Some(&'[') {
                    chars.next(); // consume '['
                    // Read until a letter (0x40-0x7E)
                    loop {
                        match chars.next() {
                            Some(c) if ('@'..='~').contains(&c) => break,
                            Some(_) => continue,
                            None => break,
                        }
                    }
                } else {
                    // Other ESC sequences: consume next char
                    chars.next();
                }
            } else if ch == '\r' {
                // Strip carriage returns
                continue;
            } else {
                result.push(ch);
            }
        }
        result
    }

    /// Clean the raw output: strip ANSI escapes, sentinel lines, exit code lines,
    /// prompt lines, and echoed command lines.
    fn clean_output(raw: &str, command: &str, sentinel: &str) -> String {
        // Strip carriage returns from PTY output (\r\n → \n)
        let raw = raw.replace('\r', "");
        let lines: Vec<&str> = raw.lines().collect();
        let mut result_lines: Vec<&str> = Vec::new();

        // Build the command string as it would appear echoed
        let cmd_trimmed = command.trim();

        for line in &lines {
            // Strip ANSI only for comparison purposes
            let plain = Self::strip_ansi(line);
            let trimmed = plain.trim();

            // Skip lines containing the sentinel
            if trimmed.contains(sentinel) {
                continue;
            }

            // Skip lines containing __SLATE_EXIT
            if trimmed.contains("__SLATE_EXIT") {
                continue;
            }

            // Skip prompt-only lines (just "$ " or similar)
            if trimmed == "$" || trimmed == "$ " {
                continue;
            }

            // Skip lines that are the echoed command (possibly prefixed with "$ ")
            let without_prompt = trimmed.strip_prefix("$ ").unwrap_or(trimmed);

            // Check if this line matches the echoed command
            if !cmd_trimmed.is_empty() && without_prompt == cmd_trimmed {
                continue;
            }

            // Skip lines that look like the full command with sentinel suffix
            if !cmd_trimmed.is_empty() && without_prompt.starts_with(cmd_trimmed) {
                let rest = &without_prompt[cmd_trimmed.len()..];
                if rest.starts_with("; __SLATE_EXIT") {
                    continue;
                }
            }

            result_lines.push(line);  // Keep original line with ANSI codes
        }

        // Trim leading and trailing empty lines (check plain version)
        while result_lines.first().is_some_and(|l| Self::strip_ansi(l).trim().is_empty()) {
            result_lines.remove(0);
        }
        while result_lines.last().is_some_and(|l| Self::strip_ansi(l).trim().is_empty()) {
            result_lines.pop();
        }

        result_lines.join("\r\n")
    }
}

impl Drop for BashCoprocess {
    fn drop(&mut self) {
        // Send SIGTERM first
        let _ = kill(self.child_pid, Signal::SIGTERM);

        // Give it a moment to exit
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Check if still alive and send SIGKILL if needed
        match waitpid(self.child_pid, Some(WaitPidFlag::WNOHANG)) {
            Ok(nix::sys::wait::WaitStatus::StillAlive) => {
                let _ = kill(self.child_pid, Signal::SIGKILL);
                let _ = waitpid(self.child_pid, None); // Reap zombie
            }
            _ => {
                // Already exited or error — try to reap just in case
                let _ = waitpid(self.child_pid, Some(WaitPidFlag::WNOHANG));
            }
        }
        // OwnedFd will be closed automatically on drop
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spawn_and_is_alive() {
        let coproc = BashCoprocess::spawn(500, 24).expect("Failed to spawn bash coprocess");
        assert!(
            coproc.is_alive(),
            "Bash coprocess should be alive after spawn"
        );
    }

    #[test]
    fn test_execute_echo_hello() {
        let coproc = BashCoprocess::spawn(500, 24).expect("Failed to spawn bash coprocess");
        let result = coproc.execute_default("echo hello");
        assert_eq!(result.exit_code, 0, "exit code should be 0");
        assert!(
            result.output.contains("hello"),
            "output should contain 'hello', got: {:?}",
            result.output
        );
    }

    #[test]
    fn test_execute_false_exit_code() {
        let coproc = BashCoprocess::spawn(500, 24).expect("Failed to spawn bash coprocess");
        let result = coproc.execute_default("false");
        assert_eq!(result.exit_code, 1, "exit code of 'false' should be 1");
    }

    #[test]
    fn test_execute_multiline() {
        let coproc = BashCoprocess::spawn(500, 24).expect("Failed to spawn bash coprocess");
        let result = coproc.execute_default("echo line1; echo line2");
        assert_eq!(result.exit_code, 0);
        assert!(
            result.output.contains("line1"),
            "output should contain 'line1', got: {:?}",
            result.output
        );
        assert!(
            result.output.contains("line2"),
            "output should contain 'line2', got: {:?}",
            result.output
        );
        // Check that line1 comes before line2
        let pos1 = result.output.find("line1").unwrap();
        let pos2 = result.output.find("line2").unwrap();
        assert!(pos1 < pos2, "line1 should come before line2");
    }

    #[test]
    fn test_capture_cwd() {
        let coproc = BashCoprocess::spawn(500, 24).expect("Failed to spawn bash coprocess");
        let cwd = coproc.capture_cwd();
        assert!(!cwd.is_empty(), "cwd should not be empty");
        assert!(
            cwd.starts_with('/'),
            "cwd should be an absolute path, got: {:?}",
            cwd
        );
    }

    #[test]
    fn test_capture_env() {
        let coproc = BashCoprocess::spawn(500, 24).expect("Failed to spawn bash coprocess");
        let env = coproc.capture_env();
        assert!(!env.is_empty(), "env should not be empty");
        // There should be at least some standard env vars
        let keys: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        // PS1 was set explicitly, so it should be present
        assert!(
            keys.contains(&"PS1"),
            "env should contain PS1, got keys: {:?}",
            keys
        );
    }

    #[test]
    fn test_generate_sentinel_format() {
        let sentinel = BashCoprocess::generate_sentinel();
        assert!(sentinel.starts_with("__SLATE_SENTINEL_"));
        assert!(sentinel.ends_with('_'));
        // Format: __SLATE_SENTINEL_ + 16 hex chars + _
        assert_eq!(sentinel.len(), 17 + 16 + 1);
    }

    #[test]
    fn test_find_expanded_sentinel() {
        let sentinel = "__SLATE_SENTINEL_aabbccdd11223344_";
        // Expanded: sentinel followed by digit
        let text = format!("some output\n{}0__\n", sentinel);
        let pos = BashCoprocess::find_expanded_sentinel(&text, sentinel);
        assert!(pos.is_some());

        // Not expanded: sentinel followed by ${
        let text2 = format!("echo \"{}${{__SLATE_EXIT}}__\"", sentinel);
        let pos2 = BashCoprocess::find_expanded_sentinel(&text2, sentinel);
        assert!(pos2.is_none());
    }

    #[test]
    fn test_large_output_not_truncated() {
        // Verify that output larger than the PTY row count is fully captured.
        let coproc = BashCoprocess::spawn(500, 10).expect("Failed to spawn bash coprocess");
        let result = coproc.execute_default("seq 1 100");
        assert_eq!(result.exit_code, 0);
        let lines: Vec<&str> = result.output.lines().collect();
        assert_eq!(
            lines.len(),
            100,
            "should capture all 100 lines, got {} lines. First 5: {:?} ... Last 5: {:?}",
            lines.len(),
            &lines[..lines.len().min(5)],
            &lines[lines.len().saturating_sub(5)..],
        );
        assert_eq!(lines[0], "1");
        assert_eq!(lines[99], "100");
    }

    #[test]
    fn test_strip_ansi() {
        let input = "\x1b[32mhello\x1b[0m world";
        let stripped = BashCoprocess::strip_ansi(input);
        assert_eq!(stripped, "hello world");
    }
}
