use slate_common::permissions::PermissionDecision;
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

    // NOTE: This function may be called via block_in_place + block_on from a
    // sync context. tokio::time::timeout can fail to fire in that situation
    // because the timer driver doesn't advance while block_on is blocking.
    // We call the LLM directly without an async timeout here; the caller
    // (permissions.rs) applies its own thread-level timeout instead.
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
    use aisdk::core::LanguageModelRequest;
    use aisdk::core::messages::Message;
    use futures::StreamExt;

    let messages = Message::builder()
        .system("You are a security evaluator. Respond only with ALLOW or DENY.")
        .user(prompt)
        .build();

    // Build model based on provider
    match assignment.provider.as_str() {
        "anthropic" => {
            let model = super::config::build_anthropic_model_pub(&assignment.model, provider_config)?;
            let mut response = LanguageModelRequest::builder()
                .model(model)
                .messages(messages)
                .build()
                .stream_text()
                .await?;
            let mut text = String::new();
            while let Some(chunk) = response.stream.next().await {
                if let aisdk::core::LanguageModelStreamChunkType::Text(t) = chunk {
                    text.push_str(&t);
                }
            }
            Ok(text)
        }
        "openai" => {
            let model = super::config::build_openai_model_pub(&assignment.model, provider_config)?;
            let mut response = LanguageModelRequest::builder()
                .model(model)
                .messages(messages)
                .build()
                .stream_text()
                .await?;
            let mut text = String::new();
            while let Some(chunk) = response.stream.next().await {
                if let aisdk::core::LanguageModelStreamChunkType::Text(t) = chunk {
                    text.push_str(&t);
                }
            }
            Ok(text)
        }
        "google" => {
            let model = super::config::build_google_model_pub(&assignment.model, provider_config)?;
            let mut response = LanguageModelRequest::builder()
                .model(model)
                .messages(messages)
                .build()
                .stream_text()
                .await?;
            let mut text = String::new();
            while let Some(chunk) = response.stream.next().await {
                if let aisdk::core::LanguageModelStreamChunkType::Text(t) = chunk {
                    text.push_str(&t);
                }
            }
            Ok(text)
        }
        other => Err(format!("unsupported provider: {}", other).into()),
    }
}
