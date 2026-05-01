//! Bash co-process management via PTY.
//!
//! Spawns a bash shell via `portable-pty`, executes commands using a sentinel
//! protocol, and provides helpers for capturing cwd and environment variables.
//!
//! Note: This intentionally uses bash regardless of the user's `$SHELL` because
//! the completion engine (`completion_engine.rs`) depends on bash-specific
//! features (programmable completions, `compgen`, `COMP_WORDS`, etc.).
//! User commands are still executed through this bash instance, but interactive
//! and daemon-side command execution respect `$SHELL`.

use codiv_common::shell::{self, pty_io::PtyIO, ShellSession};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use crossbeam_channel::Receiver;
use std::time::{Duration, Instant};

/// Result of executing a command in the bash co-process.
#[derive(Debug, Clone)]
pub struct CommandResult {
    pub output: String,
    pub exit_code: i32,
}

/// Git repository metadata: current branch and working-tree diff stats.
#[derive(Debug, Clone, Default)]
pub struct GitInfo {
    pub branch: String,
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
}

/// A bash co-process that communicates over a PTY using a sentinel protocol.
///
/// Delegates blocking command execution to `ShellSession<PtyIO>` while
/// retaining PTY-specific methods for resize, interrupt, and non-blocking IO.
pub struct BashCoprocess {
    session: ShellSession<PtyIO>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    real_rows: u16,
    real_cols: u16,
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
            .map_err(std::io::Error::other)?;

        let mut cmd = CommandBuilder::new("bash");
        cmd.args(["--noediting", "--norc", "--noprofile", "-i"]);
        // Start in the directory where the user launched codiv.
        if let Ok(cwd) = std::env::current_dir() {
            cmd.cwd(cwd);
        }
        cmd.env("PS1", "");
        cmd.env("HISTFILE", "/dev/null");
        cmd.env("TERM", "xterm-256color");
        // Note: portable-pty's CommandBuilder inherits the full parent
        // environment via std::env::vars_os(), so COLORTERM, CLICOLOR,
        // LS_COLORS, LANG, PAGER, etc. are already available.

        // Unset PROMPT_COMMAND to avoid spurious output.
        cmd.env_remove("PROMPT_COMMAND");

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(std::io::Error::other)?;

        // Drop slave — we only need master side.
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(std::io::Error::other)?;
        let writer = pair
            .master
            .take_writer()
            .map_err(std::io::Error::other)?;

        let pty_io = PtyIO::new(writer, reader);
        let mut session = ShellSession::new(pty_io);

        // Drain the initial prompt output adaptively.
        session.drain_initial(Duration::from_secs(5));
        // Suppress PTY echo so the sentinel-wrapped command line is never echoed back.
        // Programs that need echo (vim, python, ssh) call tcsetattr() themselves.
        session.execute("stty -echo", Duration::from_secs(2));
        // Disable history expansion so `!` in commands (e.g. echo "hello!")
        // doesn't trigger "event not found" errors that kill the sentinel protocol.
        session.execute("set +H", Duration::from_secs(2));

        log::info!("bash coprocess ready (pid=child)");
        Ok(BashCoprocess {
            session,
            master: pair.master,
            child,
            real_rows: rows,
            real_cols: cols,
        })
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

    /// Expose the PTY reader channel for use in `select!`-based event loops.
    pub fn pty_receiver(&self) -> &Receiver<Vec<u8>> {
        self.session.io.reader_rx()
    }

    /// Update the PTY window size. Called on terminal resize so that
    /// programs querying ioctl(TIOCGWINSZ) — including `tput lines` in
    /// the PAGER command — get the current dimensions.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.real_rows = rows;
        self.real_cols = cols;
        self.resize_full(rows, cols);
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
    pub fn execute(&mut self, command: &str, timeout_ms: u64) -> CommandResult {
        log::debug!("execute: cmd={:?} timeout={}ms", command, timeout_ms);
        let result = self
            .session
            .execute(command, Duration::from_millis(timeout_ms));
        log::debug!(
            "execute: exit_code={} output_len={}",
            result.exit_code,
            result.stdout.len()
        );
        CommandResult {
            output: result.stdout,
            exit_code: result.exit_code,
        }
    }


