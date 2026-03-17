use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use codiv_common::messages::{frame_message, DaemonMessage};
use codiv_common::truncate::truncate_output;
use tokio::sync::{mpsc, oneshot};

/// Result returned from the client after executing a relayed command.
#[derive(Debug, Clone)]
pub struct RelayResult {
    pub output: String,
    pub exit_code: i32,
    pub cwd: String,
}

/// Backend for executing shell commands: either relayed through the connected
/// client or executed locally via a `DaemonShell` (not yet wired).
#[derive(Clone)]
pub enum ShellBackend {
    /// Route commands through the connected client's terminal.
    ClientRelay {
        client_tx: mpsc::Sender<Vec<u8>>,
        pending: Arc<Mutex<HashMap<String, oneshot::Sender<RelayResult>>>>,
        handle: tokio::runtime::Handle,
    },
}

impl ShellBackend {
    /// Execute a command, blocking until the result is available.
    /// For `ClientRelay`, sends an `ExecuteCommand` IPC message and waits
    /// for the client to respond with `CommandExecutionResult`.
    pub fn execute(&self, command: &str, timeout_ms: u64, cwd: &str) -> Result<String, String> {
        match self {
            ShellBackend::ClientRelay {
                client_tx,
                pending,
                handle,
            } => {
                let execution_id = uuid::Uuid::new_v4().to_string();
                let (tx, rx) = oneshot::channel();

                // Register the pending execution
                {
                    let mut map = pending.lock().map_err(|e| format!("lock error: {}", e))?;
                    map.insert(execution_id.clone(), tx);
                }

                // Send ExecuteCommand to the client
                let msg = DaemonMessage::ExecuteCommand {
                    execution_id: execution_id.clone(),
                    command: command.to_string(),
                    timeout_ms,
                };
                let frame =
                    frame_message(&msg).map_err(|e| format!("frame error: {}", e))?;

                let client_tx = client_tx.clone();
                handle.block_on(async {
                    client_tx
                        .send(frame)
                        .await
                        .map_err(|e| format!("send error: {}", e))
                })?;

                // Wait for the result with a timeout slightly longer than the command timeout
                let wait_timeout =
                    std::time::Duration::from_millis(timeout_ms.saturating_add(5000));
                let result = handle.block_on(async {
                    tokio::time::timeout(wait_timeout, rx)
                        .await
                        .map_err(|_| "relay timeout waiting for client response".to_string())?
                        .map_err(|_| "relay channel closed".to_string())
                })?;

                Ok(format_output(&result.output, result.exit_code))
            }
        }
    }

    /// Resolve a pending relay execution with the given result.
    pub fn resolve_pending(
        pending: &Arc<Mutex<HashMap<String, oneshot::Sender<RelayResult>>>>,
        execution_id: &str,
        result: RelayResult,
    ) {
        let sender = {
            let mut map = pending.lock().unwrap();
            map.remove(execution_id)
        };
        if let Some(sender) = sender {
            let _ = sender.send(result);
        }
    }

    /// Fail all pending executions (e.g. on client disconnect).
    pub fn fail_all_pending(
        pending: &Arc<Mutex<HashMap<String, oneshot::Sender<RelayResult>>>>,
    ) {
        let mut map = pending.lock().unwrap();
        // Dropping all senders causes receivers to get a RecvError
        map.clear();
    }
}

/// Format command output with exit code, matching the style used by bash.rs.
pub fn format_output(output: &str, exit_code: i32) -> String {
    let mut result = output.to_string();

    if exit_code != 0 {
        if result.is_empty() {
            result.push_str(&format!("failed with exit code: {}", exit_code));
        } else {
            if !result.ends_with('\n') {
                result.push('\n');
            }
            result.push_str(&format!("exit code: {}", exit_code));
        }
    } else if result.is_empty() {
        result.push_str(&format!("success, exit code: {}", exit_code));
    }

    truncate_output(&result, 200, 100)
}
