use codiv_common::permissions::PermissionDecision;
use super::config::{ModelCatalog, ProviderConfig, ModelAssignment};

/// Use a lightweight LLM call to evaluate whether a High-risk tool call should be allowed.
/// Returns Allow if the LLM says it's safe, Prompt otherwise.
/// Falls back to Prompt on any failure or timeout.
pub async fn llm_evaluate_risk(
    tool_name: &str,
    args: &serde_json::Value,
    catalog: &ModelCatalog,
    context_summary: &str,
) -> PermissionDecision {
    let assignment = match catalog.roles.get("permission_evaluator") {
        Some(a) => a.clone(),
        None => {
            tracing::warn!("no permission_evaluator role configured, defaulting to Prompt");
            return PermissionDecision::Prompt;
        }
    };

    let provider_config = catalog.resolve_provider_config(&assignment);

    let context_block = if context_summary.is_empty() {
        String::new()
    } else {
        format!("\nConversation context:\n{}\n", context_summary)
    };

    let prompt = format!(
        "A coding assistant is about to execute a tool call. \
        Based on the conversation context and the tool call, evaluate if it is safe to proceed automatically.\n\
        {context_block}\n\
        Tool: {tool_name}\n\
        Arguments: {args}\n\n\
        Consider:\n\
        - Does this action align with what the user asked for?\n\
        - Is it destructive or irreversible?\n\
        - Could it cause data loss or affect systems outside the local project?\n\
        - Is the scope reasonable (not overly broad)?\n\n\
        Respond with EXACTLY one word: ALLOW or DENY",
        args = serde_json::to_string_pretty(args).unwrap_or_default()
    );

    // The caller (permissions.rs) applies a 10-second timeout via tokio::time::timeout
    // around the spawned task, so no timeout is needed here.
    match call_evaluator_llm(&assignment, &provider_config, &prompt).await {
        Ok(response) => {
            let response = response.trim().to_uppercase();
            if response.contains("ALLOW") {
                tracing::info!("LLM evaluator allowed tool call: {} {:?}", tool_name, args);
                PermissionDecision::Allow
            } else {
                tracing::info!("LLM evaluator denied tool call: {} {:?}", tool_name, args);
                PermissionDecision::Prompt
            }
        }
        Err(e) => {
            tracing::warn!("LLM evaluator error: {}, defaulting to Prompt", e);
            PermissionDecision::Prompt
        }
    }
}

async fn call_evaluator_llm(
    assignment: &ModelAssignment,
    provider_config: &ProviderConfig,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    super::config::simple_text_completion(
        assignment,
        provider_config,
        "You are a security evaluator. Respond only with ALLOW or DENY.",
        prompt,
    ).await
}
