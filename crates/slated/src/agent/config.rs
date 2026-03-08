use aisdk::core::tools::Tool;
use aisdk::core::{LanguageModel, LanguageModelRequest, LanguageModelStreamChunkType};
use aisdk::providers::{Anthropic, Google, OpenAI};
use futures::StreamExt;
use serde::Deserialize;
use slate_common::messages::{frame_message, DaemonMessage, StreamChunk};
use slate_common::truncate::{truncate_tool_output, MAX_TOOL_OUTPUT_BYTES};
use slate_common::types::AgentRole;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;

use super::error::classify_error;

type DynError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Clone, Default, Deserialize)]
pub struct ProviderConfig {
    pub api_key: Option<String>,
    pub base_url: Option<String>,
}

impl std::fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("base_url", &self.base_url)
            .finish()
    }
}

#[derive(Debug, Deserialize)]
pub struct ModelCatalog {
    pub default_provider: String,
    pub default_model: String,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(default)]
    pub roles: HashMap<String, ModelAssignment>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelAssignment {
    pub provider: String,
    pub model: String,
    pub temperature: Option<u8>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
}

const DEFAULT_MODELS_TOML: &str = r#"# Slate Agent — Model Configuration
#
# This file controls which LLM provider and model is used.
# Edit the values below to switch providers or models.

# The provider used when a role doesn't specify one.
# Supported: "anthropic", "openai", "google"
default_provider = "anthropic"

# The model used when a role doesn't specify one.
default_model = "claude-sonnet-4-5"

# ── Provider-level settings ──────────────────────────────────────────
# API keys and base URLs set here apply to all roles using that provider.
# Per-role settings (below) take precedence over these.
#
# [providers.anthropic]
# api_key = "sk-ant-..."          # or set ANTHROPIC_API_KEY env var
# base_url = "https://custom-endpoint.example.com"  # /v1 is auto-appended if missing
#
# [providers.openai]
# api_key = "sk-..."              # or set OPENAI_API_KEY env var
#
# [providers.google]
# api_key = "..."                 # or set GOOGLE_API_KEY env var

# ── Per-role overrides ───────────────────────────────────────────────
# Assign a specific provider/model to each agent role.
# Available roles: engineer, team_lead, reviewer, researcher, security
#
# [roles.engineer]
# provider = "anthropic"
# model = "claude-sonnet-4-5"
# max_tokens = 8192
# temperature = 0
# api_key = "sk-ant-..."          # overrides provider-level key
# base_url = "https://..."        # overrides provider-level url; /v1 auto-appended if missing
#
# [roles.reviewer]
# provider = "openai"
# model = "gpt-4o"
"#;

impl ModelCatalog {
    pub fn load() -> Self {
        let config_dir = slate_common::config::config_dir();
        let config_path = config_dir.join("models.toml");

        if let Ok(contents) = std::fs::read_to_string(&config_path) {
            if let Ok(catalog) = toml::from_str(&contents) {
                return catalog;
            }
        }

        // Create the default config file so users can discover and edit it.
        let _ = std::fs::create_dir_all(&config_dir);
        let _ = std::fs::write(&config_path, DEFAULT_MODELS_TOML);

        toml::from_str(DEFAULT_MODELS_TOML).expect("default models.toml must parse")
    }

    pub fn assignment_for(&self, role: &AgentRole) -> ModelAssignment {
        let key = match role {
            AgentRole::TeamLead => "team_lead",
            AgentRole::Engineer => "engineer",
            AgentRole::Reviewer => "reviewer",
            AgentRole::Researcher => "researcher",
            AgentRole::Security => "security",
        };

        self.roles.get(key).cloned().unwrap_or(ModelAssignment {
            provider: self.default_provider.clone(),
            model: self.default_model.clone(),
            temperature: None,
            api_key: None,
            base_url: None,
        })
    }

    pub fn resolve_provider_config(&self, assignment: &ModelAssignment) -> ProviderConfig {
        let global = self.providers.get(&assignment.provider);
        ProviderConfig {
            api_key: assignment.api_key.clone()
                .or_else(|| global.and_then(|g| g.api_key.clone())),
            base_url: assignment.base_url.clone()
                .or_else(|| global.and_then(|g| g.base_url.clone())),
        }
    }
}