    /// Capture the current working directory of the shell.
    pub fn capture_cwd(&mut self) -> String {
        self.session.capture_cwd()
    }

    /// Capture the current environment variables of the shell.
    pub fn capture_env(&mut self) -> Vec<(String, String)> {
        self.session.capture_env()
    }

    /// Capture git branch and working-tree diff stats in a single PTY round-trip.
    ///
    /// Returns `None` when the current directory is not inside a git repository.
    pub fn capture_git_info(&mut self) -> Option<GitInfo> {
        let result = self.execute(
            "git rev-parse --abbrev-ref HEAD 2>/dev/null && git --no-pager diff HEAD --shortstat 2>/dev/null",
            5000,
        );
        if result.exit_code != 0 {
            return None;
        }

        let output = result.output.trim();
        if output.is_empty() {
            return None;
        }

        // First line is the branch name; the rest (if any) is the shortstat output.
        let mut lines = output.lines();
        let branch = lines.next().unwrap_or("").trim().to_string();
        if branch.is_empty() {
            return None;
        }

        let mut files_changed: usize = 0;
        let mut insertions: usize = 0;
        let mut deletions: usize = 0;

        // Remaining lines: look for the shortstat summary.
        for line in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            // Parse tokens like "3 files changed", "10 insertions(+)", "2 deletions(-)"
            let parts: Vec<&str> = line.split(',').collect();
            for part in parts {
                let part = part.trim();
                if part.contains("file") {
                    if let Some(n) = part.split_whitespace().next().and_then(|s| s.parse::<usize>().ok()) {
                        files_changed = n;
                    }
                } else if part.contains("insertion") {
                    if let Some(n) = part.split_whitespace().next().and_then(|s| s.parse::<usize>().ok()) {
                        insertions = n;
                    }
                } else if part.contains("deletion") {
                    if let Some(n) = part.split_whitespace().next().and_then(|s| s.parse::<usize>().ok()) {
                        deletions = n;
                    }
                }
            }
        }

