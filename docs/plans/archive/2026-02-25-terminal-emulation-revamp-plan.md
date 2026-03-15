# Terminal Emulation Revamp: portable-pty Migration — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace raw `nix::pty::forkpty()` with `portable-pty` in both `bash_coprocess.rs` and `interactive.rs` to fix macOS PTY issues (first-command hang, broken auto-complete).

**Architecture:** The bash coprocess switches from raw forkpty + poll-based I/O to portable-pty's `openpty()` + `spawn_command()` with a reader thread feeding an mpsc channel. The interactive session switches to portable-pty for spawning but keeps the poll-based bidirectional I/O loop using `MasterPty::as_raw_fd()`. The sentinel protocol, vt100 parsing, and ratatui rendering are unchanged.

**Tech Stack:** portable-pty 0.9, nix 0.29 (termios/poll), vt100 0.16, ratatui 0.30, crossterm 0.28

---

### Task 1: Add portable-pty dependency

**Files:**
- Modify: `codiv/Cargo.toml`

**Step 1: Add dependency**

In `codiv/Cargo.toml`, add `portable-pty` to `[dependencies]`:

```toml
portable-pty = "0.9"
```

**Step 2: Verify it compiles**

Run: `cd codiv && cargo check`
Expected: Compiles with no errors

**Step 3: Commit**

```bash
git add codiv/Cargo.toml
git commit -m "Add portable-pty dependency for cross-platform PTY support"
```

---

### Task 2: Rewrite BashCoprocess struct and spawn()

**Files:**
- Modify: `codiv/src/shell/bash_coprocess.rs:1-103` (imports, struct, spawn method)

**Step 1: Update imports**

Replace the top of the file. Remove forkpty/Winsize/waitpid/WaitPidFlag/OwnedFd imports. Add portable-pty and mpsc imports:

```rust
//! Bash co-process management via PTY.
//!
//! Spawns a bash shell via portable-pty, executes commands using a sentinel
//! protocol, and provides helpers for capturing cwd and environment variables.

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize, PtySystem, Child};
use rand::Rng;
use std::io::{Read, Write};
use std::os::fd::RawFd;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
```

**Step 2: Rewrite struct definition**

```rust
/// A bash co-process that communicates over a PTY using a sentinel protocol.
pub struct BashCoprocess {
    writer: Box<dyn Write + Send>,
    reader_rx: Receiver<Vec<u8>>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    _reader_handle: JoinHandle<()>,
}
```

**Step 3: Rewrite spawn()**

```rust
impl BashCoprocess {
    /// Spawn a new bash co-process.
    ///
    /// Creates a PTY via portable-pty, spawns bash, and starts a reader
    /// thread. Drains the initial prompt output using an adaptive sentinel.
    pub fn spawn(cols: u16, rows: u16) -> std::io::Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize { rows, cols })
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        let mut cmd = CommandBuilder::new("bash");
        cmd.args(["--noediting", "--norc", "--noprofile", "-i"]);
        cmd.env("PS1", "$ ");
        cmd.env("PROMPT_COMMAND", "");
        cmd.env("HISTFILE", "/dev/null");
        cmd.env("TERM", "xterm-256color");

        let child = pair.slave.spawn_command(cmd)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        // Drop the slave — we communicate through the master only.
        drop(pair.slave);

        let reader = pair.master.try_clone_reader()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        let writer = pair.master.take_writer()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        // Spawn reader thread: reads from PTY and sends chunks through channel.
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("pty-reader".into())
            .spawn(move || {
                Self::reader_thread(reader, tx);
            })
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        let mut coprocess = BashCoprocess {
            writer,
            reader_rx: rx,
            master: pair.master,
            child,
            _reader_handle: handle,
        };

        // Drain the initial prompt output using adaptive sentinel.
        coprocess.drain_initial_output();
        Ok(coprocess)
    }

    /// Reader thread: reads from PTY in a loop and sends chunks through channel.
    fn reader_thread(mut reader: Box<dyn Read + Send>, tx: mpsc::Sender<Vec<u8>>) {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break, // EOF
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break; // Receiver dropped
                    }
                }
                Err(_) => break,
            }
        }
    }
}
```

**Step 4: Verify it compiles (with errors expected for removed methods)**

Run: `cd codiv && cargo check 2>&1 | head -30`
Expected: Compilation errors for methods not yet updated (send_signal, send_interrupt, etc.) — this is expected. The struct and spawn should compile.

---

### Task 3: Rewrite I/O methods (write, read, resize, signal)

**Files:**
- Modify: `codiv/src/shell/bash_coprocess.rs` (methods from send_signal through drain_for)

**Step 1: Rewrite send_signal, send_interrupt, send_bytes**

