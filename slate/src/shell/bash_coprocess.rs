//! Bash co-process management via PTY.
//!
//! Spawns a bash shell via `portable-pty`, executes commands using a sentinel
//! protocol, and provides helpers for capturing cwd and environment variables.

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use rand::Rng;
use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Result of executing a command in the bash co-process.
#[derive(Debug, Clone)]
pub struct CommandResult {
    pub output: String,
    pub exit_code: i32,
}

/// A bash co-process that communicates over a PTY using a sentinel protocol.
pub struct BashCoprocess {
    writer: Box<dyn Write + Send>,
    reader_rx: Receiver<Vec<u8>>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    reader_handle: Option<JoinHandle<()>>,
}

fn reader_thread(mut reader: Box<dyn Read + Send>, tx: mpsc::Sender<Vec<u8>>) {
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

impl BashCoprocess {
    /// Spawn a new bash co-process.
    ///
    /// Creates a PTY via portable-pty, spawns `bash --noediting --norc --noprofile -i`,
    /// and starts a background reader thread. Drains initial prompt output adaptively.
    pub fn spawn(cols: u16, rows: u16) -> std::io::Result<Self> {
        log::debug!("BashCoprocess::spawn(cols={}, rows={})", cols, rows);
        let pty_system = native_pty_system();

        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        let mut cmd = CommandBuilder::new("bash");
        cmd.args(["--noediting", "--norc", "--noprofile", "-i"]);
        cmd.env("PS1", "$ ");
        cmd.env("HISTFILE", "/dev/null");
        cmd.env("TERM", "xterm-256color");
        // Unset PROMPT_COMMAND to avoid spurious output.
        cmd.env_remove("PROMPT_COMMAND");

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        // Drop slave — we only need master side.
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        let (tx, rx) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("pty-reader".into())
            .spawn(move || reader_thread(reader, tx))
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        let mut coprocess = BashCoprocess {
            writer,
            reader_rx: rx,
            master: pair.master,
            child,
            reader_handle: Some(handle),
        };

        // Drain the initial prompt output adaptively.
        coprocess.drain_initial_output();
        log::info!("bash coprocess ready (pid=child)");
        Ok(coprocess)
    }

    /// Send Ctrl-C to the PTY. This causes the terminal driver to deliver
    /// SIGINT to the entire foreground process group, interrupting both
    /// bash and any child process (e.g. `sleep`, `cat`).
    pub fn send_interrupt(&mut self) -> bool {
        self.write_all(b"\x03")
    }

    /// Write raw bytes to the PTY master fd.
    ///
    /// Used to forward user keystrokes to the bash coprocess while a
    /// command is executing (e.g. password prompts for `sudo`, Y/n
    /// prompts for `apt install`).
    pub fn send_bytes(&mut self, data: &[u8]) -> bool {
        self.write_all(data)
    }

    /// Expose the reader channel and writer for use by `enter_with_sentinel()`.
    /// Returns both references at once to satisfy the borrow checker (avoids
    /// overlapping immutable + mutable borrows on `self`).
    pub fn reader_and_writer(&mut self) -> (&Receiver<Vec<u8>>, &mut dyn Write) {
        (&self.reader_rx, &mut *self.writer)
    }

    /// Update the PTY window size. Called on terminal resize so that
    /// programs querying ioctl(TIOCGWINSZ) — including `tput lines` in
    /// the PAGER command — get the current dimensions.
    pub fn resize(&self, rows: u16) {
        // Keep cols at 500 (wide PTY prevents sentinel wrapping).
        self.resize_full(rows, 500);
    }

    /// Resize the PTY to arbitrary dimensions. Used to match the real
    /// terminal size when proxying interactive programs, and to restore
    /// the wide sentinel-protocol dimensions afterwards.
    pub fn resize_full(&self, rows: u16, cols: u16) {
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
    }

    /// Execute a command in the bash co-process and return its output and exit code.
    pub fn execute(&mut self, command: &str, timeout_ms: i32) -> CommandResult {
        log::debug!("execute: cmd={:?} timeout={}ms", command, timeout_ms);
        let sentinel = Self::generate_sentinel();
        let cmd_trimmed = command.trim_end_matches('\n');
        // Single-line format: command and sentinel on one line so bash parses
        // the entire compound command before executing. This prevents commands
        // that read from stdin (like `read`) from consuming the sentinel line.
        let full_cmd = format!(
            "{}; __SLATE_EXIT=$?; echo \"{}${{__SLATE_EXIT}}__\"\n",
            cmd_trimmed, sentinel
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
        log::debug!("execute: exit_code={} output_len={}", exit_code, output.len());
        CommandResult { output, exit_code }
    }

    /// Convenience: execute with default 30s timeout.
    pub fn execute_default(&mut self, command: &str) -> CommandResult {
        self.execute(command, 30000)
    }

    /// Capture the current working directory of the shell.
    pub fn capture_cwd(&mut self) -> String {
        let result = self.execute_default("pwd");
        result.output.trim().to_string()
    }

    /// Capture the current environment variables of the shell.
    pub fn capture_env(&mut self) -> Vec<(String, String)> {
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

    // --- Non-blocking execution API ---

    /// Write command + sentinel to bash without waiting for completion.
    /// Returns the sentinel string needed to detect completion, or `None`
    /// if the write failed.
    pub fn start_command(&mut self, command: &str) -> Option<String> {
        log::debug!("start_command: cmd={:?}", command);
        let sentinel = Self::generate_sentinel();
        let cmd_trimmed = command.trim_end_matches('\n');
        // Single-line format: see execute() for rationale.
        let full_cmd = format!(
            "{}; __SLATE_EXIT=$?; echo \"{}${{__SLATE_EXIT}}__\"\n",
            cmd_trimmed, sentinel
        );
        if self.write_all(full_cmd.as_bytes()) {
            Some(sentinel)
        } else {
            None
        }
    }

    /// Non-blocking read from the PTY. Returns bytes if data is available,
    /// or an empty vec if there is nothing to read right now.
    pub fn try_read(&self) -> Vec<u8> {
        let mut result = Vec::new();
        loop {
            match self.reader_rx.try_recv() {
                Ok(data) => result.extend_from_slice(&data),
                Err(_) => break,
            }
        }
        result
    }

    /// Drain residual PTY output for up to `ms` milliseconds.
    /// Used after SIGINT to clear bash's `^C` echo and prompt.
    pub fn drain_for(&self, ms: i32) {
        let deadline = Instant::now() + Duration::from_millis(ms as u64);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match self.reader_rx.recv_timeout(remaining.min(Duration::from_millis(50))) {
                Ok(_) => {
                    // Data received and discarded; keep draining.
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break, // silence — done draining
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    /// Check if the sentinel has appeared in accumulated output.
    /// Returns `Some(CommandResult)` with cleaned output if complete,
    /// `None` if still waiting.
    pub fn check_complete(
        accumulated: &str,
        command: &str,
        sentinel: &str,
    ) -> Option<CommandResult> {
        let pos = Self::find_expanded_sentinel(accumulated, sentinel)?;

        let mut exit_code: i32 = -1;
        let code_start = pos + sentinel.len();
        if let Some(rest) = accumulated.get(code_start..) {
            if let Some(code_end) = rest.find("__") {
                let code_str = &rest[..code_end];
                if let Ok(code) = code_str.parse::<i32>() {
                    exit_code = code;
                }
            }
        }

        let output = Self::clean_output(accumulated, command, sentinel);
        Some(CommandResult { output, exit_code })
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

    /// Write all bytes to the PTY writer.
    fn write_all(&mut self, data: &[u8]) -> bool {
        self.writer.write_all(data).is_ok()
    }

    /// Read from the PTY until the expanded sentinel is found or timeout expires.
    fn read_until_sentinel(&self, sentinel: &str, timeout_ms: i32) -> String {
        let mut accumulated = String::new();
        let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }

            let wait = remaining.min(Duration::from_millis(1000));
            match self.reader_rx.recv_timeout(wait) {
                Ok(data) => {
                    let chunk = String::from_utf8_lossy(&data);
                    accumulated.push_str(&chunk);
                    // Check if expanded sentinel is present
                    if Self::find_expanded_sentinel(&accumulated, sentinel).is_some() {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // Loop back so deadline check can fire
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        accumulated
    }

    /// Drain initial prompt output after spawning.
    ///
    /// Uses the full sentinel protocol (with `__SLATE_EXIT`) so the expanded
    /// sentinel is distinguishable from the echoed command line. This ensures
    /// the initial prompt and any shell startup messages are consumed before
    /// the first `execute` call sees the PTY output.
    fn drain_initial_output(&mut self) {
        log::debug!("draining initial PTY output");
        let sentinel = Self::generate_sentinel();
        let cmd = format!(
            "true; __SLATE_EXIT=$?; echo \"{}${{__SLATE_EXIT}}__\"\n",
            sentinel
        );
        if self.write_all(cmd.as_bytes()) {
            self.read_until_sentinel(&sentinel, 5000);
        }
    }

    /// Strip ANSI escape sequences and carriage returns from text.
    pub fn strip_ansi(text: &str) -> String {
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
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Child is dead → slave fd closed → reader's read() returns error → thread exits.
        if let Some(handle) = self.reader_handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// PTY tests must run sequentially — concurrent PTY calls under the
    /// parallel test harness cause resource contention that makes bash slow to
    /// start, breaking the sentinel-based protocol.
    static PTY_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_execute_echo_hello() {
        let _lock = PTY_LOCK.lock().unwrap();
        let mut coproc = BashCoprocess::spawn(500, 24).expect("Failed to spawn bash coprocess");
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
        let _lock = PTY_LOCK.lock().unwrap();
        let mut coproc = BashCoprocess::spawn(500, 24).expect("Failed to spawn bash coprocess");
        let result = coproc.execute_default("false");
        assert_eq!(result.exit_code, 1, "exit code of 'false' should be 1");
    }

    #[test]
    fn test_execute_multiline() {
        let _lock = PTY_LOCK.lock().unwrap();
        let mut coproc = BashCoprocess::spawn(500, 24).expect("Failed to spawn bash coprocess");
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
        let _lock = PTY_LOCK.lock().unwrap();
        let mut coproc = BashCoprocess::spawn(500, 24).expect("Failed to spawn bash coprocess");
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
        let _lock = PTY_LOCK.lock().unwrap();
        let mut coproc = BashCoprocess::spawn(500, 24).expect("Failed to spawn bash coprocess");
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
        let _lock = PTY_LOCK.lock().unwrap();
        // Verify that output larger than the PTY row count is fully captured.
        let mut coproc = BashCoprocess::spawn(500, 10).expect("Failed to spawn bash coprocess");
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

    #[test]
    fn test_start_command_and_check_complete() {
        let _lock = PTY_LOCK.lock().unwrap();
        let mut coproc = BashCoprocess::spawn(500, 24).expect("Failed to spawn bash coprocess");

        let sentinel = coproc.start_command("echo hello").expect("start_command failed");
        let mut accumulated = String::new();

        // Poll until sentinel appears (up to 5s).
        let deadline = Instant::now() + Duration::from_secs(5);
        let result = loop {
            if Instant::now() > deadline {
                panic!("timed out waiting for sentinel, accumulated: {:?}", accumulated);
            }
            let bytes = coproc.try_read();
            if !bytes.is_empty() {
                accumulated.push_str(&String::from_utf8_lossy(&bytes));
            }
            if let Some(r) = BashCoprocess::check_complete(&accumulated, "echo hello", &sentinel) {
                break r;
            }
            std::thread::sleep(Duration::from_millis(10));
        };

        assert_eq!(result.exit_code, 0, "exit code should be 0");
        assert!(
            result.output.contains("hello"),
            "output should contain 'hello', got: {:?}",
            result.output
        );
    }

    #[test]
    fn test_try_read_no_data() {
        let _lock = PTY_LOCK.lock().unwrap();
        let coproc = BashCoprocess::spawn(500, 24).expect("Failed to spawn bash coprocess");

        // Drain initial output first.
        std::thread::sleep(Duration::from_millis(200));
        while !coproc.try_read().is_empty() {}

        // Now there should be no data.
        let bytes = coproc.try_read();
        assert!(
            bytes.is_empty(),
            "try_read should return empty when no data, got {} bytes",
            bytes.len()
        );
    }

    #[test]
    fn test_send_bytes_during_command() {
        let _lock = PTY_LOCK.lock().unwrap();
        let mut coproc = BashCoprocess::spawn(500, 24).expect("Failed to spawn bash coprocess");

        // Use send_bytes to write a complete command to the PTY.
        // This verifies the public API writes through to the PTY
        // master fd (the same mechanism used for sudo passwords,
        // apt Y/n prompts, etc.).
        assert!(
            coproc.send_bytes(b"echo via_send_bytes\n"),
            "send_bytes should succeed"
        );

        // Give bash time to process and produce output.
        std::thread::sleep(Duration::from_millis(500));

        // Read back the output — it should contain the echoed text.
        let mut output = String::new();
        loop {
            let bytes = coproc.try_read();
            if bytes.is_empty() {
                break;
            }
            output.push_str(&String::from_utf8_lossy(&bytes));
        }

        assert!(
            output.contains("via_send_bytes"),
            "PTY output should contain text sent via send_bytes, got: {:?}",
            output
        );

        // Verify the shell is still usable via the sentinel protocol.
        let result = coproc.execute("echo recovered", 10_000);
        assert_eq!(result.exit_code, 0);
        assert!(
            result.output.contains("recovered"),
            "output should contain 'recovered', got: {:?}",
            result.output
        );
    }

    #[test]
    fn test_interrupt_hanging_command() {
        let _lock = PTY_LOCK.lock().unwrap();
        let mut coproc = BashCoprocess::spawn(500, 24).expect("Failed to spawn bash coprocess");

        // Start `sleep 60` which blocks but is cleanly interruptible.
        let sentinel = coproc.start_command("sleep 60").expect("start_command failed");

        // Give it a moment to start.
        std::thread::sleep(Duration::from_millis(200));

        // Send Ctrl-C via PTY (delivers SIGINT to entire foreground
        // process group, including the sleep child process).
        assert!(coproc.send_interrupt(), "send_interrupt should succeed");

        // Poll until sentinel appears (SIGINT causes sleep to exit,
        // then the sentinel echo runs with exit code 130).
        let mut accumulated = String::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        let result = loop {
            if Instant::now() > deadline {
                break None;
            }
            let bytes = coproc.try_read();
            if !bytes.is_empty() {
                accumulated.push_str(&String::from_utf8_lossy(&bytes));
            }
            if let Some(r) = BashCoprocess::check_complete(&accumulated, "sleep 60", &sentinel) {
                break Some(r);
            }
            std::thread::sleep(Duration::from_millis(10));
        };

        // Whether sentinel was found or not, drain residual output.
        coproc.drain_for(500);

        if let Some(r) = result {
            // SIGINT on sleep gives exit code 130.
            assert!(
                r.exit_code == 130 || r.exit_code == 0,
                "exit code after SIGINT should be 130 (or 0), got: {}",
                r.exit_code
            );
        }

        // Verify the shell is still usable by running another command.
        let result = coproc.execute("echo recovered", 10_000);
        assert_eq!(result.exit_code, 0, "exit code should be 0 after recovery, output: {:?}", result.output);
        assert!(
            result.output.contains("recovered"),
            "output should contain 'recovered', got: {:?}",
            result.output
        );
    }
}
