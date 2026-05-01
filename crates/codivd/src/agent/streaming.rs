use aisdk::core::tools::Tool;
use aisdk::core::{LanguageModel, LanguageModelRequest, LanguageModelStreamChunkType};
use futures::StreamExt;
use codiv_common::conversation::ConversationEvent;
use codiv_common::messages::{frame_message, DaemonMessage, StreamChunk};
use codiv_common::truncate::{truncate_tool_output, MAX_TOOL_OUTPUT_BYTES};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use super::error::classify_error;
use super::models::{DynError, ModelAssignment, ProviderConfig};
use super::providers::*;

macro_rules! with_provider_model {
    ($assignment:expr, $provider_config:expr, |$model:ident, $is_openai_compat:ident| $body:expr) => {
        match $assignment.provider.as_str() {
            "anthropic" => {
                let $model = build_anthropic_model(&$assignment.model, $provider_config)?;
                let $is_openai_compat = false;
                $body
            }
            "claude_code" => {
                let $model = build_claude_code_model(&$assignment.model, $provider_config)?;
                let $is_openai_compat = false;
                $body
            }
            "openai" => {
                let $model = build_openai_model(&$assignment.model, $provider_config)?;
                let $is_openai_compat = true;
                $body
            }
            "google" => {
                let $model = build_google_model(&$assignment.model, $provider_config)?;
                let $is_openai_compat = false;
                $body
            }
            "vllm" => {
                let $model = build_vllm_model(&$assignment.model, $provider_config)?;
                let $is_openai_compat = false;
                $body
            }
            other => {
                let $model = build_openai_compatible_model(&$assignment.model, other, $provider_config)?;
                let $is_openai_compat = true;
                $body
            }
        }
    };
}

/// Perform a simple text completion: build model from assignment, stream text,
/// and collect into a String. No tools, no retries, no IPC.
///
/// Uses streaming rather than generate_text to avoid blocking the inference
/// server's request slot (important for local models with single-request
/// concurrency like vllm).
pub async fn simple_text_completion(
    assignment: &ModelAssignment,
    provider_config: &ProviderConfig,
    system: &str,
    prompt: &str,
) -> Result<String, DynError> {
    use aisdk::core::messages::Message;
    // Pass system via .system() and user content via .messages() to avoid
    // aisdk's resolve_message duplicating the system message when both
    // .system() and .prompt() are used together.
    let messages = vec![Message::User(prompt.to_string().into()).into()];
    with_provider_model!(assignment, provider_config, |model, _is_openai_compat| {
        let mut response = LanguageModelRequest::builder()
            .model(model)
            .system(system)
            .messages(messages)
            .reasoning_effort(aisdk::core::language_model::ReasoningEffort::None)
            .build()
            .stream_text()
            .await?;
        let mut text = String::new();
        while let Some(chunk) = response.stream.next().await {
            if let LanguageModelStreamChunkType::TextDelta(t) = chunk {
                text.push_str(&t);
            }
        }
        Ok(text)
    })
}

/// Like `simple_text_completion` but streams each text chunk to the client
/// via IPC as `AgentStreamChunk::Text`. Returns (full_text, input_tokens,
/// output_tokens, cache_read_tokens).
pub async fn streaming_text_completion(
    assignment: &ModelAssignment,
    provider_config: &ProviderConfig,
    system: &str,
    prompt: &str,
    request_id: &str,
    tx: &mpsc::Sender<Vec<u8>>,
) -> Result<(String, usize, usize, usize), DynError> {
    use aisdk::core::messages::Message;
    let messages = vec![Message::User(prompt.to_string().into()).into()];
    with_provider_model!(assignment, provider_config, |model, _is_openai_compat| {
        let mut response = LanguageModelRequest::builder()
            .model(model)
            .system(system)
            .messages(messages)
            .reasoning_effort(aisdk::core::language_model::ReasoningEffort::None)
            .build()
            .stream_text()
            .await?;
        let mut text = String::new();
        while let Some(chunk) = response.stream.next().await {
            if let LanguageModelStreamChunkType::TextDelta(t) = chunk {
                text.push_str(&t);
                let _ = send_ipc(
                    tx,
                    &DaemonMessage::AgentStreamChunk {
                        request_id: request_id.to_string(),
                        chunk: StreamChunk::Text(t),
                    },
                )
                .await;
            }
        }
        let usage = response.usage().await;
        let input_tokens = usage.input_tokens.unwrap_or(0);
        let output_tokens = usage.output_tokens.unwrap_or(0);
        let cache_read_tokens = usage.cached_tokens.unwrap_or(0);
        Ok((text, input_tokens, output_tokens, cache_read_tokens))
    })
}

