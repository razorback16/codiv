use std::sync::Arc;

use crate::agent::config::{self, ModelAssignment, ProviderConfig};
use aisdk::core::messages::{Message, Messages};
use slate_common::truncate::truncate_output;
use slate_common::types::AgentRole;
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

/// A single event in the unified session timeline.
pub enum SessionEvent {
    /// User ran a shell command in the terminal.
    ShellCommand {
        command: String,
        output: String,
        exit_code: i32,
        cwd: String,
    },
    /// User asked the AI a question (e.g., `? ...`).
    UserQuery(String),
    /// AI responded with text.
    AssistantResponse(String),
}

pub struct Agent {
    pub role: AgentRole,
    pub system_prompt: String,
    pub model_config: ModelAssignment,
    pub provider_config: ProviderConfig,
    pub history: Vec<SessionEvent>,
    pub cwd: String,
    pub env_vars: Vec<(String, String)>,
}

impl Agent {
    pub fn new(role: AgentRole, model_config: ModelAssignment, provider_config: ProviderConfig, system_prompt: String, cwd: String, env_vars: Vec<(String, String)>) -> Self {
        Self {
            role,
            system_prompt,
            model_config,
            provider_config,
            history: Vec::new(),
            cwd,
            env_vars,
        }
    }

    pub fn add_user_message(&mut self, content: &str) {
        self.history
            .push(SessionEvent::UserQuery(content.to_string()));
        self.enforce_limit();
    }

    pub fn add_assistant_message(&mut self, content: &str) {
        self.history
            .push(SessionEvent::AssistantResponse(content.to_string()));
        self.enforce_limit();
    }

    pub fn add_command_result(&mut self, command: &str, output: &str, exit_code: i32, cwd: &str) {
        let truncated = truncate_output(output, TRUNCATE_HEAD, TRUNCATE_TAIL);
        self.history.push(SessionEvent::ShellCommand {
            command: command.to_string(),
            output: truncated,
            exit_code,
            cwd: cwd.to_string(),
        });
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
                SessionEvent::UserQuery(text) => {
                    lines.push(format!("User: {}", truncate_str(text, 200)));
                }
                SessionEvent::AssistantResponse(text) => {
                    lines.push(format!("Assistant: {}", truncate_str(text, 200)));
                }
                SessionEvent::ShellCommand { command, exit_code, .. } => {
                    lines.push(format!("Shell: {} (exit {})", command, exit_code));
                }
            }
        }
        lines.join("\n")
    }

    /// Build aisdk Messages from the session timeline.
    ///
    /// ShellCommands become assistant + user message pairs representing
    /// the command execution and its output.
    /// UserQueries become User messages.
    /// AssistantResponses become Assistant messages.
    fn build_messages(&self) -> Messages {
        let mut builder = Message::builder().system(&self.system_prompt);

        for event in &self.history {
            match event {
                SessionEvent::ShellCommand {
                    command,
                    output,
                    exit_code,
                    cwd,
                } => {
                    // Represent shell commands as assistant/user message pairs
                    builder =
                        builder.assistant(format!("Running command: {} (in {})", command, cwd));

                    let result_text = if *exit_code == 0 {
                        output.clone()
                    } else {
                        format!("{}\n[exit code: {}]", output, exit_code)
                    };
                    builder = builder.user(format!(
                        "[Terminal output for `{}`]:\n{}",
                        command, result_text
                    ));
                }
                SessionEvent::UserQuery(text) => {
                    builder = builder.user(text.clone());
                }
                SessionEvent::AssistantResponse(text) => {
                    builder = builder.assistant(text.clone());
                }
            }
        }

        builder.build()
    }

    /// Run a streaming LLM call, forwarding chunks over IPC via `client_tx`.
    /// Returns (response_text, input_tokens, output_tokens).
    pub async fn run_streaming(
        &self,
        request_id: &str,
        client_tx: &mpsc::Sender<Vec<u8>>,
        thinking: bool,
        permission_ctx: Option<Arc<super::permissions::PermissionContext>>,
    ) -> Result<(String, usize, usize), String> {
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

        let tools = super::tools::build_tools(
            self.cwd.clone(),
            self.env_vars.clone(),
            permission_ctx,
        );

        config::stream_from_config(&self.model_config, &self.provider_config, messages, request_id, client_tx, tools, thinking)
            .await
            .map_err(|e| e.to_string())
    }
}