```rust
    /// Send Ctrl-C to the PTY. This causes the terminal driver to deliver
    /// SIGINT to the entire foreground process group.
    pub fn send_interrupt(&mut self) -> bool {
        self.write_all(b"\x03")
    }

    /// Write raw bytes to the PTY master.
    pub fn send_bytes(&mut self, data: &[u8]) -> bool {
        self.write_all(data)
    }
```

Note: `send_signal()` is removed — portable-pty doesn't expose direct signal sending. `send_interrupt()` (writing `\x03`) is the correct way to interrupt via PTY. Check if `send_signal` is used anywhere in terminal.rs and replace with `send_interrupt`.

**Step 2: Rewrite master_raw_fd()**

```rust
    /// Expose the PTY master fd for direct proxying.
    pub fn master_raw_fd(&self) -> Option<RawFd> {
        self.master.as_raw_fd()
    }
```

Note: `MasterPty::as_raw_fd()` returns `Option<RawFd>`. Update call site in `terminal.rs:247` to handle `Option`.

**Step 3: Rewrite resize methods**

```rust
    /// Update the PTY window size (keeps cols at 500 for sentinel protocol).
    pub fn resize(&self, rows: u16) {
        self.resize_full(rows, 500);
    }

    /// Resize the PTY to arbitrary dimensions.
    pub fn resize_full(&self, rows: u16, cols: u16) {
        let _ = self.master.resize(PtySize { rows, cols });
    }
```

**Step 4: Rewrite write_all()**

```rust
    /// Write all bytes to the PTY master.
    fn write_all(&mut self, data: &[u8]) -> bool {
        self.writer.write_all(data).is_ok()
    }
```

**Step 5: Rewrite try_read()**

```rust
    /// Non-blocking read from the PTY. Returns bytes if data is available.
    pub fn try_read(&self) -> Vec<u8> {
        match self.reader_rx.try_recv() {
            Ok(data) => data,
            Err(TryRecvError::Empty) => Vec::new(),
            Err(TryRecvError::Disconnected) => Vec::new(),
        }
    }
```

**Step 6: Rewrite drain_for()**

```rust
    /// Drain residual PTY output for up to `ms` milliseconds.
    pub fn drain_for(&self, ms: i32) {
        let deadline = Instant::now() + Duration::from_millis(ms as u64);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match self.reader_rx.recv_timeout(remaining.min(Duration::from_millis(50))) {
                Ok(_) => {} // Discard data
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    }
```

---

### Task 4: Rewrite read_until_sentinel and drain_initial_output

**Files:**
- Modify: `codiv/src/shell/bash_coprocess.rs` (read_until_sentinel, drain_initial_output)

**Step 1: Rewrite read_until_sentinel()**

```rust
    /// Read from the PTY until the expanded sentinel is found or timeout expires.
    fn read_until_sentinel(&self, sentinel: &str, timeout_ms: i32) -> String {
        let mut accumulated = String::new();
        let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }

            let wait = remaining.min(Duration::from_millis(100));
            match self.reader_rx.recv_timeout(wait) {
                Ok(data) => {
                    let chunk = String::from_utf8_lossy(&data);
                    accumulated.push_str(&chunk);
                    if Self::find_expanded_sentinel(&accumulated, sentinel).is_some() {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {} // Continue loop
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        accumulated
    }
```

**Step 2: Rewrite drain_initial_output() with adaptive sentinel**

```rust
    /// Drain initial prompt output after spawning.
    ///
    /// Sends a dummy sentinel echo and waits for it to appear. This
    /// guarantees all startup output is consumed regardless of platform
    /// timing (fixes macOS first-command hang).
    fn drain_initial_output(&mut self) {
        let sentinel = Self::generate_sentinel();
        let cmd = format!("echo \"{}0__\"\n", sentinel);
        if self.write_all(cmd.as_bytes()) {
            self.read_until_sentinel(&sentinel, 5000);
        }
    }
```

**Step 3: Verify compilation**

Run: `cd codiv && cargo check`
Expected: May still have errors in execute() and other methods that use `&self` but now need `&mut self` for write. Fix in next task.

---

### Task 5: Fix mutability — execute() and start_command() need &mut self

**Files:**
- Modify: `codiv/src/shell/bash_coprocess.rs` (execute, execute_default, start_command, capture_cwd, capture_env)
- Modify: `codiv/src/ui/terminal.rs` (call sites)
- Modify: `codiv/src/app.rs` (call sites)

**Step 1: Update method signatures in bash_coprocess.rs**

Change all public methods that write to the PTY from `&self` to `&mut self`:

```rust
pub fn execute(&mut self, command: &str, timeout_ms: i32) -> CommandResult { ... }
pub fn execute_default(&mut self, command: &str) -> CommandResult { ... }
pub fn capture_cwd(&mut self) -> String { ... }
pub fn capture_env(&mut self) -> Vec<(String, String)> { ... }
pub fn start_command(&mut self, command: &str) -> Option<String> { ... }
pub fn send_interrupt(&mut self) -> bool { ... }
pub fn send_bytes(&mut self, data: &[u8]) -> bool { ... }
```

