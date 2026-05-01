use aisdk::core::tools::Tool;
use aisdk::core::{LanguageModel, LanguageModelRequest, LanguageModelStreamChunkType};
use aisdk::providers::{Anthropic, Google, OpenAI, OpenAICompatible, Vllm};
use futures::StreamExt;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Deserialize;
use codiv_common::conversation::ConversationEvent;
use codiv_common::messages::{frame_message, DaemonMessage, StreamChunk};
use codiv_common::permissions::PermissionMode;
use codiv_common::truncate::{truncate_tool_output, MAX_TOOL_OUTPUT_BYTES};
use codiv_common::types::AgentRole;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use super::error::classify_error;

type DynError = Box<dyn std::error::Error + Send + Sync>;

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
// AppConfig — unified configuration (permissions + model catalog)
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
    /// 2. `~/.codiv/models.toml` (backward compat — parsed as ModelCatalog)
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

/// Ensure a base URL ends with `/v1` (or `/v1/`).
/// If the user already supplied it, keep it; otherwise append it.
fn ensure_v1_suffix(url: &str) -> String {
    let trimmed = url.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        format!("{}/", trimmed)
    } else {
        format!("{}/v1/", trimmed)
    }
}

/// Strip a trailing `/v1` (or `/v1/`) from a base URL.
/// OpenAI SDK already includes `/v1` in the request path, so the base URL
/// must NOT contain it.
fn strip_v1_suffix(url: &str) -> String {
    let trimmed = url.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        trimmed.trim_end_matches("/v1").to_string()
    } else {
        trimmed.to_string()
    }
}

/// Sanitize a tool's JSON Schema for OpenAI strict mode compatibility.
/// OpenAI strict mode requires:
/// - All properties listed in `required`
/// - No unsupported keywords like `format`, `minimum`, `maximum`, `$schema`, `title`, `default`
fn sanitize_tool_schema_for_openai(mut tool: Tool) -> Tool {
    let mut val = tool.input_schema.to_value();
    sanitize_schema_value(&mut val);
    tool.input_schema = schemars::Schema::try_from(val).unwrap();
    tool
}

fn sanitize_schema_value(val: &mut serde_json::Value) {
    if let Some(obj) = val.as_object_mut() {
        // Remove unsupported top-level keywords
        obj.remove("$schema");
        obj.remove("title");

        // If there are properties, ensure all are in required
        if let Some(props) = obj.get("properties").cloned() {
            if let Some(prop_obj) = props.as_object() {
                let all_keys: Vec<serde_json::Value> = prop_obj
                    .keys()
                    .map(|k| serde_json::Value::String(k.clone()))
                    .collect();
                obj.insert("required".to_string(), serde_json::Value::Array(all_keys));
            }
        }

        // Clean each property's sub-schema
        if let Some(props) = obj.get_mut("properties") {
            if let Some(prop_obj) = props.as_object_mut() {
                for (_key, prop_schema) in prop_obj.iter_mut() {
                    if let Some(prop) = prop_schema.as_object_mut() {
                        prop.remove("format");
                        prop.remove("minimum");
                        prop.remove("maximum");
                        prop.remove("default");
                    }
                }
            }
        }
    }
}

fn build_anthropic_model(
    model_name: &str,
    provider_config: &ProviderConfig,
) -> Result<aisdk::providers::Anthropic<aisdk::core::DynamicModel>, DynError> {
    let ua_headers = std::collections::HashMap::from([
        ("user-agent".to_string(), "Claude Code".to_string()),
    ]);
    if provider_config.api_key.is_some() || provider_config.base_url.is_some() {
        let mut builder = Anthropic::builder().model_name(model_name);
        if let Some(ref key) = provider_config.api_key {
            builder = builder.api_key(key);
        } else {
            // Builder requires api_key; fall back to env var
            let key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
            builder = builder.api_key(key);
        }
        if let Some(ref url) = provider_config.base_url {
            builder = builder.base_url(ensure_v1_suffix(url));
        }
        builder = builder.headers(ua_headers);
        Ok(builder.build()?)
    } else {
        let key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
        Ok(Anthropic::builder()
            .model_name(model_name)
            .api_key(key)
            .headers(ua_headers)
            .build()?)
    }
}

