//! Shared shell abstraction: trait, session, and IO backends.
//!
//! `ShellIO` defines the low-level read/write contract. `ShellSession<IO>`
//! layers the sentinel protocol on top to provide `execute`, `capture_cwd`,
//! and `capture_env`.

pub mod pipe_io;
pub mod pty_io;

use rand::Rng;
use std::io;
use std::time::Duration;

// ---------------------------------------------------------------------------
// ShellIO trait
// ---------------------------------------------------------------------------

/// Low-level read/write interface to a shell process.
///
/// Implementors wrap either a pipe pair (`PipeIO`) or a PTY master
/// (`PtyIO`). `ShellSession` uses this trait to run the sentinel protocol.
pub trait ShellIO: Send {
    /// Write `data` to the shell's stdin / PTY master.
    fn write_all(&mut self, data: &[u8]) -> io::Result<()>;

    /// Block until the expanded sentinel appears in the output or
    /// `timeout` elapses. Returns all accumulated output (already
    /// cleaned of `\r` and, for PTY, ANSI escapes).
    fn read_until_sentinel(&mut self, sentinel: &str, timeout: Duration) -> String;

    /// Buffered stderr content (if any). Pipe-based backends accumulate
    /// stderr from a background drain thread; PTY backends return an
    /// empty string because stderr is merged into the PTY stream.
    fn drain_stderr(&mut self) -> String;
}

// ---------------------------------------------------------------------------
// CommandOutput
// ---------------------------------------------------------------------------

/// Result of executing a command via `ShellSession`.
#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub stdout: String,
    pub exit_code: i32,
    pub stderr: String,
}

// ---------------------------------------------------------------------------
// Sentinel helpers (free functions)
// ---------------------------------------------------------------------------

/// Generate a unique sentinel string like `__CODIV_SENTINEL_abcd1234efgh5678_`.
pub fn generate_sentinel() -> String {
    let mut rng = rand::thread_rng();
    let a: u32 = rng.gen();
    let b: u32 = rng.gen();
    format!("__CODIV_SENTINEL_{:08x}{:08x}_", a, b)
}

