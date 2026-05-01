use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use serde_json::Value;
use codiv_common::messages::{DaemonMessage, RiskLevel};
use codiv_common::permissions::{PermissionDecision, PermissionMode};
use codiv_common::tools::tool_names;
use tokio::sync::{mpsc, oneshot};

use super::config::{AppConfig, ModelCatalog};
use super::permission_evaluator::evaluate_permission;
use super::risk_classifier::classify_risk;

type BoxedAsyncToolFn = Box<
    dyn Fn(Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
        + Send
        + Sync,
>;

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
    /// Shared config snapshot, updated by the daemon's file watcher.
    /// Avoids reloading config from disk on every tool call.
    pub config: Arc<RwLock<AppConfig>>,
}

/// Result of a user confirmation.
pub struct ConfirmationResult {
    pub approved: bool,
    #[allow(dead_code)]
    pub add_to_allowlist: bool,
    #[allow(dead_code)]
    pub add_to_denylist: bool,
    pub comment: Option<String>,
}

impl PermissionContext {
    pub fn new(
        mode: PermissionMode,
        client_tx: mpsc::Sender<Vec<u8>>,
        catalog: ModelCatalog,
        config: Arc<RwLock<AppConfig>>,
    ) -> Self {
        Self {
            mode: RwLock::new(mode),
            client_tx,
            pending: std::sync::Mutex::new(HashMap::new()),
            pending_meta: std::sync::Mutex::new(HashMap::new()),
            catalog,
            context_summary: RwLock::new(String::new()),
            config,
        }
    }

    pub fn current_mode(&self) -> PermissionMode {
        *self.mode.read().unwrap_or_else(|e| {
            tracing::warn!("PermissionContext mode RwLock poisoned, recovering");
            e.into_inner()
        })
    }

    pub fn set_mode(&self, mode: PermissionMode) {
        *self.mode.write().unwrap_or_else(|e| {
            tracing::warn!("PermissionContext mode RwLock poisoned, recovering");
            e.into_inner()
        }) = mode;
    }

    /// Update the conversation context summary for the LLM evaluator.
    pub fn set_context(&self, summary: String) {
        *self.context_summary.write().unwrap_or_else(|e| {
            tracing::warn!("PermissionContext context_summary RwLock poisoned, recovering");
            e.into_inner()
        }) = summary;
    }

    pub fn context_summary(&self) -> String {
        self.context_summary.read().unwrap_or_else(|e| {
            tracing::warn!("PermissionContext context_summary RwLock poisoned, recovering");
            e.into_inner()
        }).clone()
    }
}

/// Wrap a sync tool closure with permission checking.
/// Bridges to async via `block_in_place` + `Handle::block_on` for confirmation
/// and LLM evaluator calls, reusing the existing tokio runtime.
pub fn wrap_with_permissions(
    tool_name: String,
    original: Box<dyn Fn(Value) -> Result<String, String> + Send + Sync>,
    ctx: Arc<PermissionContext>,
) -> Box<dyn Fn(Value) -> Result<String, String> + Send + Sync> {
    Box::new(move |args: Value| {
        let command_prefix = super::permission_evaluator::extract_args_pattern(&tool_name, &args);

        // Check cached config permissions (shared with daemon's file watcher)
        {
            let cfg = ctx.config.read().unwrap_or_else(|e| {
                tracing::warn!("AppConfig RwLock poisoned in permission check, recovering");
                e.into_inner()
            });
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
                // Bridge async confirmation into sync closure using the existing runtime
                let result = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current()
                        .block_on(request_confirmation(&ctx, &tool_name, &args, risk))
                });
                handle_confirmation_result(result, &ctx, &tool_name, &args, &original)
            }
            PermissionDecision::LlmEvaluate => {
                // Run LLM evaluator on the existing runtime via block_in_place
                let llm_decision = tokio::task::block_in_place(|| {
                    let handle = tokio::runtime::Handle::current();
                    let eval_tool = tool_name.clone();
                    let eval_args = args.clone();
                    let eval_catalog = ctx.catalog.clone();
                    let eval_context = ctx.context_summary();
                    match handle.block_on(tokio::time::timeout(
                        std::time::Duration::from_secs(10),
                        tokio::spawn(async move {
                            super::llm_evaluator::llm_evaluate_risk(
                                &eval_tool,
                                &eval_args,
                                &eval_catalog,
                                &eval_context,
                            ).await
                        }),
                    )) {
                        Ok(Ok(decision)) => decision,
                        _ => {
                            tracing::warn!("LLM evaluator timed out after 10s, defaulting to Prompt");
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
                        // LLM said Prompt -- fall through to user confirmation
                        let result = tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current()
                                .block_on(request_confirmation(&ctx, &tool_name, &args, risk))
                        });
                        handle_confirmation_result(result, &ctx, &tool_name, &args, &original)
                    }
                }
            }
        }
    })
}