impl ModelAssignment {
    /// Map a full model identifier to a short display alias.
    pub fn model_alias(&self) -> String {
        match self.model.as_str() {
            "claude-sonnet-4-5" => "sonnet-4.5".to_string(),
            "claude-sonnet-4-5-20250514" => "sonnet-4.5".to_string(),
            "claude-opus-4" => "opus-4".to_string(),
            "claude-haiku-3-5" => "haiku-3.5".to_string(),
            "gpt-4o" => "gpt-4o".to_string(),
            "gpt-4o-mini" => "gpt-4o-mini".to_string(),
            "gemini-2.0-flash" => "gemini-flash".to_string(),
            "gemini-2.5-pro" => "gemini-pro".to_string(),
            other => {
                let stripped = other.strip_prefix("claude-").unwrap_or(other);
                if stripped.len() > 15 {
                    stripped[..15].to_string()
                } else {
                    stripped.to_string()
                }
            }
        }
    }

    /// Return the context window size (max tokens) for this model.
    pub fn context_window(&self) -> usize {
        match self.model.as_str() {
            "claude-sonnet-4-5" | "claude-sonnet-4-5-20250514" => 200_000,
            "claude-opus-4" => 200_000,
            "claude-haiku-3-5" => 200_000,
            "gpt-4o" => 128_000,
            "gpt-4o-mini" => 128_000,
            "gemini-2.0-flash" => 1_048_576,
            "gemini-2.5-pro" => 1_048_576,
            _ => 200_000, // conservative default
        }
    }
}

