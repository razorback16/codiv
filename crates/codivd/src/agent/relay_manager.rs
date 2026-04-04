use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use codiv_common::messages::{
    frame_message, CancelReason, DaemonMessage, ReleaseReason,
};
use codiv_common::truncate::truncate_output;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;

// ---

/// Outcome of a relayed bash command, returned to the tool closure.
#[derive(Debug, Clone)]
pub enum RelayOutcome {
    Completed {
        output: String,
        exit_code: i32,
    },
    Cancelled,
    TimedOut,
    Failed(String),
}

/// Phase of a shell lease.
#[derive(Debug, Clone, PartialEq)]
pub enum LeasePhase {
    Requested,
    Queued,
    Acquired,
    Released,
    Cancelled,
}

/// Phase of a command within a lease.
#[derive(Debug, Clone, PartialEq)]
pub enum CommandPhase {
    Sent,
    Started { started_at: Instant },
    Completed,
    TimedOut,
    Cancelled,
    Failed(String),
}

impl LeasePhase {
    fn is_terminal(&self) -> bool {
        matches!(self, LeasePhase::Released | LeasePhase::Cancelled)
    }
}

impl CommandPhase {
    fn is_terminal(&self) -> bool {
        matches!(
            self,
            CommandPhase::Completed | CommandPhase::TimedOut | CommandPhase::Cancelled | CommandPhase::Failed(_)
        )
    }
}

/// State for a command within a relay operation.
pub struct CommandOp {
    pub execution_id: String,
    pub phase: CommandPhase,
    pub execution_timeout_ms: u64,
}

/// A single relay operation tracking lease + command lifecycle.
pub struct RelayOp {
    pub lease_phase: LeasePhase,
    pub command: Option<CommandOp>,
    /// Sends the final result back to the awaiting tool closure.
    pub result_tx: Option<oneshot::Sender<RelayOutcome>>,
    /// Notifies the tool closure that the lease has been acquired.
    pub lease_acquired_tx: Option<oneshot::Sender<()>>,
    /// Handle for the execution timeout task (so we can cancel it).
    pub timeout_task: Option<JoinHandle<()>>,
}

// ---

/// Manages the shell lease protocol between daemon and client.
///
/// Replaces the old `ShellBackend` with an async, state-machine-based approach.
/// Each bash tool call goes through: acquire lease → send command → await
/// started ACK (starts timeout) → await completion → release lease.
pub struct RelayManager {
    client_tx: mpsc::Sender<Vec<u8>>,
    ops: Mutex<HashMap<String, RelayOp>>,
    /// Reverse index: execution_id → lease_id for fast dispatch.
    exec_index: Mutex<HashMap<String, String>>,
}

impl RelayManager {
    pub fn new(client_tx: mpsc::Sender<Vec<u8>>) -> Self {
        Self {
            client_tx,
            ops: Mutex::new(HashMap::new()),
            exec_index: Mutex::new(HashMap::new()),
        }
    }

    /// Execute a bash command through the client relay.
    ///
    /// This is the single entry point called by the async bash tool closure.
    /// It acquires a lease, sends the command, waits for completion, and
    /// releases the lease — all fully async with no `block_in_place`.
    pub async fn execute_bash(
        self: &Arc<Self>,
        request_id: String,
        command: String,
        execution_timeout_ms: u64,
    ) -> Result<String, String> {
        let lease_id = uuid_v4();
        let execution_id = uuid_v4();

        // Create channels for lease acquisition and final result.
        let (lease_tx, lease_rx) = oneshot::channel();
        let (result_tx, result_rx) = oneshot::channel();

        // Register the operation.
        {
            let mut ops = self.ops.lock().await;
            ops.insert(
                lease_id.clone(),
                RelayOp {
                    lease_phase: LeasePhase::Requested,
                    command: None,
                    result_tx: Some(result_tx),
                    lease_acquired_tx: Some(lease_tx),
                    timeout_task: None,
                },
            );
        }

        // Send AcquireShellLease to client.
        self.send_daemon_msg(&DaemonMessage::AcquireShellLease {
            lease_id: lease_id.clone(),
            request_id: request_id.clone(),
        })
        .await?;

        // Wait for lease acquisition (no timeout — queue wait is unbounded,
        // but will be cancelled if the agent task is aborted).
        match lease_rx.await {
            Ok(()) => {}
            Err(_) => {
                self.cleanup_op(&lease_id).await;
                return Err("lease acquisition cancelled".to_string());
            }
        }

        // Register the command and execution_id index.
        {
            let mut ops = self.ops.lock().await;
            if let Some(op) = ops.get_mut(&lease_id) {
                op.command = Some(CommandOp {
                    execution_id: execution_id.clone(),
                    phase: CommandPhase::Sent,
                    execution_timeout_ms,
                });
            }
            let mut idx = self.exec_index.lock().await;
            idx.insert(execution_id.clone(), lease_id.clone());
        }

        // Send ExecuteLeasedCommand.
        self.send_daemon_msg(&DaemonMessage::ExecuteLeasedCommand {
            lease_id: lease_id.clone(),
            execution_id: execution_id.clone(),
            command,
            execution_timeout_ms,
        })
        .await?;

        // Wait for the command result.
        let outcome = match result_rx.await {
            Ok(outcome) => outcome,
            Err(_) => {
                self.cleanup_op(&lease_id).await;
                return Err("relay cancelled".to_string());
            }
        };

        // Release the lease.
        // Note: cwd updates from command completion are handled by the daemon
        // event loop when it processes CommandCompleted messages, updating
        // ClientSession.cwd as the single source of truth.
        let reason = match &outcome {
            RelayOutcome::Completed { .. } => ReleaseReason::Completed,
            _ => ReleaseReason::Cancelled,
        };
        let _ = self.send_daemon_msg(&DaemonMessage::ReleaseShellLease {
            lease_id: lease_id.clone(),
            reason,
        }).await;

        // Clean up state.
        self.cleanup_op(&lease_id).await;

        // Format the result.
        match outcome {
            RelayOutcome::Completed {
                output, exit_code, ..
            } => Ok(format_output(&output, exit_code)),
            RelayOutcome::Cancelled => Err("command cancelled".to_string()),
            RelayOutcome::TimedOut => Err("command timed out".to_string()),
            RelayOutcome::Failed(e) => Err(e),
        }
    }

