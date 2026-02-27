use crate::agent::config::ModelAssignment;
use slate_common::types::AgentRole;
use tracing::info;

pub enum AgentStep {
    ToolsExecuted,
    Complete(String),
    Error(String),
}

pub struct Agent {
    pub role: AgentRole,
    pub system_prompt: String,
    pub model_config: ModelAssignment,
    pub messages: Vec<AgentMessage>,
}

pub struct AgentMessage {
    pub role: MessageRole,
    pub content: String,
}

pub enum MessageRole {
    System,
    User,
    Assistant,
}

impl Agent {
    pub fn new(role: AgentRole, model_config: ModelAssignment, system_prompt: String) -> Self {
        Self {
            role,
            system_prompt,
            model_config,
            messages: Vec::new(),
        }
    }

    pub fn add_user_message(&mut self, content: &str) {
        self.messages.push(AgentMessage {
            role: MessageRole::User,
            content: content.to_string(),
        });
    }

    /// Run the agent loop: LLM call -> tool execution -> repeat until done.
    ///
    /// TODO: Wire to aisdk.rs LanguageModelRequest when a Rust-compatible
    /// aisdk crate becomes available. The reference at references/aisdk.rs/
    /// is a TypeScript/Node.js package and cannot be used as a Rust dependency.
    pub async fn run(&mut self) -> Result<String, String> {
        info!(
            "agent {:?} running with model {}/{}",
            self.role, self.model_config.provider, self.model_config.model
        );

        // Placeholder — will be replaced with aisdk.rs integration
        Ok(format!(
            "Agent {:?} would process with {}/{}",
            self.role, self.model_config.provider, self.model_config.model
        ))
    }
}