/// Build an Anthropic model for Claude Code OAuth tokens.
///
/// Uses aisdk's `.use_oauth(true)` which sends `Authorization: Bearer {token}`
/// instead of `x-api-key`, and adds the required `anthropic-beta` flags.
fn build_claude_code_model(
    model_name: &str,
    provider_config: &ProviderConfig,
) -> Result<aisdk::providers::Anthropic<aisdk::core::DynamicModel>, DynError> {
    let api_key = provider_config
        .api_key
        .clone()
        .ok_or("claude_code provider requires an api_key (OAuth access token) in config")?;

    let headers = std::collections::HashMap::from([
        ("user-agent".to_string(), "Claude Code".to_string()),
        ("x-app".to_string(), "cli".to_string()),
        ("x-claude-code-session-id".to_string(), uuid::Uuid::new_v4().to_string()),
    ]);

    // Build metadata matching Claude Code's format.
    // The account_uuid is critical for the API to apply correct rate limits.
    let account_uuid = codiv_common::auth::read_oauth_account_uuid("claude_code").unwrap_or_default();
    let metadata = serde_json::json!({
        "metadata": {
            "user_id": serde_json::json!({
                "account_uuid": account_uuid,
            }).to_string()
        }
    });

    let model = Anthropic::builder()
        .model_name(model_name)
        .api_key(api_key)
        .use_oauth(true)
        .headers(headers)
        .body(metadata)
        .build()?;
    Ok(model)
}

fn build_openai_model(
    model_name: &str,
    provider_config: &ProviderConfig,
) -> Result<aisdk::providers::OpenAI<aisdk::core::DynamicModel>, DynError> {
    if provider_config.api_key.is_some() || provider_config.base_url.is_some() {
        let mut builder = OpenAI::builder().model_name(model_name);
        if let Some(ref key) = provider_config.api_key {
            builder = builder.api_key(key);
        } else {
            let key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
            builder = builder.api_key(key);
        }
        if let Some(ref url) = provider_config.base_url {
            builder = builder.base_url(strip_v1_suffix(url));
        }
        Ok(builder.build()?)
    } else {
        Ok(OpenAI::model_name(model_name))
    }
}

fn build_google_model(
    model_name: &str,
    provider_config: &ProviderConfig,
) -> Result<aisdk::providers::Google<aisdk::core::DynamicModel>, DynError> {
    if provider_config.api_key.is_some() || provider_config.base_url.is_some() {
        let mut builder = Google::builder().model_name(model_name);
        if let Some(ref key) = provider_config.api_key {
            builder = builder.api_key(key);
        } else {
            let key = std::env::var("GOOGLE_API_KEY").unwrap_or_default();
            builder = builder.api_key(key);
        }
        if let Some(ref url) = provider_config.base_url {
            builder = builder.base_url(url);
        }
        Ok(builder.build()?)
    } else {
        Ok(Google::model_name(model_name))
    }
}

fn build_vllm_model(
    model_name: &str,
    provider_config: &ProviderConfig,
) -> Result<aisdk::providers::Vllm<aisdk::core::DynamicModel>, DynError> {
    let base_url = provider_config.base_url.as_deref()
        .ok_or("vllm provider requires a base_url in config")?;
    let mut builder = Vllm::builder()
        .model_name(model_name)
        .base_url(base_url);
    if let Some(ref key) = provider_config.api_key {
        builder = builder.api_key(key);
    }
    Ok(builder.build()?)
}

