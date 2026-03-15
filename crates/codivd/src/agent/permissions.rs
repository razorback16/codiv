use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use serde_json::Value;
use codiv_common::messages::{DaemonMessage, RiskLevel};
use codiv_common::permissions::{PermissionDecision, PermissionMode};
use tokio::sync::{mpsc, oneshot};

use super::config::ModelCatalog;
use super::permission_evaluator::evaluate_permission;
use super::risk_classifier::classify_risk;

/// Shared permission state for a session.
pub struct PermissionContext {
    /// Current permission mode (can be changed at runtime via Ctrl+P).
    pub mode: RwLock<PermissionMode>,
    /// IPC sender to the client (for sending ConfirmationRequests).
    pub client_tx: mpsc::Sender<Vec<u8>>,
    /// Pending confirmation requests waiting for user response.
    pub pending: std::sync::Mutex<HashMap<String, oneshot::Sender<ConfirmationResult>>>,
    /// Metadata for pending confirmation requests (tool_name, args).
    pub pending_meta: std::sync::Mutex<HashMap<String, (String, serde_json::Value)>>,
    /// Model catalog for LLM evaluator calls.
    pub catalog: ModelCatalog,
    /// Recent conversation context for the LLM evaluator.
    pub context_summary: RwLock<String>,
}

/// Result of a user confirmation.
#[allow(dead_code)]
pub struct ConfirmationResult {
    pub approved: bool,
    pub add_to_allowlist: bool,
    pub add_to_denylist: bool,
    pub comment: Option<String>,
}

impl PermissionContext {
    pub fn new(
        mode: PermissionMode,
        client_tx: mpsc::Sender<Vec<u8>>,
        catalog: ModelCatalog,
    ) -> Self {
        Self {
            mode: RwLock::new(mode),
            client_tx,
            pending: std::sync::Mutex::new(HashMap::new()),
            pending_meta: std::sync::Mutex::new(HashMap::new()),
            catalog,
            context_summary: RwLock::new(String::new()),
        }
    }

    pub fn current_mode(&self) -> PermissionMode {
        *self.mode.read().unwrap()
    }

    pub fn set_mode(&self, mode: PermissionMode) {
        *self.mode.write().unwrap() = mode;
    }

    /// Update the conversation context summary for the LLM evaluator.
    pub fn set_context(&self, summary: String) {
        *self.context_summary.write().unwrap() = summary;
    }

    pub fn context_summary(&self) -> String {
        self.context_summary.read().unwrap().clone()
    }
}

/// Wrap a tool closure with permission checking.
/// Returns a new closure that checks permissions before invoking the original.
pub fn wrap_with_permissions(
    tool_name: String,
    original: Box<dyn Fn(Value) -> Result<String, String> + Send + Sync>,
    ctx: Arc<PermissionContext>,
) -> Box<dyn Fn(Value) -> Result<String, String> + Send + Sync> {
    Box::new(move |args: Value| {
        let command_prefix = super::permission_evaluator::extract_args_pattern(&tool_name, &args);

        // Check config.toml persistent permissions
        {
            let cfg = crate::agent::config::AppConfig::load();
            if let Some(d) = cfg.permissions.lookup(&tool_name, command_prefix.as_deref()) {
                if d == "allow" { return original(args); }
                if d == "deny" { return Err("Denied by saved permission rule".into()); }
            }
        }

        let risk = classify_risk(&tool_name, &args);
        let mode = ctx.current_mode();
        let decision = evaluate_permission(mode, &tool_name, &args, risk);

        match decision {
            PermissionDecision::Allow => original(args),
            PermissionDecision::Deny => Err("Denied by permission policy".into()),
            PermissionDecision::Prompt => {
                // Use block_in_place to bridge async confirmation into sync closure
                let result = tokio::task::block_in_place(|| {
                    let rt = tokio::runtime::Handle::current();
                    rt.block_on(request_confirmation(&ctx, &tool_name, &args, risk))
                });
                match result {
                    Ok(confirmation) if confirmation.approved => {
                        send_permission_outcome(&ctx, &tool_name, true, "Allowed");
                        original(args)
                    }
                    Ok(confirmation) => {
                        let is_timeout = confirmation.comment.as_deref()
                            .map(|c| c.contains("Timed out"))
                            .unwrap_or(false);
                        let reason = if is_timeout { "Denied because of timeout" } else { "Denied" };
                        send_permission_outcome(&ctx, &tool_name, false, reason);
                        let msg = confirmation.comment.unwrap_or_else(|| "Action rejected by user".to_string());
                        Err(msg)
                    }
                    Err(e) => {
                        send_permission_outcome(&ctx, &tool_name, false, "Confirmation channel error");
                        Err(format!("Confirmation failed: {}", e))
                    }
                }
            }
            PermissionDecision::LlmEvaluate => {
                // Use a std thread with a timeout to avoid blocking the tokio
                // runtime. tokio::time::timeout doesn't work reliably inside
                // block_in_place + block_on because the timer driver can stall.
                let eval_tool = tool_name.clone();
                let eval_args = args.clone();
                let eval_catalog = ctx.catalog.clone();
                let eval_context = ctx.context_summary();
                let llm_decision = std::thread::scope(|s| {
                    let handle = s.spawn(|| {
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .expect("failed to build evaluator runtime");
                        rt.block_on(super::llm_evaluator::llm_evaluate_risk(
                            &eval_tool,
                            &eval_args,
                            &eval_catalog,
                            &eval_context,
                        ))
                    });
                    match handle.join() {
                        Ok(decision) => decision,
                        Err(_) => {
                            tracing::warn!("LLM evaluator thread panicked, defaulting to Prompt");
                            PermissionDecision::Prompt
                        }
                    }
                });
                match llm_decision {
                    PermissionDecision::Allow => {
                        send_permission_outcome(&ctx, &tool_name, true, "Allowed by AI evaluator");
                        original(args)
                    }
                    _ => {
                        // LLM said Prompt — fall through to user confirmation
                        let result = tokio::task::block_in_place(|| {
                            let rt = tokio::runtime::Handle::current();
                            rt.block_on(request_confirmation(&ctx, &tool_name, &args, risk))
                        });
                        match result {
                            Ok(confirmation) if confirmation.approved => {
                                send_permission_outcome(&ctx, &tool_name, true, "Allowed");
                                original(args)
                            }
                            Ok(confirmation) => {
                                let is_timeout = confirmation.comment.as_deref()
                                    .map(|c| c.contains("Timed out"))
                                    .unwrap_or(false);
                                let reason = if is_timeout { "Denied because of timeout" } else { "Denied" };
                                send_permission_outcome(&ctx, &tool_name, false, reason);
                                let msg = confirmation.comment.unwrap_or_else(|| "Action rejected by user".to_string());
                                Err(msg)
                            }
                            Err(e) => {
                                send_permission_outcome(&ctx, &tool_name, false, "Confirmation channel error");
                                Err(format!("Confirmation failed: {}", e))
                            }
                        }
                    }
                }
            }
        }
    })
}

