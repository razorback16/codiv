use std::collections::HashMap;

use url::Url;

use super::types::{AuthMethod, OAuthConfig};

/// Metadata for a provider in the registry.
#[derive(Debug, Clone)]
pub struct ProviderEntry {
    /// Stable string identifier (e.g., "anthropic", "openai", "claude_code", "codex").
    pub id: &'static str,
    /// Human-readable display name for the wizard UI.
    pub display_name: &'static str,
    /// Auth methods this provider supports, in preference order.
    pub auth_methods: Vec<AuthMethod>,
}

/// Returns the full provider registry for v1 providers.
///
/// Providers: Anthropic, OpenAI, Claude Code, Codex.
pub fn provider_registry() -> Vec<ProviderEntry> {
    vec![]
}

/// Look up a provider entry by its stable ID.
///
/// Returns `None` if no provider with that ID exists in the registry.
pub fn provider_by_id(id: &str) -> Option<ProviderEntry> {
    provider_registry().into_iter().find(|p| p.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_exactly_four_providers() {
        assert_eq!(provider_registry().len(), 4);
    }

    #[test]
    fn registry_contains_anthropic() {
        assert!(provider_registry().iter().any(|p| p.id == "anthropic"));
    }

    #[test]
    fn registry_contains_openai() {
        assert!(provider_registry().iter().any(|p| p.id == "openai"));
    }

    #[test]
    fn registry_contains_claude_code() {
        assert!(provider_registry().iter().any(|p| p.id == "claude_code"));
    }

    #[test]
    fn registry_contains_codex() {
        assert!(provider_registry().iter().any(|p| p.id == "codex"));
    }

    #[test]
    fn anthropic_uses_api_key_auth() {
        let entry = provider_by_id("anthropic").expect("anthropic must exist");
        assert_eq!(entry.auth_methods.len(), 1);
        assert!(matches!(entry.auth_methods[0], AuthMethod::ApiKey));
    }

    #[test]
    fn openai_uses_api_key_auth() {
        let entry = provider_by_id("openai").expect("openai must exist");
        assert_eq!(entry.auth_methods.len(), 1);
        assert!(matches!(entry.auth_methods[0], AuthMethod::ApiKey));
    }

    #[test]
    fn claude_code_uses_oauth_code_with_correct_client_id() {
        let entry = provider_by_id("claude_code").expect("claude_code must exist");
        assert_eq!(entry.auth_methods.len(), 1);
        let config = entry.auth_methods[0]
            .oauth_config()
            .expect("claude_code must have oauth config");
        assert_eq!(config.client_id, "9d1c250a-e61b-44d9-88ed-5944d1962f5e");
        assert!(config.use_pkce);
    }

    #[test]
    fn codex_uses_oauth_code_with_correct_client_id() {
        let entry = provider_by_id("codex").expect("codex must exist");
        assert_eq!(entry.auth_methods.len(), 1);
        let config = entry.auth_methods[0]
            .oauth_config()
            .expect("codex must have oauth config");
        assert_eq!(config.client_id, "app_EMoamEEZ73f0CkXaXp7hrann");
        assert!(!config.use_pkce);
    }

    #[test]
    fn provider_by_id_returns_none_for_unknown() {
        assert!(provider_by_id("unknown_provider").is_none());
    }
}
