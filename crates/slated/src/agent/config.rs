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

/// Execute a streaming LLM call, forwarding chunks over IPC.
/// Returns the accumulated response text.
pub async fn stream_from_config(
    assignment: &ModelAssignment,
    messages: aisdk::core::messages::Messages,
    request_id: &str,
    tx: &mpsc::Sender<Vec<u8>>,
) -> Result<String, DynError> {
    match assignment.provider.as_str() {
        "anthropic" => {
            run_stream(Anthropic::model_name(&assignment.model), messages, request_id, tx, assignment).await
        }
        "openai" => {
            run_stream(OpenAI::model_name(&assignment.model), messages, request_id, tx, assignment).await
        }
        "google" => {
            run_stream(Google::model_name(&assignment.model), messages, request_id, tx, assignment).await
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
) -> Result<String, DynError>
where
    M: LanguageModel + aisdk::core::capabilities::TextInputSupport + Send + Sync + 'static,
{
    let mut builder = LanguageModelRequest::builder()
        .model(model)
        .messages(messages);

    if let Some(temp) = config.temperature {
        builder = builder.temperature(temp as u32);
    }
    // Note: aisdk 0.5.x does not expose a max_output_tokens builder method;
    // the provider's default is used. config.max_tokens is reserved for future use.

    let mut response = builder.build().stream_text().await?;
    let mut full_text = String::new();

    while let Some(chunk) = response.stream.next().await {
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
            LanguageModelStreamChunkType::Failed(err) => {
                return Err(err.into());
            }
            _ => {}
        }
    }
    Ok(full_text)
}

async fn send_ipc(tx: &mpsc::Sender<Vec<u8>>, msg: &DaemonMessage) -> Result<(), DynError> {
    let frame = frame_message(msg)?;
    tx.send(frame).await.map_err(|_| "ipc receiver closed")?;
    Ok(())
}
