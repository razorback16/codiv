use std::sync::{Arc, RwLock};

use crate::agent::config::{self, ModelAssignment, ModelCatalog, ProviderConfig};
use aisdk::core::messages::{AssistantMessage, Message, Messages};
use aisdk::core::language_model::LanguageModelResponseContentType;
use aisdk::core::tools::{ToolCallInfo, ToolDetails, ToolResultInfo};
use aisdk::extensions::Extensions;
use codiv_common::conversation::ConversationEvent;
use codiv_common::truncate::truncate_output;
use codiv_common::types::AgentRole;
use tokio::sync::mpsc;
use tracing::info;

fn truncate_str(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        // Find a char boundary at or before max
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}

/// Maximum number of events to keep in the session timeline.
const MAX_HISTORY_EVENTS: usize = 100;

/// Default head/tail lines for output truncation.
const TRUNCATE_HEAD: usize = 20;
const TRUNCATE_TAIL: usize = 20;

pub struct Agent {
    pub role: AgentRole,
    pub system_prompt: String,
    pub model_config: ModelAssignment,
    pub provider_config: ProviderConfig,
    pub history: Vec<ConversationEvent>,
    pub cwd: String,
    pub relay_manager: Option<Arc<super::relay_manager::RelayManager>>,
    pub cwd_ref: Arc<RwLock<String>>,
}

impl Agent {
    pub fn new(role: AgentRole, model_config: ModelAssignment, provider_config: ProviderConfig, system_prompt: String, cwd: String) -> Self {
        let cwd_ref = Arc::new(RwLock::new(cwd.clone()));
        Self {
            role,
            system_prompt,
            model_config,
            provider_config,
            history: Vec::new(),
            cwd,
            relay_manager: None,
            cwd_ref,
        }
    }

    pub fn add_user_message(&mut self, content: &str, request_id: &str) {
        self.history.push(ConversationEvent::UserPrompt {
            text: content.to_string(),
            request_id: request_id.to_string(),
        });
        self.enforce_limit();
    }

    pub fn add_assistant_message(&mut self, content: &str, request_id: &str) {
        self.history.push(ConversationEvent::AssistantText {
            request_id: request_id.to_string(),
            text: content.to_string(),
        });
        self.enforce_limit();
    }

    pub fn add_command_result(&mut self, command: &str, output: &str, exit_code: i32, cwd: &str) {
        let truncated = truncate_output(output, TRUNCATE_HEAD, TRUNCATE_TAIL);
        self.history.push(ConversationEvent::ShellCommand {
            command: command.to_string(),
            output: truncated,
            exit_code,
            cwd: cwd.to_string(),
        });
        self.enforce_limit();
    }

    /// Push tool-related events (ToolCall, ToolResult, AssistantReasoning)
    /// collected during streaming into the history.
    pub fn add_tool_events(&mut self, events: &[ConversationEvent]) {
        self.history.extend(events.iter().cloned());
        self.enforce_limit();
    }

    fn enforce_limit(&mut self) {
        if self.history.len() > MAX_HISTORY_EVENTS {
            let excess = self.history.len() - MAX_HISTORY_EVENTS;
            self.history.drain(..excess);
        }
    }

    /// Build a compact context summary for the LLM permission evaluator.
    /// Includes the last few user queries and assistant responses so the
    /// evaluator understands what the user asked for.
    fn build_context_summary(&self) -> String {
        let mut lines = Vec::new();
        // Take the last 6 events to keep it concise
        let start = self.history.len().saturating_sub(6);
        for event in &self.history[start..] {
            match event {
                ConversationEvent::UserPrompt { text, .. } => {
                    lines.push(format!("User: {}", truncate_str(text, 200)));
                }
                ConversationEvent::AssistantText { text, .. } => {
                    lines.push(format!("Assistant: {}", truncate_str(text, 200)));
                }
                ConversationEvent::ShellCommand { command, exit_code, .. } => {
                    lines.push(format!("Shell: {} (exit {})", command, exit_code));
                }
                ConversationEvent::ToolCall { tool_name, .. } => {
                    lines.push(format!("ToolCall: {}", tool_name));
                }
                ConversationEvent::ToolResult { tool_name, result, .. } => {
                    lines.push(format!("ToolResult({}): {}", tool_name, truncate_str(result, 100)));
                }
                _ => {}
            }
        }
        lines.join("\n")
    }

