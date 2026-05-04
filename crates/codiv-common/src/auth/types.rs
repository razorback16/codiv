use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

/// Newtype for API keys. Display truncates the value for safe logging.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ApiKey(String);

impl ApiKey {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ApiKey {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl std::fmt::Display for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let key = &self.0;
        if key.len() <= 20 {
            write!(f, "{}", key)
        } else {
            write!(f, "{}...{}", &key[..13], &key[key.len() - 4..])
        }
    }
}

/// Newtype for OAuth access tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccessToken(String);

impl AccessToken {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for AccessToken {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// Newtype for OAuth refresh tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RefreshToken(String);

impl RefreshToken {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for RefreshToken {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// OAuth tokens stored after a successful authentication flow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OAuthTokens {
    pub access_token: AccessToken,
    pub refresh_token: Option<RefreshToken>,
    pub expires_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
}

impl OAuthTokens {
    pub fn new(
        access_token: impl Into<AccessToken>,
        refresh_token: Option<impl Into<RefreshToken>>,
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            access_token: access_token.into(),
            refresh_token: refresh_token.map(|r| r.into()),
            expires_at,
            id_token: None,
        }
    }

    /// Returns true if the token is expired or will expire within `buffer`.
    pub fn needs_refresh(&self, buffer: chrono::Duration) -> bool {
        Utc::now() + buffer >= self.expires_at
    }

    /// Returns true if the token is currently expired.
    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }
}

/// Configuration for an OAuth 2.0 flow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthConfig {
    pub auth_url: Url,
    pub token_url: Url,
    pub client_id: String,
    pub scopes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirect_uri: Option<String>,
    #[serde(default)]
    pub use_pkce: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_refresh_url: Option<Url>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_auth_params: Option<HashMap<String, String>>,
}

/// Token response from an OAuth token exchange endpoint.
/// Used internally by auth flows before constructing OAuthTokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    /// Seconds until expiry (from token endpoint, may be absent).
    pub expires_in: Option<u64>,
    /// Absolute expiry (computed or from endpoint).
    pub expires_at: Option<DateTime<Utc>>,
    pub token_type: String,
    pub scope: Option<String>,
}

/// The authentication method a provider supports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    ApiKey,
    #[serde(rename = "oauth_code")]
    OAuthCode(Box<OAuthConfig>),
}

impl AuthMethod {
    /// Returns the OAuth config if this method is an OAuth code or device flow.
    pub fn oauth_config(&self) -> Option<&OAuthConfig> {
        match self {
            Self::OAuthCode(config) => Some(config),
            Self::ApiKey => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn oauth_tokens_needs_refresh_when_expiry_within_buffer() {
        let expires_at = Utc::now() + Duration::seconds(30);
        let tokens = OAuthTokens::new(
            AccessToken::new("tok"),
            None::<RefreshToken>,
            expires_at,
        );
        // buffer of 60s: now + 60s >= now + 30s → needs refresh
        assert!(tokens.needs_refresh(Duration::seconds(60)));
    }

    #[test]
    fn oauth_tokens_no_refresh_when_far_from_expiry() {
        let expires_at = Utc::now() + Duration::hours(1);
        let tokens = OAuthTokens::new(
            AccessToken::new("tok"),
            None::<RefreshToken>,
            expires_at,
        );
        // buffer of 5 min: now + 5m < now + 1h → no refresh needed
        assert!(!tokens.needs_refresh(Duration::minutes(5)));
    }

    #[test]
    fn oauth_tokens_is_expired_returns_true_past_expiry() {
        let expires_at = Utc::now() - Duration::seconds(1);
        let tokens = OAuthTokens::new(
            AccessToken::new("tok"),
            None::<RefreshToken>,
            expires_at,
        );
        assert!(tokens.is_expired());
    }

    #[test]
    fn api_key_display_truncates_long_keys() {
        let key = ApiKey::new("sk-ant-api03-xxxxxxxxxx12345");
        let display = format!("{}", key);
        assert!(display.contains("..."));
        assert!(display.ends_with("2345"));
    }

    #[test]
    fn api_key_display_short_key_unchanged() {
        let key = ApiKey::new("short-key");
        assert_eq!(format!("{}", key), "short-key");
    }

    #[test]
    fn auth_method_oauth_config_returns_some_for_oauth_code() {
        use url::Url;
        let config = OAuthConfig {
            auth_url: Url::parse("https://example.com/auth").unwrap(),
            token_url: Url::parse("https://example.com/token").unwrap(),
            client_id: "client-id".to_string(),
            scopes: vec!["openid".to_string()],
            redirect_uri: None,
            use_pkce: false,
            token_refresh_url: None,
            extra_auth_params: None,
        };
        let method = AuthMethod::OAuthCode(Box::new(config));
        assert!(method.oauth_config().is_some());
    }

    #[test]
    fn auth_method_oauth_config_returns_none_for_api_key() {
        assert!(AuthMethod::ApiKey.oauth_config().is_none());
    }
}
