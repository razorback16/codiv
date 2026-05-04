use aisdk::core::tools::Tool;
use aisdk::providers::{Anthropic, Google, OpenAI, OpenAICompatible, Vllm};

use super::models::{DynError, ProviderConfig};

/// Ensure a base URL ends with `/v1` (or `/v1/`).
/// If the user already supplied it, keep it; otherwise append it.
pub(super) fn ensure_v1_suffix(url: &str) -> String {
    let trimmed = url.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        format!("{}/", trimmed)
    } else {
        format!("{}/v1/", trimmed)
    }
}

/// Strip a trailing `/v1` (or `/v1/`) from a base URL.
/// OpenAI SDK already includes `/v1` in the request path, so the base URL
/// must NOT contain it.
fn strip_v1_suffix(url: &str) -> String {
    let trimmed = url.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        trimmed.trim_end_matches("/v1").to_string()
    } else {
        trimmed.to_string()
    }
}

/// Sanitize a tool's JSON Schema for OpenAI strict mode compatibility.
/// OpenAI strict mode requires:
/// - All properties listed in `required`
/// - No unsupported keywords like `format`, `minimum`, `maximum`, `$schema`, `title`, `default`
pub(super) fn sanitize_tool_schema_for_openai(mut tool: Tool) -> Tool {
    let mut val = tool.input_schema.to_value();
    sanitize_schema_value(&mut val);
    tool.input_schema = schemars::Schema::try_from(val).unwrap();
    tool
}

fn sanitize_schema_value(val: &mut serde_json::Value) {
    if let Some(obj) = val.as_object_mut() {
        // Remove unsupported top-level keywords
        obj.remove("$schema");
        obj.remove("title");

        // If there are properties, ensure all are in required
        if let Some(props) = obj.get("properties").cloned() {
            if let Some(prop_obj) = props.as_object() {
                let all_keys: Vec<serde_json::Value> = prop_obj
                    .keys()
                    .map(|k| serde_json::Value::String(k.clone()))
                    .collect();
                obj.insert("required".to_string(), serde_json::Value::Array(all_keys));
            }
        }

        // Clean each property's sub-schema
        if let Some(props) = obj.get_mut("properties") {
            if let Some(prop_obj) = props.as_object_mut() {
                for (_key, prop_schema) in prop_obj.iter_mut() {
                    if let Some(prop) = prop_schema.as_object_mut() {
                        prop.remove("format");
                        prop.remove("minimum");
                        prop.remove("maximum");
                        prop.remove("default");
                    }
                }
            }
        }
    }
}

pub(super) fn build_anthropic_model(
    model_name: &str,
    provider_config: &ProviderConfig,
) -> Result<aisdk::providers::Anthropic<aisdk::core::DynamicModel>, DynError> {
    let ua_headers = std::collections::HashMap::from([
        ("user-agent".to_string(), "Claude Code".to_string()),
    ]);
    if provider_config.api_key.is_some() || provider_config.base_url.is_some() {
        let mut builder = Anthropic::builder().model_name(model_name);
        if let Some(ref key) = provider_config.api_key {
            builder = builder.api_key(key);
        } else {
            // Builder requires api_key; fall back to env var
            let key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
            builder = builder.api_key(key);
        }
        if let Some(ref url) = provider_config.base_url {
            builder = builder.base_url(ensure_v1_suffix(url));
        }
        builder = builder.headers(ua_headers);
        Ok(builder.build()?)
    } else {
        let key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
        Ok(Anthropic::builder()
            .model_name(model_name)
            .api_key(key)
            .headers(ua_headers)
            .build()?)
    }
}

/// Build an Anthropic model for Claude Code OAuth tokens.
///
/// Uses aisdk's `.use_oauth(true)` which sends `Authorization: Bearer {token}`
/// instead of `x-api-key`, and adds the required `anthropic-beta` flags.
pub(super) fn build_claude_code_model(
    model_name: &str,
    provider_config: &ProviderConfig,
) -> Result<aisdk::providers::Anthropic<aisdk::core::DynamicModel>, DynError> {
    let api_key = provider_config
        .api_key
        .clone()
        .ok_or("claude_code provider requires an api_key (OAuth access token) in config")?;

    let headers = std::collections::HashMap::from([
        ("user-agent".to_string(), "Claude Code".to_string()),
        ("x-app".to_string(), "cli".to_string()),
        ("x-claude-code-session-id".to_string(), uuid::Uuid::new_v4().to_string()),
    ]);

    // Build metadata matching Claude Code's format.
    // The account_uuid is critical for the API to apply correct rate limits.
    let account_uuid = codiv_common::auth::read_oauth_account_uuid("claude_code").unwrap_or_default();
    let metadata = serde_json::json!({
        "metadata": {
            "user_id": serde_json::json!({
                "account_uuid": account_uuid,
            }).to_string()
        }
    });

    let model = Anthropic::builder()
        .model_name(model_name)
        .api_key(api_key)
        .use_oauth(true)
        .headers(headers)
        .body(metadata)
        .build()?;
    Ok(model)
}

