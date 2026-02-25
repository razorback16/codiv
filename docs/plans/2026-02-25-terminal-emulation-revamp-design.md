# Terminal Emulation Revamp: portable-pty Migration

**Date:** 2026-02-25
**Status:** Approved

## Problem

The current terminal emulation uses raw `nix::pty::forkpty()` with platform-specific behavior that causes issues on macOS:

1. **First command hangs**: The initial prompt drain uses a hardcoded 200ms silence timeout. On macOS, bash startup can be slower, causing the drain to miss output. The first real command then sees leftover prompt bytes mixed into its output, confusing sentinel detection.
2. **Auto-complete broken**: Tab completion depends on the same bash coprocess. If the coprocess is in a bad state from the drain issue, completions fail.
3. **Hardcoded parameters**: `POLLPRI` handling differences between Linux and macOS, timing-based drain, raw ioctl calls.

## Solution

Replace the raw PTY layer with `portable-pty` (from wezterm, v0.9), which abstracts platform-specific PTY differences across Linux and macOS. Keep the existing vt100 parsing and ratatui rendering stack.

## Architecture

### BashCoprocess (bash_coprocess.rs)

**Before:**
```
forkpty() → OwnedFd (master) + Pid (child)
  ├── nix::unistd::read/write on master fd
  ├── nix::poll::poll() for non-blocking I/O
  ├── libc::ioctl(TIOCSWINSZ) for resize
  └── kill(SIGTERM/SIGKILL) + waitpid() for cleanup
```

**After:**
```
native_pty_system().openpty() → MasterPty + SlavePty
slave.spawn_command() → Child
  ├── Reader thread: master.try_clone_reader() → mpsc::channel
  ├── Writer: master.take_writer() → Box<dyn Write>
  ├── master.resize(PtySize{...}) for resize
  └── child.kill() / child.wait() for cleanup
```

**Struct:**
```rust
pub struct BashCoprocess {
    writer: Box<dyn Write + Send>,
    reader_rx: Receiver<Vec<u8>>,
    child: Box<dyn Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    _reader_handle: JoinHandle<()>,  // Reader thread
}
```

**Reader Thread:**
- Spawns on construction
- Reads from PTY in a loop (blocking `Read::read()`)
- Sends chunks through `mpsc::Sender<Vec<u8>>`
- Exits when PTY closes (read returns 0 or error)

**Non-blocking API:**
- `try_read()` → `reader_rx.try_recv()`
- `read_until_sentinel()` → loop with `reader_rx.recv_timeout()`
- `drain_for()` → loop with `reader_rx.recv_timeout()`

**Adaptive Initial Drain:**
Instead of 200ms silence timeout, send a sentinel echo and wait for it:
```rust
fn drain_initial_output(&mut self) {
    let sentinel = Self::generate_sentinel();
    let cmd = format!("echo \"{}0__\"\n", sentinel);
    self.write_all(cmd.as_bytes());
    self.read_until_sentinel(&sentinel, 5000);  // 5s timeout
}
```
This guarantees all startup output is consumed regardless of platform timing.

**Resize:**
```rust
pub fn resize(&self, rows: u16) {
    let _ = self.master.resize(PtySize {
        rows, cols: 500, pixel_width: 0, pixel_height: 0,
    });
}
```

**Cleanup (Drop):**
```rust
impl Drop for BashCoprocess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
```

### InteractiveSession (interactive.rs)

**`spawn_and_enter()`:** Replace `forkpty()` with portable-pty's `openpty()` + `spawn_command()`. Use `CommandBuilder` to set env, cwd, and args cleanly without unsafe fork/exec code.

**`enter()` / `enter_with_sentinel()`:** Keep the poll-based bidirectional I/O loop. Get the raw fd from portable-pty's master (available on both Linux and macOS via the underlying POSIX PTY). The poll loop is proven and efficient.

**Raw fd access:** portable-pty's `MasterPty` on Unix wraps a file descriptor. We access it via `AsRawFd` (available on the concrete Unix type) for the poll loop.

### What Stays the Same

- **Sentinel protocol**: generate, detect, clean — all unchanged
- **vt100 parsing**: `vt100::Parser` for terminal state, alternate screen detection
- **tui-term widget**: `PseudoTerminal` rendering into ratatui
- **ratatui + crossterm**: TUI framework and terminal manipulation
- **terminal.rs event loop**: Main loop, rendering, command dispatch
- **All tests**: They test behavior (execute, sentinel, strip_ansi), not PTY implementation

## Dependency Changes

**Add:**
- `portable-pty = "0.9"`

**Keep (still needed):**
- `nix = "0.29"` — termios (raw mode in interactive.rs), poll (interactive I/O loops)
- `libc = "0.2"` — STDIN_FILENO/STDOUT_FILENO constants
- `vt100 = "0.16"`, `tui-term = "0.3"`, `ratatui = "0.30"`, `crossterm = "0.28"`

**Remove from usage:**
- `nix::pty::forkpty`, `ForkptyResult`, `Winsize`
- `nix::sys::wait::waitpid`, `WaitPidFlag`
- `libc::ioctl(TIOCSWINSZ)` / `libc::ioctl(TIOCGWINSZ)` for PTY resize/query

## Files Changed

| File | Change | Lines |
|------|--------|-------|
| `slate/Cargo.toml` | Add `portable-pty = "0.9"` | ~1 |
| `slate/src/shell/bash_coprocess.rs` | Rewrite spawn, read, write, resize, drain, drop | ~250 |
| `slate/src/shell/interactive.rs` | Rewrite `spawn_and_enter()`, adapt fd access | ~80 |
| `slate/src/ui/terminal.rs` | Adapt `master_raw_fd()` usage if API changes | ~5 |

## Risk Mitigation

1. **portable-pty blocking readers**: Mitigated by reader thread + mpsc channel pattern
2. **Raw fd access for interactive poll loop**: portable-pty uses native POSIX PTYs on Unix; fd is accessible
3. **Sentinel protocol unchanged**: No risk to command completion detection
4. **Existing tests**: All tests verify sentinel protocol behavior, not PTY implementation details