/// Spawn a background task that refreshes expiring OAuth tokens every 4 minutes.
///
/// Checks providers: claude_code, codex.
/// Errors are logged as warnings and do not propagate.
pub fn spawn_token_refresh_task() -> tokio::task::JoinHandle<()> {
    use codiv_common::auth::{provider_by_id, maybe_refresh_stored_token, AuthMethod};

    tokio::spawn(async move {
        let oauth_providers = ["claude_code", "codex"];
        loop {
            for provider_id in &oauth_providers {
                let entry = provider_by_id(provider_id);
                if let Some(entry) = entry {
                    let config = entry.auth_methods.into_iter().find_map(|m| {
                        if let AuthMethod::OAuthCode(c) = m { Some(c) } else { None }
                    });
                    if let Some(config) = config {
                        if maybe_refresh_stored_token(provider_id, &config).await.is_some() {
                            tracing::info!("refreshed OAuth token for {}", provider_id);
                        }
                    }
                }
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await; // check every 30s, only refresh within 1 min of expiry
        }
    })
}

const MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF_MS: u64 = 1000;

/// Execute a streaming LLM call, forwarding chunks over IPC.
/// Returns (response_text, input_tokens, output_tokens, cache_read_tokens, collected_events).
///
/// Retries transient errors (server errors, rate limits) up to `MAX_RETRIES`
/// times with exponential backoff. Non-retryable errors fail immediately with
/// a user-friendly message.
pub async fn stream_from_config(
    assignment: &ModelAssignment,
    provider_config: &ProviderConfig,
    system_prompt: &str,
    messages: aisdk::core::messages::Messages,
    request_id: &str,
    tx: &mpsc::Sender<Vec<u8>>,
    tools: Vec<Tool>,
    thinking: bool,
) -> Result<(String, usize, usize, usize, Vec<ConversationEvent>), DynError> {
    let mut attempt = 0u32;
    loop {
        let result = with_provider_model!(assignment, provider_config, |model, is_openai_compat| {
            let tools = if is_openai_compat {
                tools.iter().cloned().map(sanitize_tool_schema_for_openai).collect()
            } else {
                tools.clone()
            };
            run_stream(model, system_prompt, messages.clone(), request_id, tx, assignment, tools, thinking).await
        });

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

// ---------------------------------------------------------------------------
// StreamAccumulator — collects streaming state across chunk callbacks
// ---------------------------------------------------------------------------

/// Accumulates text, reasoning, and tool-call events during an LLM stream.
struct StreamAccumulator {
    full_text: String,
    chunk_count: u32,
    collected_events: Vec<ConversationEvent>,
    reasoning_buffer: String,
    reasoning_start: Option<Instant>,
    tool_call_names: HashMap<String, String>,
    request_id: String,
}

impl StreamAccumulator {
    fn new(request_id: &str) -> Self {
        Self {
            full_text: String::new(),
            chunk_count: 0,
            collected_events: Vec::new(),
            reasoning_buffer: String::new(),
            reasoning_start: None,
            tool_call_names: HashMap::new(),
            request_id: request_id.to_string(),
        }
    }

    async fn handle_text_delta(&mut self, text: String, tx: &mpsc::Sender<Vec<u8>>) -> Result<(), DynError> {
        self.full_text.push_str(&text);
        send_ipc(
            tx,
            &DaemonMessage::AgentStreamChunk {
                request_id: self.request_id.clone(),
                chunk: StreamChunk::Text(text),
            },
        )
        .await
    }

    async fn handle_reasoning_delta(&mut self, text: String, tx: &mpsc::Sender<Vec<u8>>) -> Result<(), DynError> {
        self.reasoning_start.get_or_insert(Instant::now());
        self.reasoning_buffer.push_str(&text);
        send_ipc(
            tx,
            &DaemonMessage::AgentStreamChunk {
                request_id: self.request_id.clone(),
                chunk: StreamChunk::Reasoning(text),
            },
        )
        .await
    }

    async fn handle_tool_call_delta(&self, id: String, delta: String, tx: &mpsc::Sender<Vec<u8>>) -> Result<(), DynError> {
        let tool_name = self.tool_call_names.get(&id).cloned().unwrap_or_default();
        send_ipc(
            tx,
            &DaemonMessage::AgentStreamChunk {
                request_id: self.request_id.clone(),
                chunk: StreamChunk::ToolCallDelta { tool_call_id: id, tool_name, delta },
            },
        )
        .await
    }

    fn handle_tool_call_start(&mut self, info: aisdk::core::tools::ToolDetails) {
        // Flush pre-tool-call reasoning as its own event
        if !self.reasoning_buffer.is_empty() {
            let duration = self.reasoning_start.map(|s| s.elapsed().as_secs_f32()).unwrap_or(0.0);
            self.collected_events.push(ConversationEvent::AssistantReasoning {
                request_id: self.request_id.clone(),
                text: std::mem::take(&mut self.reasoning_buffer),
                duration_secs: duration,
            });
            self.reasoning_start = None;
        }
        // Register name so ToolCallDelta can include it in IPC
        self.tool_call_names.insert(info.id.clone(), info.name.clone());
    }

    async fn handle_tool_call_available(&mut self, info: aisdk::core::ToolCallInfo, tx: &mpsc::Sender<Vec<u8>>) -> Result<(), DynError> {
        let arguments = serde_json::to_string(&info.input).unwrap_or_default();
        self.collected_events.push(ConversationEvent::ToolCall {
            request_id: self.request_id.clone(),
            tool_call_id: info.tool.id.clone(),
            tool_name: info.tool.name.clone(),
            arguments: arguments.clone(),
        });
        // For edit tools, capture the file content NOW (before the tool
        // executes) so the client can compute an accurate diff.
        let pre_edit_content = if info.tool.name == "edit" {
            info.input.get("file_path")
                .and_then(|v| v.as_str())
                .and_then(|p| std::fs::read_to_string(p).ok())
        } else {
            None
        };
        send_ipc(
            tx,
            &DaemonMessage::AgentStreamChunk {
                request_id: self.request_id.clone(),
                chunk: StreamChunk::ToolCall {
                    name: info.tool.name,
                    arguments,
                    pre_edit_content,
                },
            },
        )
        .await
    }

    async fn handle_tool_call_end(&mut self, info: aisdk::core::ToolResultInfo, tx: &mpsc::Sender<Vec<u8>>) -> Result<(), DynError> {
        let output = match &info.output {
            Ok(v) => match v {
                serde_json::Value::String(s) => s.clone(),
                other => serde_json::to_string(&other).unwrap_or_default(),
            },
            Err(e) => format!("Error: {e}"),
        };
        let output = truncate_tool_output(&output, MAX_TOOL_OUTPUT_BYTES);
        self.collected_events.push(ConversationEvent::ToolResult {
            request_id: self.request_id.clone(),
            tool_call_id: info.tool.id.clone(),
            tool_name: info.tool.name.clone(),
            result: output.clone(),
        });
        send_ipc(
            tx,
            &DaemonMessage::AgentStreamChunk {
                request_id: self.request_id.clone(),
                chunk: StreamChunk::ToolResult {
                    name: info.tool.name,
                    result: output,
                },
            },
        )
        .await
    }

    /// Flush any remaining reasoning into a final event.
    fn flush_reasoning(&mut self) {
        if !self.reasoning_buffer.is_empty() {
            let duration = self.reasoning_start.map(|s| s.elapsed().as_secs_f32()).unwrap_or(0.0);
            self.collected_events.push(ConversationEvent::AssistantReasoning {
                request_id: self.request_id.clone(),
                text: std::mem::take(&mut self.reasoning_buffer),
                duration_secs: duration,
            });
        }
    }
}

async fn run_stream<M>(
    model: M,
    system_prompt: &str,
    messages: aisdk::core::messages::Messages,
    request_id: &str,
    tx: &mpsc::Sender<Vec<u8>>,
    config: &ModelAssignment,
    tools: Vec<Tool>,
    thinking: bool,
) -> Result<(String, usize, usize, usize, Vec<ConversationEvent>), DynError>
where
    M: LanguageModel + aisdk::core::capabilities::TextInputSupport + aisdk::core::capabilities::ToolCallSupport + aisdk::core::capabilities::ReasoningSupport + Send + Sync + 'static,
{
    {
        let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        tracing::debug!(
            request_id,
            system_prompt_len = system_prompt.len(),
            message_count = messages.len(),
            tools = ?tool_names,
            thinking,
            provider = %config.provider,
            model = %config.model,
            "preparing API request"
        );
    }

    let mut builder = LanguageModelRequest::builder()
        .model(model)
        .system(system_prompt)
        .messages(messages);

    if thinking {
        // Anthropic API does not allow setting temperature with extended thinking.
        builder = builder.reasoning_effort(aisdk::core::language_model::ReasoningEffort::Medium);
    } else {
        // Explicitly disable thinking so the API doesn't default to enabling it.
        builder = builder.reasoning_effort(aisdk::core::language_model::ReasoningEffort::None);
        if let Some(temp) = config.temperature {
            builder = builder.temperature(temp as u32);
        }
    }

    for tool in tools {
        builder = builder.with_tool(tool);
    }
    let mut response = builder.build().stream_text().await?;
    let mut acc = StreamAccumulator::new(request_id);

    while let Some(chunk) = response.stream.next().await {
        acc.chunk_count += 1;

        match chunk {
            LanguageModelStreamChunkType::TextDelta(text) => {
                acc.handle_text_delta(text, tx).await?;
            }
            LanguageModelStreamChunkType::ReasoningDelta(text) => {
                acc.handle_reasoning_delta(text, tx).await?;
            }
            LanguageModelStreamChunkType::ToolCallDelta { id, delta } => {
                acc.handle_tool_call_delta(id, delta, tx).await?;
            }
            LanguageModelStreamChunkType::ToolCallStart(info) => {
                acc.handle_tool_call_start(info);
            }
            LanguageModelStreamChunkType::ToolCallAvailable(info) => {
                acc.handle_tool_call_available(info, tx).await?;
            }
            LanguageModelStreamChunkType::ToolCallEnd(info) => {
                acc.handle_tool_call_end(info, tx).await?;
            }
            LanguageModelStreamChunkType::Failed(err) => {
                tracing::error!("stream failed: {err}");
                return Err(err.to_string().into());
            }
            LanguageModelStreamChunkType::Incomplete(reason) => {
                tracing::warn!("stream incomplete: {reason}");
            }
            _ => {}
        }
    }

    // Flush accumulated reasoning into a single event.
    acc.flush_reasoning();

    tracing::info!(
        "stream ended for request {}: {} chunks, {} bytes of text",
        request_id, acc.chunk_count, acc.full_text.len()
    );

    if let Some(reason) = response.stop_reason().await {
        tracing::debug!("stop_reason: {:?}", reason);
    }

    let usage = response.usage().await;
    let input_tokens = usage.input_tokens.unwrap_or(0);
    let output_tokens = usage.output_tokens.unwrap_or(0);
    let cache_read_tokens = usage.cached_tokens.unwrap_or(0);

    Ok((acc.full_text, input_tokens, output_tokens, cache_read_tokens, acc.collected_events))
}

pub(super) async fn send_ipc(tx: &mpsc::Sender<Vec<u8>>, msg: &DaemonMessage) -> Result<(), DynError> {
    let frame = frame_message(msg)?;
    tx.send(frame).await.map_err(|_| "ipc receiver closed")?;
    Ok(())
}