        Some(GitInfo {
            branch,
            files_changed,
            insertions,
            deletions,
        })
    }

    // --- Non-blocking execution API ---

    /// Write command + sentinel to bash without waiting for completion.
    /// Returns the sentinel string needed to detect completion, or `None`
    /// if the write failed.
    pub fn start_command(&mut self, command: &str) -> Option<String> {
        log::debug!("start_command: cmd={:?}", command);
        let sentinel = shell::generate_sentinel();
        let cmd_trimmed = command.trim_end_matches('\n');
        // Single-line format: see execute() for rationale.
        let full_cmd = format!(
            "{}; __CODIV_EXIT=$?; echo \"{}${{__CODIV_EXIT}}__\"\n",
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
        let rx = self.session.io.reader_rx();
        while let Ok(data) = rx.try_recv() {
            result.extend_from_slice(&data);
        }
        result
    }

    /// Drain residual PTY output for up to `ms` milliseconds.
    /// Used after SIGINT to clear bash's `^C` echo and prompt.
    pub fn drain_for(&self, ms: u64) {
        let deadline = Instant::now() + Duration::from_millis(ms);
        let rx = self.session.io.reader_rx();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match rx.recv_timeout(remaining.min(Duration::from_millis(50))) {
                Ok(_) => {
                    // Data received and discarded; keep draining.
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => break,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
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
        let pos = shell::find_expanded_sentinel(accumulated, sentinel)?;

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

    /// Write all bytes to the PTY writer.
    fn write_all(&mut self, data: &[u8]) -> bool {
        use codiv_common::shell::ShellIO;
        self.session.io.write_all(data).is_ok()
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
            let plain = shell::strip_ansi(line);
            let trimmed = plain.trim();

            // Skip lines containing the sentinel
            if trimmed.contains(sentinel) {
                continue;
            }

            // Skip lines containing __CODIV_EXIT
            if trimmed.contains("__CODIV_EXIT") {
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
                if rest.starts_with("; __CODIV_EXIT") {
                    continue;
                }
            }

            result_lines.push(line);  // Keep original line with ANSI codes
        }

        // Trim leading and trailing empty lines (check plain version)
        while result_lines.first().is_some_and(|l| shell::strip_ansi(l).trim().is_empty()) {
            result_lines.remove(0);
        }
        while result_lines.last().is_some_and(|l| shell::strip_ansi(l).trim().is_empty()) {
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
        let mut coproc = BashCoprocess::spawn(80,24).expect("Failed to spawn bash coprocess");
        let result = coproc.execute("echo hello", 30000);
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
        let mut coproc = BashCoprocess::spawn(80,24).expect("Failed to spawn bash coprocess");
        let result = coproc.execute("false", 30000);
        assert_eq!(result.exit_code, 1, "exit code of 'false' should be 1");
    }

    #[test]
    fn test_execute_multiline() {
        let _lock = PTY_LOCK.lock().unwrap();
        let mut coproc = BashCoprocess::spawn(80,24).expect("Failed to spawn bash coprocess");
        let result = coproc.execute("echo line1; echo line2", 30000);
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
        let mut coproc = BashCoprocess::spawn(80,24).expect("Failed to spawn bash coprocess");
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
        let mut coproc = BashCoprocess::spawn(80,24).expect("Failed to spawn bash coprocess");
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
        let sentinel = shell::generate_sentinel();
        assert!(sentinel.starts_with("__CODIV_SENTINEL_"));
        assert!(sentinel.ends_with('_'));
        // Format: __CODIV_SENTINEL_ + 16 hex chars + _
        assert_eq!(sentinel.len(), 17 + 16 + 1);
    }

    #[test]
    fn test_find_expanded_sentinel() {
        let sentinel = "__CODIV_SENTINEL_aabbccdd11223344_";
        // Expanded: sentinel followed by digit
        let text = format!("some output\n{}0__\n", sentinel);
        let pos = shell::find_expanded_sentinel(&text, sentinel);
        assert!(pos.is_some());

        // Not expanded: sentinel followed by ${
        let text2 = format!("echo \"{}${{__CODIV_EXIT}}__\"", sentinel);
        let pos2 = shell::find_expanded_sentinel(&text2, sentinel);
        assert!(pos2.is_none());
    }

    #[test]
    fn test_large_output_not_truncated() {
        let _lock = PTY_LOCK.lock().unwrap();
        // Verify that output larger than the PTY row count is fully captured.
        let mut coproc = BashCoprocess::spawn(80,10).expect("Failed to spawn bash coprocess");
        let result = coproc.execute("seq 1 100", 30000);
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
        let stripped = shell::strip_ansi(input);
        assert_eq!(stripped, "hello world");
    }

    #[test]
    fn test_strip_ansi_osc_bel() {
        // OSC 11 terminated by BEL
        let input = "\x1b]11;rgb:2890/31d7/378c\x07pwd output";
        let stripped = shell::strip_ansi(input);
        assert_eq!(stripped, "pwd output");
    }

    #[test]
    fn test_strip_ansi_osc_st() {
        // OSC 7 terminated by ST (ESC \)
        let input = "\x1b]7;file:///home/user\x1b\\pwd output";
        let stripped = shell::strip_ansi(input);
        assert_eq!(stripped, "pwd output");
    }

    #[test]
    fn test_strip_ansi_mixed() {
        // Mix of CSI, OSC, and plain text
        let input = "\x1b[32m\x1b]0;title\x07hello\x1b[0m";
        let stripped = shell::strip_ansi(input);
        assert_eq!(stripped, "hello");
    }

    #[test]
    fn test_start_command_and_check_complete() {
        let _lock = PTY_LOCK.lock().unwrap();
        let mut coproc = BashCoprocess::spawn(80,24).expect("Failed to spawn bash coprocess");

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
        let coproc = BashCoprocess::spawn(80,24).expect("Failed to spawn bash coprocess");

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
        let mut coproc = BashCoprocess::spawn(80,24).expect("Failed to spawn bash coprocess");

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
        let mut coproc = BashCoprocess::spawn(80,24).expect("Failed to spawn bash coprocess");

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
