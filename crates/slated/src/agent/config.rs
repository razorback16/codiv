use aisdk::core::tools::Tool;
use aisdk::core::{LanguageModel, LanguageModelRequest, LanguageModelStreamChunkType};
use aisdk::providers::{Anthropic, Google, OpenAI};
use futures::StreamExt;
use serde::Deserialize;
use slate_common::messages::{frame_message, DaemonMessage, StreamChunk};
use slate_common::types::AgentRole;
use std::collections::HashMap;
use tokio::sync::mpsc;

type DynError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Deserialize)]
pub struct ModelCatalog {
    pub default_provider: String,
    pub roles: HashMap<String, ModelAssignment>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelAssignment {
    pub provider: String,
    pub model: String,
    #[allow(dead_code)] // reserved for future aisdk support
    pub max_tokens: Option<u32>,
    pub temperature: Option<u8>,
}

impl ModelCatalog {
    pub fn load() -> Self {
        let config_path = slate_common::config::config_dir().join("models.toml");
        if let Ok(contents) = std::fs::read_to_string(&config_path) {
            if let Ok(catalog) = toml::from_str(&contents) {
                return catalog;
            }
        }

        // Default: Anthropic Claude for everything
        Self {
            default_provider: "anthropic".to_string(),
            roles: HashMap::new(),
        }
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
            model: "claude-sonnet-4-5".to_string(),
            max_tokens: Some(8192),
            temperature: None,
        })
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

/// Execute a streaming LLM call, forwarding chunks over IPC.
/// Returns (response_text, input_tokens, output_tokens).
pub async fn stream_from_config(
    assignment: &ModelAssignment,
    messages: aisdk::core::messages::Messages,
    request_id: &str,
    tx: &mpsc::Sender<Vec<u8>>,
    tools: Vec<Tool>,
) -> Result<(String, usize, usize), DynError> {
    match assignment.provider.as_str() {
        "anthropic" => {
            run_stream(Anthropic::model_name(&assignment.model), messages, request_id, tx, assignment, tools).await
        }
        "openai" => {
            run_stream(OpenAI::model_name(&assignment.model), messages, request_id, tx, assignment, tools).await
        }
        "google" => {
            run_stream(Google::model_name(&assignment.model), messages, request_id, tx, assignment, tools).await
        }
        other => Err(format!("unsupported provider: {}", other).into()),
    }
}

async fn run_stream<M>(
    model: M,
    messages: aisdk::core::messages::Messages,
    request_id: &str,
    tx: &mpsc::Sender<Vec<u8>>,
    config: &ModelAssignment,
    tools: Vec<Tool>,
) -> Result<(String, usize, usize), DynError>
where
    M: LanguageModel + aisdk::core::capabilities::TextInputSupport + aisdk::core::capabilities::ToolCallSupport + Send + Sync + 'static,
{
    let mut builder = LanguageModelRequest::builder()
        .model(model)
        .messages(messages);

    if let Some(temp) = config.temperature {
        builder = builder.temperature(temp as u32);
    }

    for tool in tools {
        builder = builder.with_tool(tool);
    }
    // Note: aisdk 0.5.x does not expose a max_output_tokens builder method;
    // the provider's default is used. config.max_tokens is reserved for future use.

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
                tracing::error!("stream failed: {err}");
                return Err(err.into());
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