    // --- Client message handlers ---

    /// Route an incoming client message to the appropriate state transition.
    /// Returns true if the message was handled (i.e. it was a lease protocol message).
    pub async fn handle_client_message(
        self: &Arc<Self>,
        msg: &codiv_common::messages::ClientMessage,
    ) -> bool {
        use codiv_common::messages::ClientMessage;
        match msg {
            ClientMessage::ShellLeaseAcquired { lease_id } => {
                self.on_lease_acquired(lease_id).await;
                true
            }
            ClientMessage::ShellLeaseQueued { lease_id, .. } => {
                self.on_lease_queued(lease_id).await;
                true
            }
            ClientMessage::CommandStarted {
                lease_id,
                execution_id,
            } => {
                self.on_command_started(lease_id, execution_id).await;
                true
            }
            ClientMessage::CommandCompleted {
                lease_id,
                execution_id,
                output,
                exit_code,
                cwd: _,
            } => {
                self.on_command_completed(lease_id, execution_id, output, *exit_code)
                    .await;
                true
            }
            ClientMessage::CommandFailed {
                lease_id,
                execution_id,
                error,
                ..
            } => {
                self.on_command_failed(lease_id, execution_id, error).await;
                true
            }
            ClientMessage::CommandCancelled {
                lease_id,
                execution_id,
                ..
            } => {
                self.on_command_cancelled(lease_id, execution_id).await;
                true
            }
            ClientMessage::ShellLeaseReleased { lease_id } => {
                self.on_lease_released(lease_id).await;
                true
            }
            _ => false,
        }
    }

    async fn on_lease_acquired(&self, lease_id: &str) {
        let mut ops = self.ops.lock().await;
        if let Some(op) = ops.get_mut(lease_id) {
            if op.lease_phase == LeasePhase::Requested || op.lease_phase == LeasePhase::Queued {
                op.lease_phase = LeasePhase::Acquired;
                if let Some(tx) = op.lease_acquired_tx.take() {
                    let _ = tx.send(());
                }
            }
        }
    }

    async fn on_lease_queued(&self, lease_id: &str) {
        let mut ops = self.ops.lock().await;
        if let Some(op) = ops.get_mut(lease_id) {
            if op.lease_phase == LeasePhase::Requested {
                op.lease_phase = LeasePhase::Queued;
            }
        }
    }

    async fn on_command_started(self: &Arc<Self>, lease_id: &str, execution_id: &str) {
        let mgr = Arc::clone(self);
        let lease_id_owned = lease_id.to_string();
        let execution_id_owned = execution_id.to_string();

        let mut ops = self.ops.lock().await;
        if let Some(op) = ops.get_mut(lease_id) {
            if let Some(ref mut cmd) = op.command {
                if cmd.execution_id == execution_id && cmd.phase == CommandPhase::Sent {
                    cmd.phase = CommandPhase::Started {
                        started_at: Instant::now(),
                    };

                    // Spawn execution timeout task — timer starts NOW, not at send time.
                    let timeout_ms = cmd.execution_timeout_ms;
                    let timeout_handle = tokio::spawn(async move {
                        // Add 5s buffer beyond the command's own timeout.
                        tokio::time::sleep(std::time::Duration::from_millis(
                            timeout_ms.saturating_add(5000),
                        ))
                        .await;
                        mgr.on_execution_timeout(&lease_id_owned, &execution_id_owned)
                            .await;
                    });
                    op.timeout_task = Some(timeout_handle);
                }
            }
        }
    }

