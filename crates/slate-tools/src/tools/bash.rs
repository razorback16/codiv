use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use slate_common::truncate::truncate_output;

#[derive(Deserialize, schemars::JsonSchema)]
pub struct BashInput {
    /// The bash command to execute
    pub command: String,
    /// Timeout in milliseconds (default: 30000)
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

fn default_timeout() -> u64 {
    30000
}

/// Maximum bytes to keep from each of stdout/stderr.
/// We continue draining the pipe beyond this to avoid blocking the child,
/// but discard excess bytes.
const MAX_OUTPUT_BYTES: usize = 512 * 1024;

/// Read from `reader` into a `String`, capping at `max_bytes`.
/// Excess bytes are read and discarded so the child process doesn't block.
fn read_capped(mut reader: impl std::io::Read, max_bytes: usize) -> String {
    let mut kept = Vec::with_capacity(max_bytes.min(8192));
    let mut buf = [0u8; 8192];
    let mut total = 0usize;

    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if total < max_bytes {
                    let take = n.min(max_bytes - total);
                    kept.extend_from_slice(&buf[..take]);
                }
                total += n;
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }

    String::from_utf8_lossy(&kept).into_owned()
}


pub fn execute(value: Value, cwd: &str, env_vars: &[(String, String)]) -> Result<String, String> {
    let input: BashInput =
        serde_json::from_value(value).map_err(|e| format!("invalid bash input: {e}"))?;

    tracing::debug!(command = %input.command, timeout_ms = input.timeout_ms, "bash");

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "bash".into());
    let mut child = Command::new(&shell)
        .arg("-c")
        .arg(&input.command)
        .current_dir(cwd)
        .envs(env_vars.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn bash: {e}"))?;

    // Take pipe handles before wrapping child in Arc<Mutex<>>
    let stdout_pipe = child.stdout.take().expect("stdout piped");
    let stderr_pipe = child.stderr.take().expect("stderr piped");

    let child = Arc::new(Mutex::new(child));
    let timed_out = Arc::new(AtomicBool::new(false));

    // --- Timeout thread ---
    let (cancel_tx, cancel_rx) = crossbeam_channel::bounded::<()>(0);
    let timeout_child = Arc::clone(&child);
    let timeout_flag = Arc::clone(&timed_out);
    let timeout_ms = input.timeout_ms;

    let timeout_thread = std::thread::Builder::new()
        .name("bash-timeout".into())
        .spawn(move || {
            if let Err(crossbeam_channel::RecvTimeoutError::Timeout) = cancel_rx.recv_timeout(Duration::from_millis(timeout_ms)) {
                timeout_flag.store(true, Ordering::SeqCst);
                if let Ok(mut c) = timeout_child.lock() {
                    let _ = c.kill();
                }
            }
            // Cancelled (sender dropped) or received signal — exit cleanly
        })
        .map_err(|e| format!("failed to spawn timeout thread: {e}"))?;

    // --- Stderr reader thread ---
    let stderr_thread = std::thread::Builder::new()
        .name("bash-stderr".into())
        .spawn(move || read_capped(stderr_pipe, MAX_OUTPUT_BYTES))
        .map_err(|e| format!("failed to spawn stderr thread: {e}"))?;

    // --- Read stdout on current thread ---
    let stdout_output = read_capped(stdout_pipe, MAX_OUTPUT_BYTES);

    // --- Join stderr ---
    let stderr_output = stderr_thread
        .join()
        .map_err(|_| "stderr reader thread panicked".to_string())?;

    // --- Cancel timeout thread and join ---
    drop(cancel_tx);
    timeout_thread
        .join()
        .map_err(|_| "timeout thread panicked".to_string())?;

    // --- Wait for child exit status ---
    let status = child
        .lock()
        .map_err(|_| "child mutex poisoned".to_string())?
        .wait()
        .map_err(|e| format!("wait error: {e}"))?;

    // --- Build result ---
    let mut result = stdout_output;

    if !stderr_output.is_empty() {
        if !result.is_empty() && !result.ends_with('\n') {
            result.push('\n');
        }
        result.push_str("<stderr>\n");
        result.push_str(&stderr_output);
        if !stderr_output.ends_with('\n') {
            result.push('\n');
        }
        result.push_str("</stderr>");
    }

    if timed_out.load(Ordering::SeqCst) {
        if !result.is_empty() && !result.ends_with('\n') {
            result.push('\n');
        }
        result.push_str(&format!("[timed out after {}ms]", timeout_ms));
    }

    let code = status.code().unwrap_or(-1);
    if !status.success() {
        if result.is_empty() {
            result.push_str(&format!("failed with exit code: {code}"));
        } else {
            if !result.ends_with('\n') {
                result.push('\n');
            }
            result.push_str(&format!("exit code: {code}"));
        }
    } else if result.is_empty() {
        result.push_str(&format!("success, exit code: {code}"));
    }

    let result = truncate_output(&result, 200, 100);

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(cmd: &str, timeout_ms: u64) -> Result<String, String> {
        let value = json!({ "command": cmd, "timeout_ms": timeout_ms });
        let cwd = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        execute(value, &cwd, &[])
    }

    #[test]
    fn test_basic_echo() {
        let result = run("echo hello", 5000).unwrap();
        assert!(result.contains("hello"), "expected 'hello', got: {result}");
    }

    #[test]
    fn test_exit_code() {
        let result = run("exit 42", 5000).unwrap();
        assert!(
            result.contains("exit code: 42"),
            "expected 'exit code: 42', got: {result}"
        );
    }

    #[test]
    fn test_stdin_null() {
        // `cat` with no args reads stdin; with stdin null it should exit immediately
        let result = run("cat", 3000).unwrap();
        // Should complete without timeout
        assert!(
            !result.contains("timed out"),
            "cat should not have timed out: {result}"
        );
    }

    #[test]
    fn test_timeout() {
        let start = std::time::Instant::now();
        let result = run("sleep 60", 500).unwrap();
        let elapsed = start.elapsed();

        assert!(
            result.contains("timed out after 500ms"),
            "expected timeout message, got: {result}"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "should have timed out quickly, took {:?}",
            elapsed
        );
    }

    #[test]
    fn test_concurrent_stdout_stderr() {
        // Write 5000 lines to each pipe — would deadlock with sequential reads
        // if the pipe buffer (typically 64KB) fills up
        let cmd = r#"
            for i in $(seq 1 5000); do
                echo "stdout line $i"
                echo "stderr line $i" >&2
            done
        "#;
        let result = run(cmd, 30000).unwrap();
        // Should complete without timeout (deadlock would cause timeout)
        assert!(
            !result.contains("timed out"),
            "should not have timed out (deadlock?): {result}"
        );
        // stdout lines should be in head portion
        assert!(
            result.contains("stdout line 1"),
            "missing stdout: {result}"
        );
        // stderr content should appear somewhere (in tail after truncation)
        assert!(
            result.contains("<stderr>") || result.contains("stderr line"),
            "missing stderr content: {result}"
        );
    }

    #[test]
    fn test_large_output_truncated() {
        let result = run("seq 1 10000", 10000).unwrap();
        assert!(
            result.contains("lines omitted"),
            "expected truncation marker, got length: {}",
            result.len()
        );
        // Should have first lines and last lines
        assert!(result.contains("1\n"), "missing first line");
        assert!(result.contains("10000"), "missing last line");
    }
}
