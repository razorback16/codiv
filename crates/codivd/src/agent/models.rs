use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Deserialize;
use codiv_common::permissions::PermissionMode;
use codiv_common::types::AgentRole;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

pub(crate) type DynError = Box<dyn std::error::Error + Send + Sync>;

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

#[derive(Debug, Clone, Deserialize)]
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

const DEFAULT_CONFIG_TOML: &str = include_str!("../../config.default.toml");

// ---------------------------------------------------------------------------
// AppConfig -- unified configuration (permissions + model catalog)
// ---------------------------------------------------------------------------

fn default_threshold_percent() -> u8 { 75 }

#[derive(Debug, Clone, Deserialize)]
pub struct CompactionConfig {
    /// Trigger auto-compaction when input_tokens exceed this % of context_window.
    #[serde(default = "default_threshold_percent")]
    pub threshold_percent: u8,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            threshold_percent: default_threshold_percent(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub permissions: PermissionsConfig,
    #[serde(default)]
    pub compaction: CompactionConfig,
    #[serde(flatten)]
    pub models: ModelCatalog,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PermissionsConfig {
    #[serde(default)]
    pub mode: PermissionMode,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

impl PermissionsConfig {
    /// Look up whether a tool (optionally with a bash command prefix) is in the
    /// allow or deny list. Returns `Some("allow")`, `Some("deny")`, or `None`.
    pub fn lookup(&self, tool_name: &str, command_prefix: Option<&str>) -> Option<&str> {
        let key = match command_prefix {
            Some(prefix) => format!("{}:{}", tool_name, prefix),
            None => tool_name.to_string(),
        };
        if self.deny.contains(&key) { return Some("deny"); }
        if self.allow.contains(&key) { return Some("allow"); }
        if command_prefix.is_some() {
            let tool_key = tool_name.to_string();
            if self.deny.contains(&tool_key) { return Some("deny"); }
            if self.allow.contains(&tool_key) { return Some("allow"); }
        }
        None
    }
}

/// Append an entry to the `allow` or `deny` array in the `[permissions]`
/// section of `~/.codiv/config.toml`, using `toml_edit` for
/// format-preserving edits.
pub fn add_permission_to_config(entry: &str, list: &str) {
    use toml_edit::{DocumentMut, value, Array};
    let config_path = AppConfig::config_path();
    let content = std::fs::read_to_string(&config_path).unwrap_or_default();
    let mut doc = content.parse::<DocumentMut>().unwrap_or_else(|_| DocumentMut::new());
    if !doc.contains_table("permissions") {
        doc["permissions"] = toml_edit::table();
    }
    let perms = doc["permissions"].as_table_mut().unwrap();
    if !perms.contains_key(list) {
        perms[list] = value(Array::new());
    }
    let arr = perms[list].as_value_mut().and_then(|v| v.as_array_mut()).unwrap();
    if !arr.iter().any(|v| v.as_str() == Some(entry)) {
        arr.push(entry);
    }
    if let Err(e) = std::fs::write(&config_path, doc.to_string()) {
        tracing::warn!("Failed to write permission config to {}: {e}", config_path.display());
    }
}

impl AppConfig {
    /// Resolve the canonical config file path (`~/.codiv/config.toml`).
    pub fn config_path() -> PathBuf {
        codiv_common::config::config_dir().join("config.toml")
    }

    /// Load configuration, trying (in order):
    /// 1. `~/.codiv/config.toml`
    /// 2. `~/.codiv/models.toml` (backward compat -- parsed as ModelCatalog)
    /// 3. Embedded default template (also written to `config.toml` for discoverability)
    pub fn load() -> Self {
        let config_dir = codiv_common::config::config_dir();
        let config_path = config_dir.join("config.toml");

        if let Ok(contents) = std::fs::read_to_string(&config_path) {
            if let Ok(cfg) = toml::from_str::<AppConfig>(&contents) {
                return cfg;
            }
        }

        // Create default config.toml so users can discover and edit it.
        let _ = std::fs::create_dir_all(&config_dir);
        let _ = std::fs::write(&config_path, DEFAULT_CONFIG_TOML);

        toml::from_str(DEFAULT_CONFIG_TOML).expect("default config.toml must parse")
    }

    /// Wrap this config in an `Arc<RwLock<>>` and start a filesystem watcher
    /// that automatically re-parses on changes.
    /// Returns the shared handle and a guard that keeps the watcher alive.
    pub fn into_watched(self, change_tx: Option<tokio::sync::mpsc::Sender<()>>) -> (Arc<RwLock<AppConfig>>, ConfigWatcherGuard) {
        let shared = Arc::new(RwLock::new(self));
        let guard = ConfigWatcherGuard::start(Arc::clone(&shared), change_tx);
        (shared, guard)
    }
}

// ---------------------------------------------------------------------------
// Filesystem watcher
// ---------------------------------------------------------------------------

/// Holds the `notify` watcher so it stays alive as long as needed.
pub struct ConfigWatcherGuard {
    _watcher: RecommendedWatcher,
}

impl ConfigWatcherGuard {
    fn start(shared: Arc<RwLock<AppConfig>>, change_tx: Option<tokio::sync::mpsc::Sender<()>>) -> Self {
        let config_path = AppConfig::config_path();
        let watch_dir = config_path
            .parent()
            .expect("config path must have parent")
            .to_path_buf();

        let target_file = config_path.clone();
        let mut watcher = notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                // Only react to writes/creates that touch our config file.
                let dominated = event.paths.iter().any(|p| p == &target_file);
                if !dominated {
                    return;
                }
                match event.kind {
                    notify::EventKind::Create(_) | notify::EventKind::Modify(_) => {
                        tracing::info!("config file changed, reloading");
                        let new_cfg = AppConfig::load();
                        if let Ok(mut guard) = shared.write() {
                            *guard = new_cfg;
                        }
                        if let Some(ref tx) = change_tx {
                            let _ = tx.try_send(());
                        }
                    }
                    _ => {}
                }
            }
        })
        .expect("failed to create config file watcher");

        watcher
            .watch(&watch_dir, RecursiveMode::NonRecursive)
            .expect("failed to watch config directory");

        Self { _watcher: watcher }
    }
}