    async fn on_command_completed(
        &self,
        lease_id: &str,
        execution_id: &str,
        output: &str,
        exit_code: i32,
    ) {
        let mut ops = self.ops.lock().await;
        if let Some(op) = ops.get_mut(lease_id) {
            // Cancel the timeout task.
            if let Some(handle) = op.timeout_task.take() {
                handle.abort();
            }

            if let Some(ref mut cmd) = op.command {
                if cmd.execution_id == execution_id && !cmd.phase.is_terminal() {
                    cmd.phase = CommandPhase::Completed;
                    if let Some(tx) = op.result_tx.take() {
                        let _ = tx.send(RelayOutcome::Completed {
                            output: output.to_string(),
                            exit_code,
                        });
                    }
                }
                // Late completion for already-terminal op is silently ignored.
            }
        }
    }

    async fn on_command_failed(&self, lease_id: &str, execution_id: &str, error: &str) {
        let mut ops = self.ops.lock().await;
        if let Some(op) = ops.get_mut(lease_id) {
            if let Some(handle) = op.timeout_task.take() {
                handle.abort();
            }
            if let Some(ref mut cmd) = op.command {
                if cmd.execution_id == execution_id && !cmd.phase.is_terminal() {
                    cmd.phase = CommandPhase::Failed(error.to_string());
                    if let Some(tx) = op.result_tx.take() {
                        let _ = tx.send(RelayOutcome::Failed(error.to_string()));
                    }
                }
            }
        }
    }

    async fn on_command_cancelled(&self, lease_id: &str, execution_id: &str) {
        let mut ops = self.ops.lock().await;
        if let Some(op) = ops.get_mut(lease_id) {
            if let Some(handle) = op.timeout_task.take() {
                handle.abort();
            }
            if let Some(ref mut cmd) = op.command {
                if cmd.execution_id == execution_id && !cmd.phase.is_terminal() {
                    cmd.phase = CommandPhase::Cancelled;
                    if let Some(tx) = op.result_tx.take() {
                        let _ = tx.send(RelayOutcome::Cancelled);
                    }
                }
            }
        }
    }

    async fn on_lease_released(&self, lease_id: &str) {
        let mut ops = self.ops.lock().await;
        if let Some(op) = ops.get_mut(lease_id) {
            if !op.lease_phase.is_terminal() {
                op.lease_phase = LeasePhase::Released;
            }
        }
    }

    async fn on_execution_timeout(self: &Arc<Self>, lease_id: &str, execution_id: &str) {
        let should_cancel = {
            let mut ops = self.ops.lock().await;
            if let Some(op) = ops.get_mut(lease_id) {
                if let Some(ref mut cmd) = op.command {
                    if cmd.execution_id == execution_id
                        && matches!(cmd.phase, CommandPhase::Started { .. })
                    {
                        cmd.phase = CommandPhase::TimedOut;
                        if let Some(tx) = op.result_tx.take() {
                            let _ = tx.send(RelayOutcome::TimedOut);
                        }
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            }
        };

        if should_cancel {
            // Tell client to cancel the command.
            let _ = self
                .send_daemon_msg(&DaemonMessage::CancelLeasedCommand {
                    lease_id: lease_id.to_string(),
                    execution_id: execution_id.to_string(),
                    reason: CancelReason::ExecutionTimeout,
                })
                .await;
        }
    }

    // --- Cancellation ---

    /// Cancel all pending relay operations (e.g. on user abort or disconnect).
    pub async fn cancel_all(&self, _reason: CancelReason) {
        let mut ops = self.ops.lock().await;
        for (_lease_id, op) in ops.iter_mut() {
            // Cancel timeout task.
            if let Some(handle) = op.timeout_task.take() {
                handle.abort();
            }
            // Resolve lease waiters — dropping sender causes Err on receiver.
            op.lease_acquired_tx.take();
            // Resolve command waiters.
            if let Some(ref mut cmd) = op.command {
                if !cmd.phase.is_terminal() {
                    cmd.phase = CommandPhase::Cancelled;
                }
            }
            if let Some(tx) = op.result_tx.take() {
                let _ = tx.send(RelayOutcome::Cancelled);
            }
            if !op.lease_phase.is_terminal() {
                op.lease_phase = LeasePhase::Cancelled;
            }
        }
        ops.clear();
        self.exec_index.lock().await.clear();
    }

    // --- Internal helpers ---

    async fn send_daemon_msg(&self, msg: &DaemonMessage) -> Result<(), String> {
        let frame = frame_message(msg).map_err(|e| format!("frame error: {}", e))?;
        self.client_tx
            .send(frame)
            .await
            .map_err(|e| format!("send error: {}", e))
    }

    async fn cleanup_op(&self, lease_id: &str) {
        let mut ops = self.ops.lock().await;
        if let Some(op) = ops.remove(lease_id) {
            if let Some(handle) = op.timeout_task {
                handle.abort();
            }
            if let Some(ref cmd) = op.command {
                self.exec_index.lock().await.remove(&cmd.execution_id);
            }
        }
    }
}

// ---

/// Format command output with exit code.
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

fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}
