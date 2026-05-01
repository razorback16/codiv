//! `PipeIO`: `ShellIO` implementation over stdin/stdout/stderr pipes.
//!
//! Spawns no process itself — the caller provides `ChildStdin`, `ChildStdout`,
//! and `ChildStderr` from a `std::process::Command`. Background threads drain
//! both stdout and stderr so reads never block the calling thread past the
//! configured timeout.

use super::{read_until_sentinel_shared, ShellIO};
use crossbeam_channel::{self, Receiver, Sender};
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// `ShellIO` backend that communicates via stdin/stdout/stderr pipes.
///
/// A background thread reads stdout and sends chunks through a channel
/// (same pattern as `PtyIO`), giving us proper timeout support on reads.
/// A second background thread drains stderr into a buffer.
pub struct PipeIO {
    stdin: Box<dyn Write + Send>,
    stdout_rx: Receiver<Vec<u8>>,
    stderr_buf: Arc<Mutex<String>>,
    _stdout_handle: Option<JoinHandle<()>>,
    _stderr_handle: Option<JoinHandle<()>>,
}

impl PipeIO {
    /// Create a new `PipeIO` from the given stdin, stdout, and stderr handles.
    ///
    /// Starts background threads that drain stdout (into a channel) and
    /// stderr (into an internal buffer).
    pub fn new(
        stdin: Box<dyn Write + Send>,
        stdout: Box<dyn Read + Send>,
        stderr: Box<dyn Read + Send>,
    ) -> Self {
        // Stdout reader thread → channel (for timeout-aware reads)
        let (stdout_tx, stdout_rx) = crossbeam_channel::unbounded();
        let stdout_handle = thread::Builder::new()
            .name("pipe-stdout-drain".into())
            .spawn(move || Self::reader_thread(stdout, stdout_tx))
            .expect("failed to spawn stdout drain thread");

        // Stderr reader thread → buffer
        let stderr_buf = Arc::new(Mutex::new(String::new()));
        let buf_clone = Arc::clone(&stderr_buf);
        let stderr_handle = thread::Builder::new()
            .name("pipe-stderr-drain".into())
            .spawn(move || Self::drain_stderr_thread(stderr, buf_clone))
            .expect("failed to spawn stderr drain thread");

        Self {
            stdin,
            stdout_rx,
            stderr_buf,
            _stdout_handle: Some(stdout_handle),
            _stderr_handle: Some(stderr_handle),
        }
    }

    fn reader_thread(mut stdout: Box<dyn Read + Send>, tx: Sender<Vec<u8>>) {
        let mut buf = [0u8; 4096];
        loop {
            match stdout.read(&mut buf) {
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

    fn drain_stderr_thread(mut stderr: Box<dyn Read + Send>, buf: Arc<Mutex<String>>) {
        let mut tmp = [0u8; 4096];
        loop {
            match stderr.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = String::from_utf8_lossy(&tmp[..n]);
                    if let Ok(mut locked) = buf.lock() {
                        locked.push_str(&chunk);
                    }
                }
                Err(_) => break,
            }
        }
    }
}

impl ShellIO for PipeIO {
    fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        self.stdin.write_all(data)?;
        self.stdin.flush()
    }

    fn read_until_sentinel(&mut self, sentinel: &str, timeout: Duration) -> String {
        read_until_sentinel_shared(&self.stdout_rx, sentinel, timeout, false)
    }

    fn drain_stderr(&mut self) -> String {
        let mut locked = self.stderr_buf.lock().unwrap_or_else(|e| e.into_inner());
        let result = locked.clone();
        locked.clear();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::ShellSession;
    use std::process::{Command, Stdio};
    use std::time::Duration;

    fn spawn_bash_session() -> (ShellSession<PipeIO>, std::process::Child) {
        let mut child = Command::new("bash")
            .args(["--norc", "--noprofile", "-i"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("PS1", "")
            .env("HISTFILE", "/dev/null")
            .env_remove("PROMPT_COMMAND")
            .spawn()
            .expect("failed to spawn bash");

        let stdin = Box::new(child.stdin.take().unwrap());
        let stdout = Box::new(child.stdout.take().unwrap());
        let stderr = Box::new(child.stderr.take().unwrap());

        let io = PipeIO::new(stdin, stdout, stderr);
        let mut session = ShellSession::new(io);
        session.drain_initial(Duration::from_secs(5));
        (session, child)
    }

    #[test]
    fn test_pipe_echo() {
        let (mut session, mut child) = spawn_bash_session();
        let result = session.execute_default("echo hello_pipe");
        assert_eq!(result.exit_code, 0);
        assert!(
            result.stdout.contains("hello_pipe"),
            "expected 'hello_pipe' in output, got: {:?}",
            result.stdout
        );
        let _ = child.kill();
    }

    #[test]
    fn test_pipe_exit_code() {
        let (mut session, mut child) = spawn_bash_session();
        let result = session.execute_default("(exit 42)");
        assert_eq!(
            result.exit_code, 42,
            "expected exit code 42, got: {}",
            result.exit_code
        );
        let _ = child.kill();
    }

    #[test]
    fn test_pipe_cwd_persistence() {
        let (mut session, mut child) = spawn_bash_session();
        session.execute_default("cd /tmp");
        let cwd = session.capture_cwd();
        // /tmp may be symlinked to /private/tmp on macOS
        assert!(
            cwd == "/tmp" || cwd == "/private/tmp",
            "expected /tmp or /private/tmp, got: {:?}",
            cwd
        );
        let _ = child.kill();
    }

    #[test]
    fn test_pipe_env_persistence() {
        let (mut session, mut child) = spawn_bash_session();
        session.execute_default("export MY_TEST_VAR=hello_from_pipe");
        let env = session.capture_env();
        let found = env.iter().any(|(k, v)| k == "MY_TEST_VAR" && v == "hello_from_pipe");
        assert!(found, "MY_TEST_VAR not found in env: {:?}", env);
        let _ = child.kill();
    }
}