    /// Compact the entire conversation history into a summary, streaming
    /// the result via IPC. Returns (summary, compacted_count, input_tokens,
    /// output_tokens, cache_read_tokens).
    pub async fn compact(
        &mut self,
        models: &ModelCatalog,
        client_tx: &tokio::sync::mpsc::Sender<Vec<u8>>,
        request_id: &str,
    ) -> Result<(String, usize, usize, usize, usize), String> {
        if self.history.is_empty() {
            return Err("no events to compact".to_string());
        }
        let compacted_count = self.history.len();

        // Build text representation of all events
        let mut context = String::new();
        for event in &self.history {
            match event {
                ConversationEvent::UserPrompt { text, .. } => {
                    context.push_str(&format!("User: {}\n", text));
                }
                ConversationEvent::AssistantText { text, .. } => {
                    context.push_str(&format!("Assistant: {}\n", text));
                }
                ConversationEvent::ToolCall { tool_name, arguments, .. } => {
                    context.push_str(&format!(
                        "Tool call: {} {}\n",
                        tool_name,
                        truncate_str(arguments, 200)
                    ));
                }
                ConversationEvent::ToolResult { tool_name, result, .. } => {
                    context.push_str(&format!(
                        "Tool result ({}): {}\n",
                        tool_name,
                        truncate_str(result, 500)
                    ));
                }
                ConversationEvent::ShellCommand { command, output, exit_code, .. } => {
                    context.push_str(&format!(
                        "Shell: {} (exit {}) -> {}\n",
                        command,
                        exit_code,
                        truncate_str(output, 200)
                    ));
                }
                ConversationEvent::Summary { text, .. } => {
                    context.push_str(&format!("Previous summary: {}\n", text));
                }
                _ => {}
            }
        }

        let assignment = models
            .roles
            .get("compactor")
            .cloned()
            .ok_or("no compactor role configured in config.toml")?;
        let provider_config = models.resolve_provider_config(&assignment);

        let system = "You are a conversation summarizer. Produce a concise summary of the conversation below. \
            Focus on: what the user asked for, what actions were taken (files edited, commands run), \
            what the current state of the task is, and any important decisions made. \
            Keep it under 500 words. Do not include pleasantries or meta-commentary.";

        let (summary, input_tokens, output_tokens, cache_read_tokens) =
            config::streaming_text_completion(
                &assignment,
                &provider_config,
                system,
                &context,
                request_id,
                client_tx,
            )
            .await
            .map_err(|e| e.to_string())?;

        // Replace entire history with summary
        self.history.clear();
        self.history.push(ConversationEvent::Summary {
            text: summary.clone(),
            compacted_event_count: compacted_count,
        });

        Ok((summary, compacted_count, input_tokens, output_tokens, cache_read_tokens))
    }