/// Find the position of the "expanded" sentinel in `text`.
///
/// The expanded sentinel is followed by a digit (the exit code), as opposed
/// to the echoed command line which contains `${__CODIV_EXIT}` literally.
pub fn find_expanded_sentinel(text: &str, sentinel: &str) -> Option<usize> {
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

/// Shared `read_until_sentinel` loop used by both `PipeIO` and `PtyIO`.
///
/// Reads chunks from `rx` until the sentinel is found or `timeout` elapses.
/// When `strip_cr` is true, `\r` characters are removed from each chunk
/// (needed for PTY backends where line endings are `\r\n`).
pub fn read_until_sentinel_shared(
    rx: &crossbeam_channel::Receiver<Vec<u8>>,
    sentinel: &str,
    timeout: Duration,
    strip_cr: bool,
) -> String {
    let mut accumulated = String::new();
    let deadline = std::time::Instant::now() + timeout;

    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }

        let wait = remaining.min(Duration::from_millis(1000));
        match rx.recv_timeout(wait) {
            Ok(data) => {
                let chunk = String::from_utf8_lossy(&data);
                if strip_cr {
                    accumulated.push_str(&chunk.replace('\r', ""));
                } else {
                    accumulated.push_str(&chunk);
                }
                if find_expanded_sentinel(&accumulated, sentinel).is_some() {
                    break;
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                // Loop back for deadline check
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    }

    accumulated
}

/// Strip ANSI escape sequences, carriage returns, and BEL from text.
pub fn strip_ansi(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                loop {
                    match chars.next() {
                        Some(c) if ('@'..='~').contains(&c) => break,
                        Some(_) => continue,
                        None => break,
                    }
                }
            } else if chars.peek() == Some(&']') {
                chars.next();
                loop {
                    match chars.next() {
                        Some('\x07') => break,
                        Some('\x1b') => {
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                            }
                            break;
                        }
                        Some(_) => continue,
                        None => break,
                    }
                }
            } else {
                chars.next();
            }
        } else if ch == '\r' || ch == '\x07' {
            continue;
        } else {
            result.push(ch);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// ShellSession<IO>
// ---------------------------------------------------------------------------

/// A shell session that uses the sentinel protocol over any `ShellIO` backend.
pub struct ShellSession<IO: ShellIO> {
    pub io: IO,
}

impl<IO: ShellIO> ShellSession<IO> {
    /// Wrap an IO backend in a session. The caller is responsible for
    /// spawning the shell process and performing any initial drain.
    pub fn new(io: IO) -> Self {
        Self { io }
    }

    /// Drain initial shell output by running a no-op sentinel command.
    pub fn drain_initial(&mut self, timeout: Duration) {
        let sentinel = generate_sentinel();
        let cmd = format!(
            "true; __CODIV_EXIT=$?; echo \"{}${{__CODIV_EXIT}}__\"\n",
            sentinel
        );
        if self.io.write_all(cmd.as_bytes()).is_ok() {
            self.io.read_until_sentinel(&sentinel, timeout);
        }
    }

    /// Execute `command` and return its output, exit code, and stderr.
    pub fn execute(&mut self, command: &str, timeout: Duration) -> CommandOutput {
        let sentinel = generate_sentinel();
        let cmd_trimmed = command.trim_end_matches('\n');
        let full_cmd = format!(
            "{}; __CODIV_EXIT=$?; echo \"{}${{__CODIV_EXIT}}__\"\n",
            cmd_trimmed, sentinel
        );

        if self.io.write_all(full_cmd.as_bytes()).is_err() {
            return CommandOutput {
                stdout: String::new(),
                exit_code: -1,
                stderr: String::new(),
            };
        }

        let raw = self.io.read_until_sentinel(&sentinel, timeout);
        let stderr = self.io.drain_stderr();

        let mut exit_code: i32 = -1;
        if let Some(pos) = find_expanded_sentinel(&raw, &sentinel) {
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

        let stdout = Self::clean_output(&raw, command, &sentinel);
        CommandOutput {
            stdout,
            exit_code,
            stderr,
        }
    }

    /// Convenience: execute with a 30-second timeout.
    pub fn execute_default(&mut self, command: &str) -> CommandOutput {
        self.execute(command, Duration::from_secs(30))
    }

    /// Capture the shell's current working directory.
    pub fn capture_cwd(&mut self) -> String {
        let result = self.execute_default("pwd");
        result.stdout.trim().to_string()
    }

    /// Capture the shell's current environment variables.
    pub fn capture_env(&mut self) -> Vec<(String, String)> {
        let result = self.execute_default("env");
        let mut env_vars = Vec::new();
        for line in result.stdout.lines() {
            if let Some(eq_pos) = line.find('=') {
                let key = &line[..eq_pos];
                let value = &line[eq_pos + 1..];
                if !key.is_empty() && !key.contains(' ') {
                    env_vars.push((key.to_string(), value.to_string()));
                }
            }
        }
        env_vars
    }

    /// Clean the raw output: strip sentinel lines, exit code artifacts,
    /// and echoed command lines.
    fn clean_output(raw: &str, command: &str, sentinel: &str) -> String {
        let raw = raw.replace('\r', "");
        let lines: Vec<&str> = raw.lines().collect();
        let mut result_lines: Vec<&str> = Vec::new();
        let cmd_trimmed = command.trim();

        for line in &lines {
            let plain = strip_ansi(line);
            let trimmed = plain.trim();

            if trimmed.contains(sentinel) {
                continue;
            }
            if trimmed.contains("__CODIV_EXIT") {
                continue;
            }
            if trimmed == "$" || trimmed == "$ " {
                continue;
            }
            let without_prompt = trimmed.strip_prefix("$ ").unwrap_or(trimmed);
            if !cmd_trimmed.is_empty() && without_prompt == cmd_trimmed {
                continue;
            }
            if !cmd_trimmed.is_empty() && without_prompt.starts_with(cmd_trimmed) {
                let rest = &without_prompt[cmd_trimmed.len()..];
                if rest.starts_with("; __CODIV_EXIT") {
                    continue;
                }
            }
            result_lines.push(line);
        }

        while result_lines
            .first()
            .is_some_and(|l| strip_ansi(l).trim().is_empty())
        {
            result_lines.remove(0);
        }
        while result_lines
            .last()
            .is_some_and(|l| strip_ansi(l).trim().is_empty())
        {
            result_lines.pop();
        }

        result_lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_sentinel_format() {
        let sentinel = generate_sentinel();
        assert!(sentinel.starts_with("__CODIV_SENTINEL_"));
        assert!(sentinel.ends_with('_'));
        assert_eq!(sentinel.len(), 17 + 16 + 1);
    }

    #[test]
    fn test_find_expanded_sentinel() {
        let sentinel = "__CODIV_SENTINEL_aabbccdd11223344_";
        let text = format!("some output\n{}0__\n", sentinel);
        assert!(find_expanded_sentinel(&text, sentinel).is_some());

        let text2 = format!("echo \"{}${{__CODIV_EXIT}}__\"", sentinel);
        assert!(find_expanded_sentinel(&text2, sentinel).is_none());
    }

    #[test]
    fn test_strip_ansi_csi() {
        assert_eq!(strip_ansi("\x1b[32mhello\x1b[0m world"), "hello world");
    }

    #[test]
    fn test_strip_ansi_osc_bel() {
        let input = "\x1b]11;rgb:2890/31d7/378c\x07pwd output";
        assert_eq!(strip_ansi(input), "pwd output");
    }

    #[test]
    fn test_strip_ansi_osc_st() {
        let input = "\x1b]7;file:///home/user\x1b\\pwd output";
        assert_eq!(strip_ansi(input), "pwd output");
    }
}