/// Ensure a base URL ends with `/v1` (or `/v1/`).
/// If the user already supplied it, keep it; otherwise append it.
fn ensure_v1_suffix(url: &str) -> String {
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
fn sanitize_tool_schema_for_openai(mut tool: Tool) -> Tool {
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

fn build_anthropic_model(
    model_name: &str,
    provider_config: &ProviderConfig,
) -> Result<aisdk::providers::Anthropic<aisdk::core::DynamicModel>, DynError> {
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
        Ok(builder.build()?)
    } else {
        Ok(Anthropic::model_name(model_name))
    }
}

fn build_openai_model(
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

fn build_google_model(
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

const MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF_MS: u64 = 1000;

/// Execute a streaming LLM call, forwarding chunks over IPC.
/// Returns (response_text, input_tokens, output_tokens).
///
/// Retries transient errors (server errors, rate limits) up to `MAX_RETRIES`
/// times with exponential backoff. Non-retryable errors fail immediately with
/// a user-friendly message.
pub async fn stream_from_config(
    assignment: &ModelAssignment,
    provider_config: &ProviderConfig,
    messages: aisdk::core::messages::Messages,
    request_id: &str,
    tx: &mpsc::Sender<Vec<u8>>,
    tools: Vec<Tool>,
    thinking: bool,
) -> Result<(String, usize, usize), DynError> {
    let mut attempt = 0u32;
    loop {
        let result = match assignment.provider.as_str() {
            "anthropic" => {
                let model = build_anthropic_model(&assignment.model, provider_config)?;
                run_stream(model, messages.clone(), request_id, tx, assignment, tools.clone(), thinking).await
            }
            "openai" => {
                let model = build_openai_model(&assignment.model, provider_config)?;
                let tools = tools.iter().cloned().map(sanitize_tool_schema_for_openai).collect();
                run_stream(model, messages.clone(), request_id, tx, assignment, tools, thinking).await
            }
            "google" => {
                let model = build_google_model(&assignment.model, provider_config)?;
                run_stream(model, messages.clone(), request_id, tx, assignment, tools.clone(), thinking).await
            }
            other => return Err(format!("unsupported provider: {}", other).into()),
        };

        match result {
            Ok(v) => return Ok(v),
            Err(e) => {
                let kind = classify_error(&e.to_string());
                if kind.is_retryable() && attempt < MAX_RETRIES {
                    attempt += 1;
                    let backoff = INITIAL_BACKOFF_MS * 2u64.pow(attempt - 1);
                    tracing::warn!(
                        "retryable error on attempt {}/{}: {}. Retrying in {}ms",
                        attempt, MAX_RETRIES, e, backoff
                    );
                    let _ = send_ipc(
                        tx,
                        &DaemonMessage::AgentStreamChunk {
                            request_id: request_id.to_string(),
                            chunk: StreamChunk::Text(format!(
                                "\n[Retrying request (attempt {}/{}): {}]\n",
                                attempt + 1,
                                MAX_RETRIES + 1,
                                kind.user_message(&e.to_string())
                            )),
                        },
                    )
                    .await;
                    tokio::time::sleep(Duration::from_millis(backoff)).await;
                    continue;
                }
                return Err(kind.user_message(&e.to_string()).into());
            }
        }
    }
}

async fn run_stream<M>(
    model: M,
    messages: aisdk::core::messages::Messages,
    request_id: &str,
    tx: &mpsc::Sender<Vec<u8>>,
    config: &ModelAssignment,
    tools: Vec<Tool>,
    thinking: bool,
) -> Result<(String, usize, usize), DynError>
where
    M: LanguageModel + aisdk::core::capabilities::TextInputSupport + aisdk::core::capabilities::ToolCallSupport + aisdk::core::capabilities::ReasoningSupport + Send + Sync + 'static,
{
    let mut builder = LanguageModelRequest::builder()
        .model(model)
        .messages(messages);

    if thinking {
        // Anthropic API does not allow setting temperature with extended thinking.
        builder = builder.reasoning_effort(aisdk::core::language_model::ReasoningEffort::Medium);
    } else if let Some(temp) = config.temperature {
        builder = builder.temperature(temp as u32);
    }

    for tool in tools {
        builder = builder.with_tool(tool);
    }
    let mut response = builder.build().stream_text().await?;
    let mut full_text = String::new();
    let mut chunk_count: u32 = 0;

    tracing::debug!("stream started for request {}", request_id);

    while let Some(chunk) = response.stream.next().await {
        chunk_count += 1;

        match chunk {
            LanguageModelStreamChunkType::Text(text) => {
                full_text.push_str(&text);
                send_ipc(
                    tx,
                    &DaemonMessage::AgentStreamChunk {
                        request_id: request_id.to_string(),
                        chunk: StreamChunk::Text(text),
                    },
                )
                .await?;
            }
            LanguageModelStreamChunkType::Reasoning(text) => {
                send_ipc(
                    tx,
                    &DaemonMessage::AgentStreamChunk {
                        request_id: request_id.to_string(),
                        chunk: StreamChunk::Reasoning(text),
                    },
                )
                .await?;
            }
            LanguageModelStreamChunkType::ToolCallDelta { tool_call_id, tool_name, delta } => {
                send_ipc(
                    tx,
                    &DaemonMessage::AgentStreamChunk {
                        request_id: request_id.to_string(),
                        chunk: StreamChunk::ToolCallDelta { tool_call_id, tool_name, delta },
                    },
                )
                .await?;
            }
            LanguageModelStreamChunkType::ToolCallStart(info) => {
                let args = serde_json::to_string(&info.input).unwrap_or_default();
                send_ipc(
                    tx,
                    &DaemonMessage::AgentStreamChunk {
                        request_id: request_id.to_string(),
                        chunk: StreamChunk::ToolCall {
                            name: info.tool.name,
                            arguments: args,
                        },
                    },
                )
                .await?;
            }
            LanguageModelStreamChunkType::ToolResult(info) => {
                let output = match &info.output {
                    Ok(v) => match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => serde_json::to_string(other).unwrap_or_default(),
                    },
                    Err(e) => format!("Error: {e}"),
                };
                let output = truncate_tool_output(&output, MAX_TOOL_OUTPUT_BYTES);
                send_ipc(
                    tx,
                    &DaemonMessage::AgentStreamChunk {
                        request_id: request_id.to_string(),
                        chunk: StreamChunk::ToolResult {
                            name: info.tool.name,
                            result: output,
                        },
                    },
                )
                .await?;
            }
            LanguageModelStreamChunkType::Failed(err) => {
                let kind = classify_error(&err.to_string());
                tracing::error!("stream failed: {err}");
                return Err(kind.user_message(&err.to_string()).into());
            }
            LanguageModelStreamChunkType::Incomplete(reason) => {
                tracing::warn!("stream incomplete: {reason}");
            }
            _ => {}
        }
    }

    tracing::info!(
        "stream ended for request {}: {} chunks, {} bytes of text",
        request_id, chunk_count, full_text.len()
    );

    if let Some(reason) = response.stop_reason().await {
        tracing::debug!("stop_reason: {:?}", reason);
    }

    let usage = response.usage().await;
    let input_tokens = usage.input_tokens.unwrap_or(0);
    let output_tokens = usage.output_tokens.unwrap_or(0);

    Ok((full_text, input_tokens, output_tokens))
}

async fn send_ipc(tx: &mpsc::Sender<Vec<u8>>, msg: &DaemonMessage) -> Result<(), DynError> {
    let frame = frame_message(msg)?;
    tx.send(frame).await.map_err(|_| "ipc receiver closed")?;
    Ok(())
}