fn build_openai_compatible_model(
    model_name: &str,
    provider_name: &str,
    provider_config: &ProviderConfig,
) -> Result<aisdk::providers::OpenAICompatible<aisdk::core::DynamicModel>, DynError> {
    let base_url = provider_config.base_url.as_deref()
        .ok_or_else(|| format!("provider '{}' requires a base_url in config", provider_name))?;
    let api_key = provider_config.api_key.clone()
        .ok_or_else(|| format!("provider '{}' requires an api_key in config", provider_name))?;

    let model = OpenAICompatible::<aisdk::core::DynamicModel>::builder()
        .provider_name(provider_name)
        .base_url(base_url)
        .api_key(api_key)
        .model_name(model_name)
        .build()?;
    Ok(model)
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
    let mut full_text = String::new();
    let mut chunk_count: u32 = 0;
    let mut collected_events: Vec<ConversationEvent> = Vec::new();
    let mut reasoning_buffer = String::new();
    let mut reasoning_start: Option<Instant> = None;
    let mut tool_call_names: HashMap<String, String> = HashMap::new();

    while let Some(chunk) = response.stream.next().await {
        chunk_count += 1;

        match chunk {
            LanguageModelStreamChunkType::TextDelta(text) => {
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
            LanguageModelStreamChunkType::ReasoningDelta(text) => {
                reasoning_start.get_or_insert(Instant::now());
                reasoning_buffer.push_str(&text);
                send_ipc(
                    tx,
                    &DaemonMessage::AgentStreamChunk {
                        request_id: request_id.to_string(),
                        chunk: StreamChunk::Reasoning(text),
                    },
                )
                .await?;
            }
            LanguageModelStreamChunkType::ToolCallDelta { id, delta } => {
                let tool_name = tool_call_names.get(&id).cloned().unwrap_or_default();
                send_ipc(
                    tx,
                    &DaemonMessage::AgentStreamChunk {
                        request_id: request_id.to_string(),
                        chunk: StreamChunk::ToolCallDelta { tool_call_id: id, tool_name, delta },
                    },
                )
                .await?;
            }
            LanguageModelStreamChunkType::ToolCallStart(info) => {
                // Flush pre-tool-call reasoning as its own event
                if !reasoning_buffer.is_empty() {
                    let duration = reasoning_start.map(|s| s.elapsed().as_secs_f32()).unwrap_or(0.0);
                    collected_events.push(ConversationEvent::AssistantReasoning {
                        request_id: request_id.to_string(),
                        text: std::mem::take(&mut reasoning_buffer),
                        duration_secs: duration,
                    });
                    reasoning_start = None;
                }
                // Register name so ToolCallDelta can include it in IPC
                tool_call_names.insert(info.id.clone(), info.name.clone());
            }
            LanguageModelStreamChunkType::ToolCallAvailable(info) => {
                let arguments = serde_json::to_string(&info.input).unwrap_or_default();
                collected_events.push(ConversationEvent::ToolCall {
                    request_id: request_id.to_string(),
                    tool_call_id: info.tool.id.clone(),
                    tool_name: info.tool.name.clone(),
                    arguments: arguments.clone(),
                });
                // For edit tools, capture the file content NOW (before the tool
                // executes) so the client can compute an accurate diff. Without
                // this, the client may read the file after the edit has already
                // been applied due to IPC batching.
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
                        request_id: request_id.to_string(),
                        chunk: StreamChunk::ToolCall {
                            name: info.tool.name,
                            arguments,
                            pre_edit_content,
                        },
                    },
                )
                .await?;
            }
            LanguageModelStreamChunkType::ToolCallEnd(info) => {
                let output = match &info.output {
                    Ok(v) => match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => serde_json::to_string(&other).unwrap_or_default(),
                    },
                    Err(e) => format!("Error: {e}"),
                };
                let output = truncate_tool_output(&output, MAX_TOOL_OUTPUT_BYTES);
                collected_events.push(ConversationEvent::ToolResult {
                    request_id: request_id.to_string(),
                    tool_call_id: info.tool.id.clone(),
                    tool_name: info.tool.name.clone(),
                    result: output.clone(),
                });
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
                return Err(err.to_string().into());
            }
            LanguageModelStreamChunkType::Incomplete(reason) => {
                tracing::warn!("stream incomplete: {reason}");
            }
            _ => {}
        }
    }

    // Flush accumulated reasoning into a single event.
    if !reasoning_buffer.is_empty() {
        let duration = reasoning_start.map(|s| s.elapsed().as_secs_f32()).unwrap_or(0.0);
        collected_events.push(ConversationEvent::AssistantReasoning {
            request_id: request_id.to_string(),
            text: reasoning_buffer,
            duration_secs: duration,
        });
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
    let cache_read_tokens = usage.cached_tokens.unwrap_or(0);

    Ok((full_text, input_tokens, output_tokens, cache_read_tokens, collected_events))
}

async fn send_ipc(tx: &mpsc::Sender<Vec<u8>>, msg: &DaemonMessage) -> Result<(), DynError> {
    let frame = frame_message(msg)?;
    tx.send(frame).await.map_err(|_| "ipc receiver closed")?;
    Ok(())
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