// ---------------------------------------------------------------------------
// ModelCatalog
// ---------------------------------------------------------------------------

impl ModelCatalog {
    /// Load a `ModelCatalog` (delegates to `AppConfig::load()`).
    #[allow(dead_code)]
    pub fn load() -> Self {
        AppConfig::load().models
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_exact_match() {
        let cfg = PermissionsConfig {
            mode: PermissionMode::Auto,
            allow: vec!["read".to_string()],
            deny: vec![],
        };
        assert_eq!(cfg.lookup("read", None), Some("allow"));
        assert_eq!(cfg.lookup("write", None), None);
    }

    #[test]
    fn lookup_bash_prefix() {
        let cfg = PermissionsConfig {
            mode: PermissionMode::Auto,
            allow: vec!["bash:cargo build".to_string()],
            deny: vec![],
        };
        assert_eq!(cfg.lookup("bash", Some("cargo build")), Some("allow"));
        assert_eq!(cfg.lookup("bash", Some("cargo test")), None);
        assert_eq!(cfg.lookup("bash", Some("rm")), None);
    }

    #[test]
    fn lookup_deny_wins() {
        let cfg = PermissionsConfig {
            mode: PermissionMode::Auto,
            allow: vec!["bash:cargo build".to_string()],
            deny: vec!["bash:cargo build".to_string()],
        };
        assert_eq!(cfg.lookup("bash", Some("cargo build")), Some("deny"));
    }

    #[test]
    fn lookup_fallback_tool_only() {
        let cfg = PermissionsConfig {
            mode: PermissionMode::Auto,
            allow: vec!["bash".to_string()],
            deny: vec![],
        };
        assert_eq!(cfg.lookup("bash", Some("anything")), Some("allow"));
    }
}