pub(super) fn build_openai_model(
    model_name: &str,
    provider_config: &ProviderConfig,
) -> Result<aisdk::providers::OpenAI<aisdk::core::DynamicModel>, DynError> {
    if provider_config.api_key.is_some() || provider_config.base_url.is_some() {
        let mut builder = OpenAI::builder().model_name(model_name);
        if let Some(ref key) = provider_config.api_key {
            builder = builder.api_key(key);
        } else {
            let key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
            builder = builder.api_key(key);
        }
        if let Some(ref url) = provider_config.base_url {
            builder = builder.base_url(strip_v1_suffix(url));
        }
        Ok(builder.build()?)
    } else {
        Ok(OpenAI::model_name(model_name))
    }
}

/// Build an OpenAI model for the Codex OAuth provider.
///
/// Uses `https://chatgpt.com/backend-api/codex` as base URL with path `/responses`.
/// The aisdk detects this as a "chatgpt codex" request and automatically bridges
/// the request format (moves system messages to `instructions`, sets `store: false`).
pub(super) fn build_codex_model(
    model_name: &str,
    provider_config: &ProviderConfig,
) -> Result<aisdk::providers::OpenAI<aisdk::core::DynamicModel>, DynError> {
    let api_key = provider_config
        .api_key
        .clone()
        .ok_or("codex provider requires an api_key (OAuth access token) in config")?;

    let headers = std::collections::HashMap::from([
        ("originator".to_string(), "codex-tui".to_string()),
    ]);

    let model = OpenAI::builder()
        .model_name(model_name)
        .api_key(api_key)
        .base_url("https://chatgpt.com/backend-api/codex")
        .path("/responses")
        .headers(headers)
        .build()?;
    Ok(model)
}

pub(super) fn build_google_model(
    model_name: &str,
    provider_config: &ProviderConfig,
) -> Result<aisdk::providers::Google<aisdk::core::DynamicModel>, DynError> {
    if provider_config.api_key.is_some() || provider_config.base_url.is_some() {
        let mut builder = Google::builder().model_name(model_name);
        if let Some(ref key) = provider_config.api_key {
            builder = builder.api_key(key);
        } else {
            let key = std::env::var("GOOGLE_API_KEY").unwrap_or_default();
            builder = builder.api_key(key);
        }
        if let Some(ref url) = provider_config.base_url {
            builder = builder.base_url(url);
        }
        Ok(builder.build()?)
    } else {
        Ok(Google::model_name(model_name))
    }
}

pub(super) fn build_vllm_model(
    model_name: &str,
    provider_config: &ProviderConfig,
) -> Result<aisdk::providers::Vllm<aisdk::core::DynamicModel>, DynError> {
    let base_url = provider_config.base_url.as_deref()
        .ok_or("vllm provider requires a base_url in config")?;
    let mut builder = Vllm::builder()
        .model_name(model_name)
        .base_url(base_url);
    if let Some(ref key) = provider_config.api_key {
        builder = builder.api_key(key);
    }
    Ok(builder.build()?)
}

pub(super) fn build_openai_compatible_model(
    model_name: &str,
    provider_name: &str,
    provider_config: &ProviderConfig,
) -> Result<aisdk::providers::OpenAICompatible<aisdk::core::DynamicModel>, DynError> {
    let base_url = provider_config.base_url.as_deref()
        .ok_or_else(|| format!("provider '{}' requires a base_url in config", provider_name))?;
    let api_key = provider_config.api_key.clone()
        .ok_or_else(|| format!("provider '{}' requires an api_key in config", provider_name))?;

    let model = OpenAICompatible::<aisdk::core::DynamicModel>::builder()
        .provider_name(provider_name)
        .base_url(base_url)
        .api_key(api_key)
        .model_name(model_name)
        .build()?;
    Ok(model)
}
