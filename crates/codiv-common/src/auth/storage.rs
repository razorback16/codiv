//! Credential storage — read/write auth credentials to ~/.codiv/config.toml.
//!
//! Uses toml_edit for format-preserving merges (preserves comments, section order).
//! Follows the same pattern as add_permission_to_config() in codivd.

use anyhow::Result;
use chrono::{DateTime, Utc};
use toml_edit::{DocumentMut, value};

use crate::config::config_dir;
use super::types::{AccessToken, OAuthTokens, RefreshToken};

fn config_path() -> std::path::PathBuf {
    config_dir().join("config.toml")
}

fn load_doc() -> DocumentMut {
    let content = std::fs::read_to_string(config_path()).unwrap_or_default();
    content.parse::<DocumentMut>().unwrap_or_else(|_| DocumentMut::new())
}

fn save_doc(doc: &DocumentMut) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, doc.to_string())?;
    Ok(())
}

/// Write an API key for a provider into [providers.{provider_id}] api_key.
///
/// Merges with existing config.toml — does not overwrite unrelated sections.
/// For Anthropic: provider_id = "anthropic"
/// For OpenAI: provider_id = "openai"
pub fn write_api_key_to_config(provider_id: &str, api_key: &str) -> Result<()> {
    let mut doc = load_doc();

    if !doc.contains_table("providers") {
        doc["providers"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    let providers = doc["providers"]
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("config.toml: 'providers' exists but is not a table"))?;
    if !providers.contains_key(provider_id) {
        providers[provider_id] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    providers[provider_id]["api_key"] = value(api_key);

    save_doc(&doc)
}

/// Write OAuth tokens for a provider into:
///   [auth.tokens.{provider_id}] — full token record
///   [providers.{provider_id}]   — access_token as api_key (daemon compatibility)
///
/// The daemon reads [providers.{provider_id}] api_key via hot-reload.
pub fn write_oauth_tokens_to_config(provider_id: &str, tokens: &OAuthTokens) -> Result<()> {
    let mut doc = load_doc();

    // Write [auth.tokens.{provider_id}]
    {
        // Ensure [auth] exists
        if !doc.contains_table("auth") {
            doc["auth"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let auth = doc["auth"]
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("config.toml: 'auth' exists but is not a table"))?;
        // Ensure [auth.tokens] exists
        if !auth.contains_key("tokens") {
            auth["tokens"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let tokens_table = auth["tokens"]
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("config.toml: 'auth.tokens' exists but is not a table"))?;
        // Ensure [auth.tokens.{provider_id}] exists
        if !tokens_table.contains_key(provider_id) {
            tokens_table[provider_id] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let entry = tokens_table[provider_id]
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("config.toml: 'auth.tokens.{provider_id}' exists but is not a table"))?;
        entry["access_token"] = value(tokens.access_token.as_str());
        if let Some(rt) = &tokens.refresh_token {
            entry["refresh_token"] = value(rt.as_str());
        }
        if let Some(ref id_token) = tokens.id_token {
            entry["id_token"] = value(id_token.as_str());
        }
        entry["expires_at"] = value(tokens.expires_at.to_rfc3339());
    }

    // Also write access_token to [providers.{provider_id}] api_key
    // so existing build_anthropic_model() / build_openai_model() pick it up via hot-reload.
    {
        if !doc.contains_table("providers") {
            doc["providers"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let providers = doc["providers"]
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("config.toml: 'providers' exists but is not a table"))?;
        if !providers.contains_key(provider_id) {
            providers[provider_id] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        providers[provider_id]["api_key"] = value(tokens.access_token.as_str());
    }

    save_doc(&doc)
}

/// Write the account UUID for an OAuth provider into [auth.tokens.{provider_id}].
pub fn write_oauth_account_uuid(provider_id: &str, uuid: &str) -> Result<()> {
    let mut doc = load_doc();

    if !doc.contains_table("auth") {
        doc["auth"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    let auth = doc["auth"]
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("config.toml: 'auth' exists but is not a table"))?;
    if !auth.contains_key("tokens") {
        auth["tokens"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    let tokens_table = auth["tokens"]
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("config.toml: 'auth.tokens' exists but is not a table"))?;
    if !tokens_table.contains_key(provider_id) {
        tokens_table[provider_id] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    let entry = tokens_table[provider_id]
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("config.toml: 'auth.tokens.{provider_id}' exists but is not a table"))?;
    entry["account_uuid"] = value(uuid);

    save_doc(&doc)
}

/// Read the account UUID for an OAuth provider from [auth.tokens.{provider_id}].
pub fn read_oauth_account_uuid(provider_id: &str) -> Option<String> {
    let doc = load_doc();
    doc.get("auth")
        .and_then(|a| a.as_table())
        .and_then(|t| t.get("tokens"))
        .and_then(|t| t.as_table())
        .and_then(|t| t.get(provider_id))
        .and_then(|e| e.as_table())
        .and_then(|e| e.get("account_uuid"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Read stored OAuth tokens for a provider from [auth.tokens.{provider_id}].
/// Returns Ok(None) if no tokens are stored for this provider.
pub fn read_oauth_tokens_from_config(provider_id: &str) -> Result<Option<OAuthTokens>> {
    let doc = load_doc();
    let entry = doc
        .get("auth")
        .and_then(|a| a.as_table())
        .and_then(|t| t.get("tokens"))
        .and_then(|t| t.as_table())
        .and_then(|t| t.get(provider_id))
        .and_then(|e| e.as_table());

    let Some(entry) = entry else { return Ok(None) };

    let access_token = entry
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing access_token for {}", provider_id))?;

    let refresh_token = entry
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(RefreshToken::new);

    let id_token = entry
        .get("id_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let expires_at_str = entry
        .get("expires_at")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing expires_at for {}", provider_id))?;

    let expires_at = DateTime::parse_from_rfc3339(expires_at_str)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| anyhow::anyhow!("invalid expires_at: {e}"))?;

    Ok(Some(OAuthTokens {
        access_token: AccessToken::new(access_token),
        refresh_token,
        expires_at,
        id_token,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use std::fs;
    use std::sync::Mutex;
    use tempfile::TempDir;

    // Serialize all tests that mutate HOME to avoid data races.
    static HOME_LOCK: Mutex<()> = Mutex::new(());

    fn with_temp_config<F: FnOnce()>(f: F) {
        let _guard = HOME_LOCK.lock().unwrap();
        let dir = TempDir::new().unwrap();
        let codiv_dir = dir.path().join(".codiv");
        fs::create_dir_all(&codiv_dir).unwrap();
        let old_home = std::env::var("HOME").unwrap_or_default();
        unsafe { std::env::set_var("HOME", dir.path()) };
        f();
        unsafe { std::env::set_var("HOME", &old_home) };
    }

    #[test]
    fn write_api_key_creates_providers_section() {
        with_temp_config(|| {
            write_api_key_to_config("anthropic", "sk-ant-test-key").unwrap();
            let content = fs::read_to_string(config_path()).unwrap();
            assert!(content.contains("[providers.anthropic]") || content.contains("anthropic"));
            assert!(content.contains("sk-ant-test-key"));
        });
    }

    #[test]
    fn write_api_key_preserves_existing_sections() {
        with_temp_config(|| {
            let initial = "[permissions]\nmode = \"ask\"\n";
            fs::write(config_path(), initial).unwrap();

            write_api_key_to_config("openai", "sk-openai-test").unwrap();
            let content = fs::read_to_string(config_path()).unwrap();
            assert!(content.contains("mode = \"ask\""), "permissions section was destroyed");
            assert!(content.contains("sk-openai-test"));
        });
    }

    #[test]
    fn write_oauth_tokens_round_trips() {
        with_temp_config(|| {
            let expires_at = Utc::now() + Duration::hours(1);
            let tokens = OAuthTokens::new(
                AccessToken::new("access-tok"),
                Some(RefreshToken::new("refresh-tok")),
                expires_at,
            );
            write_oauth_tokens_to_config("claude_code", &tokens).unwrap();

            let loaded = read_oauth_tokens_from_config("claude_code").unwrap().unwrap();
            assert_eq!(loaded.access_token.as_str(), "access-tok");
            assert_eq!(loaded.refresh_token.as_ref().map(|r| r.as_str()), Some("refresh-tok"));
        });
    }

    #[test]
    fn write_oauth_tokens_also_writes_provider_api_key() {
        with_temp_config(|| {
            let expires_at = Utc::now() + Duration::hours(1);
            let tokens = OAuthTokens::new(
                AccessToken::new("access-tok"),
                None::<RefreshToken>,
                expires_at,
            );
            write_oauth_tokens_to_config("claude_code", &tokens).unwrap();
            let content = fs::read_to_string(config_path()).unwrap();
            // [providers.claude_code] api_key must be set for daemon compatibility
            assert!(content.contains("access-tok"));
        });
    }

    #[test]
    fn read_oauth_tokens_returns_none_for_missing_provider() {
        with_temp_config(|| {
            let result = read_oauth_tokens_from_config("nonexistent").unwrap();
            assert!(result.is_none());
        });
    }
}
