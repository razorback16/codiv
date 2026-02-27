use serde::Deserialize;
use slate_common::types::AgentRole;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct ModelCatalog {
    pub default_provider: String,
    pub roles: HashMap<String, ModelAssignment>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelAssignment {
    pub provider: String,
    pub model: String,
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
            model: "claude-sonnet-4-5-20250514".to_string(),
            max_tokens: Some(8192),
            temperature: None,
        })
    }
}