Read-only methods stay as `&self`:
```rust
pub fn try_read(&self) -> Vec<u8> { ... }
pub fn resize(&self, rows: u16) { ... }
pub fn resize_full(&self, rows: u16, cols: u16) { ... }
pub fn master_raw_fd(&self) -> Option<RawFd> { ... }
pub fn drain_for(&self, ms: i32) { ... }
```

**Step 2: Update app.rs**

In `app.rs`, change `bash` to `mut bash`:

```rust
let mut bash = BashCoprocess::spawn(500, rows)?;
```

And in `connect_to_daemon`, change parameter to `&mut BashCoprocess`:

```rust
fn connect_to_daemon(bash: &mut BashCoprocess, cwd: &str) -> Option<CodivdClient> {
```

**Step 3: Update terminal.rs**

In `ui::terminal::run()`, the `bash` parameter changes from `&BashCoprocess` to `&mut BashCoprocess`:

```rust
pub fn run(
    bash: &mut BashCoprocess,
    ...
```

All call sites within terminal.rs already use `bash.xxx()` which will work with `&mut self`.

**Step 4: Verify compilation**

Run: `cd codiv && cargo check`
Expected: Should compile. May need to fix a few more borrow issues if `bash` is borrowed immutably somewhere while a mutable borrow is active.

---

### Task 6: Rewrite Drop impl

**Files:**
- Modify: `codiv/src/shell/bash_coprocess.rs` (Drop impl)

**Step 1: Replace the Drop implementation**

```rust
impl Drop for BashCoprocess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
```

This replaces the manual SIGTERM → sleep → SIGKILL → waitpid dance. portable-pty's `Child::kill()` handles platform-specific termination, and `wait()` reaps the process.

**Step 2: Verify compilation**

Run: `cd codiv && cargo check`
Expected: Compiles with no errors

**Step 3: Commit all BashCoprocess changes**

```bash
git add codiv/src/shell/bash_coprocess.rs codiv/src/app.rs codiv/src/ui/terminal.rs
git commit -m "Rewrite BashCoprocess to use portable-pty with reader thread"
```

---

### Task 7: Rewrite InteractiveSession::spawn_and_enter()

**Files:**
- Modify: `codiv/src/shell/interactive.rs:1-112` (imports, spawn_and_enter)

**Step 1: Update imports**

Remove `nix::pty::{forkpty, ForkptyResult, Winsize}` and `nix::sys::wait::{waitpid, WaitPidFlag}`. Add portable-pty imports:

```rust
use portable_pty::{native_pty_system, CommandBuilder, PtySize, PtySystem};
```

Keep these nix imports (still needed for enter/enter_with_sentinel poll loops):
```rust
use nix::errno::Errno;
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use nix::sys::termios::{self, SetArg, Termios};
use nix::unistd;
use std::os::fd::{BorrowedFd, RawFd};
```

**Step 2: Rewrite spawn_and_enter()**

```rust
    pub fn spawn_and_enter(
        &mut self,
        command: &str,
        env: &[(String, String)],
        cwd: &str,
    ) {
        // Get the real terminal size.
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));

        let pty_system = native_pty_system();
        let pair = match pty_system.openpty(PtySize { rows, cols }) {
            Ok(p) => p,
            Err(_) => return,
        };

        let mut cmd = CommandBuilder::new("bash");
        cmd.args(["-c", command]);
        cmd.cwd(cwd);
        // Clear inherited env and set snapshot.
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

        // Enter the poll loop (blocks until HUP).
        self.enter(master_fd);

        // Reap the child.
        let _ = child.wait();
        drop(pair.master);
    }
```

**Step 3: Remove get_terminal_winsize() helper**

The `get_terminal_winsize()` function used `libc::ioctl(TIOCGWINSZ)` — we now use `crossterm::terminal::size()` instead. Remove the `get_terminal_winsize()` function entirely.

**Step 4: Verify compilation**

Run: `cd codiv && cargo check`
Expected: Compiles with no errors

**Step 5: Commit**

```bash
git add codiv/src/shell/interactive.rs
git commit -m "Rewrite InteractiveSession::spawn_and_enter() to use portable-pty"
```

---

### Task 8: Update terminal.rs for master_raw_fd() Option return

**Files:**
- Modify: `codiv/src/ui/terminal.rs:246-247`

**Step 1: Handle Option<RawFd>**

At line ~247, change:
```rust
// Before:
let accumulated = interactive_session.enter_with_sentinel(
    bash.master_raw_fd(),
    &pending.sentinel,
    real_size.height,
    real_size.width,
);
```