/// Send a confirmation request to the client and wait for response.
async fn request_confirmation(
    ctx: &PermissionContext,
    tool_name: &str,
    args: &Value,
    risk: RiskLevel,
) -> Result<ConfirmationResult, String> {
    let request_id = uuid_v4();
    let (tx, rx) = oneshot::channel();

    // Store the sender so daemon can resolve it
    {
        let mut pending = ctx.pending.lock().map_err(|e| e.to_string())?;
        pending.insert(request_id.clone(), tx);
    }

    // Store metadata so the daemon can look up tool_name/args when persisting decisions
    {
        let mut meta = ctx.pending_meta.lock().map_err(|e| e.to_string())?;
        meta.insert(request_id.clone(), (tool_name.to_string(), args.clone()));
    }

    // Build description
    let args_summary = match tool_name {
        "bash" => args.get("command").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        "write" => args.get("file_path").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        "edit" => args.get("file_path").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        _ => serde_json::to_string(args).unwrap_or_default(),
    };
    let description = format!("{} `{}`", tool_name, args_summary);

    // Send confirmation request to client
    let msg = DaemonMessage::ConfirmationRequest {
        request_id: request_id.clone(),
        description,
        risk,
        tool_name: tool_name.to_string(),
        tool_args: serde_json::to_string(args).unwrap_or_default(),
    };
    let frame = codiv_common::messages::frame_message(&msg).map_err(|e| e.to_string())?;
    ctx.client_tx.send(frame).await.map_err(|e| e.to_string())?;

    // Wait for response with 60-second timeout
    match tokio::time::timeout(std::time::Duration::from_secs(60), rx).await {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(_)) => {
            // Channel closed — clean up
            let mut pending = ctx.pending.lock().map_err(|e| e.to_string())?;
            pending.remove(&request_id);
            let mut meta = ctx.pending_meta.lock().map_err(|e| e.to_string())?;
            meta.remove(&request_id);
            Err("Confirmation channel closed".to_string())
        }
        Err(_) => {
            // Timeout — clean up and default to reject
            let mut pending = ctx.pending.lock().map_err(|e| e.to_string())?;
            pending.remove(&request_id);
            let mut meta = ctx.pending_meta.lock().map_err(|e| e.to_string())?;
            meta.remove(&request_id);
            tracing::warn!("confirmation timed out for {} (60s), rejecting", tool_name);
            Ok(ConfirmationResult {
                approved: false,
                add_to_allowlist: false,
                add_to_denylist: false,
                comment: Some("Timed out waiting for confirmation (60s)".to_string()),
            })
        }
    }
}

/// Send a permission outcome message to the client (best-effort).
fn send_permission_outcome(ctx: &PermissionContext, tool_name: &str, granted: bool, reason: &str) {
    use codiv_common::messages::DaemonMessage;
    let msg = DaemonMessage::PermissionOutcome {
        tool_name: tool_name.to_string(),
        granted,
        reason: reason.to_string(),
    };
    if let Ok(frame) = codiv_common::messages::frame_message(&msg) {
        if ctx.client_tx.try_send(frame).is_err() {
            tracing::warn!("failed to send PermissionOutcome for {} (channel full or closed)", tool_name);
        }
    }
}

/// Simple UUID v4 generator (no external crate needed).
fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let nanos = now.as_nanos();
    let random: u64 = (nanos as u64) ^ (std::process::id() as u64).wrapping_mul(0x517cc1b727220a95);
    format!("{:016x}-{:016x}", nanos as u64, random)
}
