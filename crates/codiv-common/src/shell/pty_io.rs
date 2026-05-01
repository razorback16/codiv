//! `PtyIO`: `ShellIO` implementation over a portable-pty master.
//!
//! PTY merges stdout and stderr into a single stream, so `drain_stderr`
//! always returns an empty string. Output is stripped of `\r` characters
//! and ANSI escape sequences are left in the raw stream (they are stripped
//! during `clean_output` in `ShellSession`).

use super::{read_until_sentinel_shared, ShellIO};
use crossbeam_channel::{self, Receiver, Sender};
use std::io::{self, Read, Write};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// `ShellIO` backend that communicates via a portable-pty reader/writer pair.
///
/// A background thread reads from the PTY master and sends chunks through
/// a channel, mirroring the pattern in `BashCoprocess`.
pub struct PtyIO {
    writer: Box<dyn Write + Send>,
    reader_rx: Receiver<Vec<u8>>,
    _reader_handle: Option<JoinHandle<()>>,
}

impl PtyIO {
    /// Create a new `PtyIO` from the given writer and reader.
    ///
    /// Starts a background thread that reads from the PTY reader and
    /// sends chunks through a crossbeam channel.
    pub fn new(writer: Box<dyn Write + Send>, reader: Box<dyn Read + Send>) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();

        let handle = thread::Builder::new()
            .name("pty-io-reader".into())
            .spawn(move || Self::reader_thread(reader, tx))
            .expect("failed to spawn PTY reader thread");

        Self {
            writer,
            reader_rx: rx,
            _reader_handle: Some(handle),
        }
    }

    /// Expose the reader channel for external use (e.g., `select!`-based
    /// event loops in `BashCoprocess`).
    pub fn reader_rx(&self) -> &Receiver<Vec<u8>> {
        &self.reader_rx
    }

    fn reader_thread(mut reader: Box<dyn Read + Send>, tx: Sender<Vec<u8>>) {
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
}

impl ShellIO for PtyIO {
    fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        self.writer.write_all(data)?;
        self.writer.flush()
    }

    fn read_until_sentinel(&mut self, sentinel: &str, timeout: Duration) -> String {
        // strip_cr=true: PTY line endings are \r\n; stripping \r keeps
        // the sentinel from being split by \r at column boundaries.
        read_until_sentinel_shared(&self.reader_rx, sentinel, timeout, true)
    }

    fn drain_stderr(&mut self) -> String {
        // PTY merges stderr into stdout — no separate stderr.
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::ShellSession;
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};
    use std::sync::Mutex;
    use std::time::Duration;

    /// PTY tests must run sequentially to avoid resource contention.
    static PTY_LOCK: Mutex<()> = Mutex::new(());

    fn spawn_pty_session() -> (
        ShellSession<PtyIO>,
        Box<dyn portable_pty::Child + Send + Sync>,
        Box<dyn portable_pty::MasterPty + Send>,
    ) {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("failed to open PTY");

        let mut cmd = CommandBuilder::new("bash");
        cmd.args(["--noediting", "--norc", "--noprofile", "-i"]);
        if let Ok(cwd) = std::env::current_dir() {
            cmd.cwd(cwd);
        }
        cmd.env("PS1", "");
        cmd.env("HISTFILE", "/dev/null");
        cmd.env("TERM", "xterm-256color");
        cmd.env_remove("PROMPT_COMMAND");

        let child = pair
            .slave
            .spawn_command(cmd)
            .expect("failed to spawn bash");
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .expect("failed to clone PTY reader");
        let writer = pair.master.take_writer().expect("failed to take PTY writer");

        let io = PtyIO::new(writer, reader);
        let mut session = ShellSession::new(io);
        session.drain_initial(Duration::from_secs(5));
        // Suppress echo so sentinel commands aren't echoed back
        session.execute("stty -echo", Duration::from_secs(2));
        (session, child, pair.master)
    }

    #[test]
    fn test_pty_echo() {
        let _lock = PTY_LOCK.lock().unwrap();
        let (mut session, _child, _master) = spawn_pty_session();
        let result = session.execute_default("echo hello_pty");
        assert_eq!(result.exit_code, 0);
        assert!(
            result.stdout.contains("hello_pty"),
            "expected 'hello_pty' in output, got: {:?}",
            result.stdout
        );
    }

    #[test]
    fn test_pty_exit_code() {
        let _lock = PTY_LOCK.lock().unwrap();
        let (mut session, _child, _master) = spawn_pty_session();
        let result = session.execute_default("(exit 42)");
        assert_eq!(
            result.exit_code, 42,
            "expected exit code 42, got: {}",
            result.exit_code
        );
    }

    #[test]
    fn test_pty_cwd_persistence() {
        let _lock = PTY_LOCK.lock().unwrap();
        let (mut session, _child, _master) = spawn_pty_session();
        session.execute_default("cd /tmp");
        let cwd = session.capture_cwd();
        assert!(
            cwd == "/tmp" || cwd == "/private/tmp",
            "expected /tmp or /private/tmp, got: {:?}",
            cwd
        );
    }

    #[test]
    fn test_pty_no_stderr() {
        let _lock = PTY_LOCK.lock().unwrap();
        let (mut session, _child, _master) = spawn_pty_session();
        let stderr = session.io.drain_stderr();
        assert!(stderr.is_empty(), "PTY should have no separate stderr");
    }
}