/// Shared helper for processing confirmation results (used by both sync and async paths).
fn handle_confirmation_result(
    result: Result<ConfirmationResult, String>,
    ctx: &PermissionContext,
    tool_name: &str,
    args: &Value,
    original: &dyn Fn(Value) -> Result<String, String>,
) -> Result<String, String> {
    match result {
        Ok(confirmation) if confirmation.approved => {
            send_permission_outcome(ctx, tool_name, true, "Allowed");
            original(args.clone())
        }
        Ok(confirmation) => {
            let is_timeout = confirmation.comment.as_deref()
                .map(|c| c.contains("Timed out"))
                .unwrap_or(false);
            let reason = if is_timeout { "Denied because of timeout" } else { "Denied" };
            send_permission_outcome(ctx, tool_name, false, reason);
            let msg = confirmation.comment.unwrap_or_else(|| "Action rejected by user".to_string());
            Err(msg)
        }
        Err(e) => {
            send_permission_outcome(ctx, tool_name, false, "Confirmation channel error");
            Err(format!("Confirmation failed: {}", e))
        }
    }
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
        tool_names::BASH => args.get("command").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        tool_names::WRITE => args.get("file_path").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        tool_names::EDIT => args.get("file_path").and_then(|v| v.as_str()).unwrap_or("").to_string(),
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

/// Wrap an async tool closure with permission checking.
/// This is the primary permission path — runs entirely on the existing
/// tokio runtime without creating nested runtimes or blocking threads.
pub fn wrap_with_permissions_async<F, Fut>(
    tool_name: String,
    original: F,
    ctx: Arc<PermissionContext>,
) -> BoxedAsyncToolFn
where
    F: Fn(Value) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<String, String>> + Send + 'static,
{
    let original = Arc::new(original);
    Box::new(move |args: Value| {
        let tool_name = tool_name.clone();
        let ctx = Arc::clone(&ctx);
        let original = Arc::clone(&original);
        Box::pin(async move {
            let command_prefix = super::permission_evaluator::extract_args_pattern(&tool_name, &args);

            // Check cached config permissions (shared with daemon's file watcher)
            let config_decision = {
                let cfg = ctx.config.read().unwrap_or_else(|e| {
                    tracing::warn!("AppConfig RwLock poisoned in async permission check, recovering");
                    e.into_inner()
                });
                cfg.permissions.lookup(&tool_name, command_prefix.as_deref()).map(|s| s.to_string())
            };
            if let Some(ref d) = config_decision {
                if d == "allow" { return original(args).await; }
                if d == "deny" { return Err("Denied by saved permission rule".into()); }
            }

            let risk = classify_risk(&tool_name, &args);
            let mode = ctx.current_mode();
            let decision = evaluate_permission(mode, &tool_name, &args, risk);

            match decision {
                PermissionDecision::Allow => original(args).await,
                PermissionDecision::Deny => Err("Denied by permission policy".into()),
                PermissionDecision::Prompt => {
                    let result = request_confirmation(&ctx, &tool_name, &args, risk).await;
                    handle_confirmation_result_async(result, &ctx, &tool_name, &args, original.as_ref()).await
                }
                PermissionDecision::LlmEvaluate => {
                    // Run LLM evaluator as a spawned task on the existing runtime
                    let eval_tool = tool_name.clone();
                    let eval_args = args.clone();
                    let eval_catalog = ctx.catalog.clone();
                    let eval_context = ctx.context_summary();
                    let llm_decision = match tokio::time::timeout(
                        std::time::Duration::from_secs(10),
                        tokio::spawn(async move {
                            super::llm_evaluator::llm_evaluate_risk(
                                &eval_tool,
                                &eval_args,
                                &eval_catalog,
                                &eval_context,
                            ).await
                        }),
                    ).await {
                        Ok(Ok(decision)) => decision,
                        _ => {
                            tracing::warn!("LLM evaluator timed out after 10s, defaulting to Prompt");
                            PermissionDecision::Prompt
                        }
                    };
                    match llm_decision {
                        PermissionDecision::Allow => {
                            send_permission_outcome(&ctx, &tool_name, true, "Allowed by AI evaluator");
                            original(args).await
                        }
                        _ => {
                            let result = request_confirmation(&ctx, &tool_name, &args, risk).await;
                            handle_confirmation_result_async(result, &ctx, &tool_name, &args, original.as_ref()).await
                        }
                    }
                }
            }
        })
    })
}

/// Async helper for processing confirmation results.
async fn handle_confirmation_result_async<F, Fut>(
    result: Result<ConfirmationResult, String>,
    ctx: &PermissionContext,
    tool_name: &str,
    args: &Value,
    original: &F,
) -> Result<String, String>
where
    F: Fn(Value) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    match result {
        Ok(confirmation) if confirmation.approved => {
            send_permission_outcome(ctx, tool_name, true, "Allowed");
            original(args.clone()).await
        }
        Ok(confirmation) => {
            let is_timeout = confirmation.comment.as_deref()
                .map(|c| c.contains("Timed out"))
                .unwrap_or(false);
            let reason = if is_timeout { "Denied because of timeout" } else { "Denied" };
            send_permission_outcome(ctx, tool_name, false, reason);
            let msg = confirmation.comment.unwrap_or_else(|| "Action rejected by user".to_string());
            Err(msg)
        }
        Err(e) => {
            send_permission_outcome(ctx, tool_name, false, "Confirmation channel error");
            Err(format!("Confirmation failed: {}", e))
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

/// Generate a random UUID v4 string.
fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}