To:
```rust
// After:
let master_fd = match bash.master_raw_fd() {
    Some(fd) => fd,
    None => {
        pending_command = Some(pending);
        continue;
    }
};
let accumulated = interactive_session.enter_with_sentinel(
    master_fd,
    &pending.sentinel,
    real_size.height,
    real_size.width,
);
```

**Step 2: Also check for send_signal usage**

Search for `send_signal` in terminal.rs. If found, replace with `send_interrupt()` (which writes `\x03` to PTY — the correct way to interrupt via terminal).

Run: `grep -n send_signal codiv/src/ui/terminal.rs`

If any uses found, replace `bash.send_signal(nix::sys::signal::Signal::SIGINT)` with `bash.send_interrupt()`.

**Step 3: Verify compilation**

Run: `cd codiv && cargo check`
Expected: Compiles with no errors

**Step 4: Commit**

```bash
git add codiv/src/ui/terminal.rs
git commit -m "Handle Option<RawFd> from portable-pty master_raw_fd()"
```

---

### Task 9: Run existing tests

**Files:**
- Test: `codiv/src/shell/bash_coprocess.rs` (existing tests)

**Step 1: Run all tests**

Run: `cd codiv && cargo test 2>&1`
Expected: All existing tests pass. Key tests to watch:
- `test_execute_echo_hello` — basic command execution
- `test_execute_false_exit_code` — exit code capture
- `test_execute_multiline` — multi-command output
- `test_capture_cwd` — cwd capture
- `test_capture_env` — env capture
- `test_large_output_not_truncated` — scrollback capacity
- `test_start_command_and_check_complete` — non-blocking API
- `test_send_bytes_during_command` — keystroke passthrough
- `test_interrupt_hanging_command` — SIGINT delivery

**Step 2: Fix any test failures**

Common issues to watch for:
- Tests use `&self` methods that now need `&mut self` — update test code
- `send_signal` used in tests — replace with `send_interrupt`
- PTY_LOCK mutex may still be needed (portable-pty may have same concurrency issues)

**Step 3: If tests use send_signal, update them**

In tests, `send_signal` is not directly used — `send_interrupt()` is. Verify and fix if needed.

**Step 4: Commit test fixes if any**

```bash
git add codiv/src/shell/bash_coprocess.rs
git commit -m "Fix tests for portable-pty migration"
```

---

### Task 10: Clean up unused imports and build full project

**Files:**
- Modify: `codiv/src/shell/bash_coprocess.rs` (remove unused nix imports)
- Modify: `codiv/src/shell/interactive.rs` (remove unused nix imports)

**Step 1: Remove unused imports from bash_coprocess.rs**

Remove any remaining unused imports from the old forkpty/poll/waitpid code. The compiler warnings will tell you exactly which ones.

Run: `cd codiv && cargo check 2>&1 | grep "unused import"`

**Step 2: Remove unused imports from interactive.rs**

Same — check for unused nix::pty imports.

**Step 3: Remove nix features no longer needed**

In `Cargo.toml`, check if `"process"` feature on nix is still needed. It was for `forkpty` and `waitpid`. If `interactive.rs` no longer uses them, remove `"process"` from features:

```toml
nix = { version = "0.29", features = ["term", "signal", "poll", "fs"] }
```

Note: Keep `"signal"` if any signal-related code remains. Keep `"term"` for termios. Keep `"poll"` for the interactive poll loops.

**Step 4: Full build**

Run: `make clean && make all`
Expected: Both codivd and codiv build successfully

**Step 5: Run full test suite**

Run: `make test`
Expected: All tests pass

**Step 6: Commit**

```bash
git add codiv/Cargo.toml codiv/src/shell/bash_coprocess.rs codiv/src/shell/interactive.rs
git commit -m "Clean up unused imports after portable-pty migration"
```

---

### Task 11: Manual smoke test

**Step 1: Run codiv**

Run: `cd codiv && cargo run`

Test the following:
1. Type `ls -al` — should execute immediately (no hang)
2. Type `echo hello` — should print "hello"
3. Type `pwd` — should show current directory
4. Press Tab for auto-complete — should work
5. Type `less /etc/hosts` — should open pager, exit with `q`, scrollback preserved
6. Type `vim` — should enter vim, exit with `:q`, TUI resumes
7. Type `sudo echo test` — should prompt for password
8. Type `sleep 10` then Ctrl-C — should interrupt
9. Resize the terminal window — should not crash

**Step 2: If any failures, debug and fix**

Common issues:
- If `less` or `vim` doesn't work in interactive mode, check `master_raw_fd()` returns a valid fd
- If resize doesn't work, check `master.resize()` is called correctly
- If first command still hangs, check the adaptive drain sentinel is being consumed

**Step 3: Final commit if any fixes needed**

```bash
git add -u
git commit -m "Fix issues found during smoke testing"
```