    /// Build aisdk Messages from the session timeline.
    ///
    /// ShellCommands become assistant + user message pairs representing
    /// the command execution and its output.
    /// UserPrompts become User messages.
    /// AssistantText become Assistant messages.
    /// ToolCall/ToolResult are mapped to appropriate aisdk tool message pairs.
    fn build_messages(&self) -> Messages {
        use aisdk::core::messages::UserMessage;

        // Build the message list directly (instead of via the builder) so we
        // can include native tool-call / tool-result message variants that the
        // builder does not expose convenience methods for.
        // Note: system prompt is passed separately via .system() on the request
        // builder, not as a Message::System in the vec (aisdk pattern).
        let mut messages: Messages = Vec::new();

        for event in &self.history {
            match event {
                ConversationEvent::ShellCommand {
                    command,
                    output,
                    exit_code,
                    cwd,
                } => {
                    // Represent shell commands as assistant/user message pairs
                    messages.push(Message::Assistant(
                        format!("Running command: {} (in {})", command, cwd).into(),
                    ));

                    let result_text = if *exit_code == 0 {
                        output.clone()
                    } else {
                        format!("{}\n[exit code: {}]", output, exit_code)
                    };
                    messages.push(Message::User(UserMessage::new(format!(
                        "[Terminal output for `{}`]:\n{}",
                        command, result_text
                    ))));
                }
                ConversationEvent::UserPrompt { text, .. } => {
                    messages.push(Message::User(UserMessage::new(text.clone())));
                }
                ConversationEvent::AssistantText { text, .. } => {
                    messages.push(Message::Assistant(text.clone().into()));
                }
                ConversationEvent::AssistantReasoning { .. } => {
                    // Reasoning is internal; not included in the message history
                    // sent back to the model.
                }
                ConversationEvent::ToolCall { tool_name, arguments, request_id } => {
                    // Native aisdk tool-call message format.
                    // We synthesize a tool_call_id from request_id + tool_name
                    // since ConversationEvent doesn't store the provider's ID.
                    let tool_call_id = format!("{}_{}", request_id, tool_name);
                    let input: serde_json::Value = serde_json::from_str(arguments)
                        .unwrap_or_else(|_| serde_json::Value::String(arguments.clone()));
                    let tool_call = ToolCallInfo {
                        tool: ToolDetails {
                            name: tool_name.clone(),
                            id: tool_call_id,
                        },
                        input,
                        extensions: Extensions::default(),
                    };
                    messages.push(Message::Assistant(AssistantMessage::new(
                        LanguageModelResponseContentType::ToolCall(tool_call),
                        None,
                    )));
                }
                ConversationEvent::ToolResult { tool_name, result, request_id } => {
                    // Native aisdk tool-result message format.
                    let tool_call_id = format!("{}_{}", request_id, tool_name);
                    let output: serde_json::Value = serde_json::from_str(result)
                        .unwrap_or_else(|_| serde_json::Value::String(result.clone()));
                    let tool_result = ToolResultInfo {
                        tool: ToolDetails {
                            name: tool_name.clone(),
                            id: tool_call_id,
                        },
                        output: Ok(output),
                    };
                    messages.push(Message::Tool(tool_result));
                }
                ConversationEvent::Error { message, .. } => {
                    messages.push(Message::User(UserMessage::new(format!("[Error: {}]", message))));
                }
                ConversationEvent::TokenUsage { .. } => {
                    // Metadata only — not part of the conversation history.
                }
                ConversationEvent::Summary { text, .. } => {
                    messages.push(Message::User(UserMessage::new(format!(
                        "[Conversation summary — earlier messages were compacted]:\n{}",
                        text
                    ))));
                }
            }
        }

        messages
    }

    /// Run a streaming LLM call, forwarding chunks over IPC via `client_tx`.
    /// Returns (response_text, input_tokens, output_tokens, cache_read_tokens, collected_events).
    pub async fn run_streaming(
        &self,
        request_id: &str,
        client_tx: &mpsc::Sender<Vec<u8>>,
        thinking: bool,
        permission_ctx: Option<Arc<super::permissions::PermissionContext>>,
    ) -> Result<(String, usize, usize, usize, Vec<ConversationEvent>), String> {
        info!(
            "agent {:?} calling {}/{} ({} events in history)",
            self.role,
            self.model_config.provider,
            self.model_config.model,
            self.history.len()
        );

        // Update the permission context with recent conversation history
        // so the LLM evaluator can make informed decisions.
        if let Some(ref ctx) = permission_ctx {
            ctx.set_context(self.build_context_summary());
        }

        let messages = self.build_messages();

        // Update the shared cwd reference so tools use the latest cwd
        {
            let mut cwd_guard = self.cwd_ref.write().unwrap();
            *cwd_guard = self.cwd.clone();
        }

        let relay = self.relay_manager.clone().expect("relay_manager must be set before run_streaming");
        let tools = super::tools::build_tools(
            relay,
            Arc::clone(&self.cwd_ref),
            permission_ctx,
        );

        config::stream_from_config(&self.model_config, &self.provider_config, &self.system_prompt, messages, request_id, client_tx, tools, thinking)
            .await
            .map_err(|e| e.to_string())
    }
}
