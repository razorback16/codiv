//! `DaemonShell`: a `ShellSession<PipeIO>` wrapped in a `Mutex` for use by the
//! daemon. Hardcodes `bash` as the shell (matching the spec) and kills the
//! child process on drop.

use codiv_common::shell::{pipe_io::PipeIO, CommandOutput, ShellSession};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

/// Thread-safe shell session for the daemon.
///
/// Wraps `ShellSession<PipeIO>` and the child process handle in a `Mutex`
/// so that multiple async tasks can share a single shell.
pub struct DaemonShell {
    inner: Mutex<DaemonShellInner>,
}

struct DaemonShellInner {
    session: ShellSession<PipeIO>,
    child: Child,
}

impl DaemonShell {
    /// Spawn a new daemon shell using `bash -i` in the given directory.
    /// Uses interactive mode so `.bashrc` is sourced (access to nvm, pyenv, etc.).
    pub fn spawn(initial_cwd: &str) -> std::io::Result<Self> {
        let mut child = Command::new("bash")
            .arg("-i")
            .current_dir(initial_cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("PS1", "")
            .env("HISTFILE", "/dev/null")
            .env("GIT_PAGER", "cat")
            .env("PAGER", "cat")
            .env("SYSTEMD_PAGER", "cat")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_EDITOR", "true")
            .env_remove("PROMPT_COMMAND")
            .spawn()?;

        let stdin = Box::new(child.stdin.take().unwrap());
        let stdout = Box::new(child.stdout.take().unwrap());
        let stderr = Box::new(child.stderr.take().unwrap());

        let io = PipeIO::new(stdin, stdout, stderr);
        let mut session = ShellSession::new(io);
        session.drain_initial(Duration::from_secs(5));

        Ok(Self {
            inner: Mutex::new(DaemonShellInner { session, child }),
        })
    }

    /// Execute a command and return output, exit code, and stderr.
    pub fn execute(&self, command: &str, timeout: Duration) -> CommandOutput {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.session.execute(command, timeout)
    }

    /// Execute with a default 30-second timeout.
    pub fn execute_default(&self, command: &str) -> CommandOutput {
        self.execute(command, Duration::from_secs(30))
    }

    /// Capture the shell's current working directory.
    pub fn capture_cwd(&self) -> String {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.session.capture_cwd()
    }

    /// Capture the shell's current environment variables.
    pub fn capture_env(&self) -> Vec<(String, String)> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.session.capture_env()
    }

    /// Get the child process ID (useful for kill-on-drop verification).
    pub fn child_pid(&self) -> u32 {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.child.id()
    }
}

impl Drop for DaemonShell {
    fn drop(&mut self) {
        if let Ok(mut inner) = self.inner.lock() {
            let _ = inner.child.kill();
            let _ = inner.child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daemon_shell_execute() {
        let shell = DaemonShell::spawn("/tmp").expect("failed to spawn daemon shell");
        let result = shell.execute_default("echo hello_daemon");
        assert_eq!(result.exit_code, 0);
        assert!(
            result.stdout.contains("hello_daemon"),
            "expected 'hello_daemon' in output, got: {:?}",
            result.stdout
        );
    }

    #[test]
    fn test_daemon_shell_cwd_persistence() {
        let shell = DaemonShell::spawn("/tmp").expect("failed to spawn daemon shell");
        shell.execute_default("cd /tmp");
        let cwd = shell.capture_cwd();
        assert!(
            cwd == "/tmp" || cwd == "/private/tmp",
            "expected /tmp or /private/tmp, got: {:?}",
            cwd
        );
    }

    #[test]
    fn test_daemon_shell_env_persistence() {
        let shell = DaemonShell::spawn("/tmp").expect("failed to spawn daemon shell");
        shell.execute_default("export DAEMON_TEST_VAR=from_daemon");
        let env = shell.capture_env();
        let found = env
            .iter()
            .any(|(k, v)| k == "DAEMON_TEST_VAR" && v == "from_daemon");
        assert!(found, "DAEMON_TEST_VAR not found in env: {:?}", env);
    }

    #[test]
    fn test_daemon_shell_exit_code() {
        let shell = DaemonShell::spawn("/tmp").expect("failed to spawn daemon shell");
        let result = shell.execute_default("(exit 42)");
        assert_eq!(
            result.exit_code, 42,
            "expected exit code 42, got: {}",
            result.exit_code
        );
    }

    #[test]
    fn test_daemon_shell_drop_kills_child() {
        let pid;
        {
            let shell = DaemonShell::spawn("/tmp").expect("failed to spawn daemon shell");
            pid = shell.child_pid();
            // shell is dropped here
        }

        // Give the OS a moment to clean up
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Verify the process is no longer alive
        let alive = unsafe { libc::kill(pid as i32, 0) } == 0;
        assert!(
            !alive,
            "child process (pid {}) should be dead after drop",
            pid
        );
    }
}
