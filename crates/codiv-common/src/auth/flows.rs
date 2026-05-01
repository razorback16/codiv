//! OAuth code flow HTTP adapters for Anthropic (PKCE with state=verifier quirk)
//! and standard OAuth2 (Codex).
//!
//! Ported from:
//!   forgecode/crates/forge_infra/src/auth/http/anthropic.rs
//!   forgecode/crates/forge_infra/src/auth/http/standard.rs
//!
//! Key constraint: Anthropic token exchange uses JSON body (not form-encoded).
//! DO NOT use oauth2::BasicClient::exchange_code() for Anthropic — it sends form data.

use anyhow::Result;
use chrono::{Duration, Utc};
use oauth2::PkceCodeChallenge;
use serde::Serialize;

use super::types::{AccessToken, OAuthConfig, OAuthTokenResponse, OAuthTokens, RefreshToken};

/// Parameters produced by build_*_auth_url functions.
#[derive(Debug, Clone)]
pub struct AuthParams {
    /// The full authorization URL to open in the browser.
    pub auth_url: String,
    /// For Anthropic: equals code_verifier (state=verifier quirk).
    /// For standard OAuth: a random CSRF state token.
    pub state: String,
    /// PKCE code verifier, present only when use_pkce=true.
    pub code_verifier: Option<String>,
}

/// Build the Anthropic OAuth authorization URL.
///
/// CRITICAL quirk: state = PKCE code_verifier (not a random CSRF token).
/// Source: forgecode AnthropicHttpProvider.build_auth_url()
pub fn build_anthropic_auth_url(config: &OAuthConfig) -> Result<AuthParams> {
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let verifier_secret = verifier.secret().to_string();

    let mut url = config.auth_url.clone();
    {
        let mut pairs = url.query_pairs_mut();
        pairs
            .append_pair("client_id", &config.client_id)
            .append_pair("response_type", "code")
            .append_pair("scope", &config.scopes.join(" "))
            .append_pair("code_challenge", challenge.as_str())
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &verifier_secret); // Non-standard: state = verifier

        if let Some(redirect_uri) = &config.redirect_uri {
            pairs.append_pair("redirect_uri", redirect_uri);
        }
        if let Some(extra) = &config.extra_auth_params {
            for (k, v) in extra {
                pairs.append_pair(k, v);
            }
        }
    }

    Ok(AuthParams {
        auth_url: url.to_string(),
        state: verifier_secret.clone(),
        code_verifier: Some(verifier_secret),
    })
}

/// Build a standard OAuth authorization URL with PKCE (Codex).
///
/// Unlike Anthropic, state is a random CSRF token (standard behavior).
/// PKCE verifier is generated and returned for the token exchange step.
pub fn build_standard_auth_url(config: &OAuthConfig) -> Result<AuthParams> {
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let verifier_secret = verifier.secret().to_string();

    let state = {
        use rand::Rng;
        let bytes: [u8; 16] = rand::thread_rng().gen();
        hex::encode(bytes)
    };

    let mut url = config.auth_url.clone();
    {
        let mut pairs = url.query_pairs_mut();
        pairs
            .append_pair("response_type", "code")
            .append_pair("client_id", &config.client_id)
            .append_pair("scope", &config.scopes.join(" "))
            .append_pair("code_challenge", challenge.as_str())
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state);

        if let Some(redirect_uri) = &config.redirect_uri {
            pairs.append_pair("redirect_uri", redirect_uri);
        }
        if let Some(extra) = &config.extra_auth_params {
            for (k, v) in extra {
                pairs.append_pair(k, v);
            }
        }
    }

    Ok(AuthParams {
        auth_url: url.to_string(),
        state,
        code_verifier: Some(verifier_secret),
    })
}

/// Custom JSON body for Anthropic token exchange.
/// Anthropic deviates from RFC 6749: expects application/json, not form-encoded.
#[derive(Serialize)]
struct AnthropicTokenRequest {
    code: String,
    state: String,
    grant_type: String,
    client_id: String,
    redirect_uri: Option<String>,
    code_verifier: String,
}

/// Exchange an authorization code for tokens using Anthropic's custom JSON endpoint.
///
/// DO NOT replace with oauth2::BasicClient::exchange_code() — Anthropic rejects form-encoded bodies.
pub async fn exchange_anthropic_code(
    config: &OAuthConfig,
    code: &str,
    verifier: Option<&str>,
) -> Result<OAuthTokenResponse> {
    let verifier = verifier
        .ok_or_else(|| anyhow::anyhow!("PKCE verifier required for Anthropic token exchange"))?;

    // Anthropic callback appends state after '#' in the pasted code: "code#state"
    // Split and use the embedded state if present, otherwise fall back to verifier.
    let (actual_code, state) = if code.contains('#') {
        let parts: Vec<&str> = code.split('#').collect();
        (
            parts[0].to_string(),
            parts.get(1).map(|s| s.to_string()),
        )
    } else {
        (code.to_string(), None)
    };

    let body = AnthropicTokenRequest {
        code: actual_code,
        state: state.unwrap_or_else(|| verifier.to_string()),
        grant_type: "authorization_code".to_string(),
        client_id: config.client_id.clone(),
        redirect_uri: config.redirect_uri.clone(),
        code_verifier: verifier.to_string(),
    };

    let client = reqwest::Client::new();
    let resp = client
        .post(config.token_url.as_str())
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Anthropic token exchange failed {status}: {text}");
    }

    Ok(resp.json::<OAuthTokenResponse>().await?)
}

/// Exchange an authorization code using the standard OAuth2 form-encoded flow (Codex).
///
/// Sends form-encoded body with PKCE code_verifier and redirect_uri.
pub async fn exchange_standard_code(
    config: &OAuthConfig,
    code: &str,
    verifier: Option<&str>,
) -> Result<OAuthTokenResponse> {
    let client = reqwest::Client::new();

    let mut form = vec![
        ("grant_type", "authorization_code".to_string()),
        ("client_id", config.client_id.clone()),
        ("code", code.to_string()),
    ];

    if let Some(v) = verifier {
        form.push(("code_verifier", v.to_string()));
    }
    if let Some(redirect_uri) = &config.redirect_uri {
        form.push(("redirect_uri", redirect_uri.clone()));
    }

    let resp = client
        .post(config.token_url.as_str())
        .form(&form)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Token exchange failed {status}: {text}");
    }

    Ok(resp.json::<OAuthTokenResponse>().await?)
}

/// Convert an OAuthTokenResponse into OAuthTokens, computing expiry.
///
/// Fallback: if expires_in is absent, assumes 1 hour from now.
/// When `old_refresh_token` is provided and the response omits a refresh token,
/// the old one is carried forward (prevents silent token loss on refresh).
pub fn token_response_to_oauth_tokens(
    resp: OAuthTokenResponse,
    old_refresh_token: Option<String>,
) -> OAuthTokens {
    let expires_at = resp
        .expires_at
        .or_else(|| {
            resp.expires_in
                .map(|secs| Utc::now() + Duration::seconds(secs as i64))
        })
        .unwrap_or_else(|| Utc::now() + Duration::hours(1));

    let refresh_token = resp
        .refresh_token
        .or(old_refresh_token)
        .map(RefreshToken::new);

    OAuthTokens::new(
        AccessToken::new(resp.access_token),
        refresh_token,
        expires_at,
    )
}

/// Refresh an OAuth token using the refresh_token grant.
///
/// Tries token_refresh_url first; falls back to token_url (same endpoint for Anthropic and Codex).
/// Uses form-encoded body (standard) — Anthropic's refresh endpoint accepts form-encoded unlike
/// the authorization code exchange endpoint which requires JSON.
///
/// Returns Ok(new_tokens) on success, Err on network or parse failure.
/// The caller is responsible for writing the new tokens to storage.
pub async fn refresh_oauth_token(
    config: &OAuthConfig,
    refresh_token: &str,
) -> Result<OAuthTokens> {
    let endpoint = config
        .token_refresh_url
        .as_ref()
        .unwrap_or(&config.token_url);

    let client = reqwest::Client::new();
    let form = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh_token.to_string()),
        ("client_id", config.client_id.clone()),
    ];

    let resp = client
        .post(endpoint.as_str())
        .form(&form)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Token refresh failed {status}: {text}");
    }

    let token_resp = resp.json::<OAuthTokenResponse>().await?;
    // Carry forward the old refresh token if the server omits one in the response
    Ok(token_response_to_oauth_tokens(token_resp, Some(refresh_token.to_string())))
}

/// Check stored OAuth tokens for `provider_id`. If they will expire within 5 minutes,
/// refresh them using the stored refresh_token and write the new tokens to config.toml.
///
/// Returns Ok(Some(new_access_token)) if refresh happened, Ok(None) if not needed or not possible.
/// Never propagates errors — logs warnings instead so callers can fall through gracefully.
pub async fn maybe_refresh_stored_token(provider_id: &str, config: &OAuthConfig) -> Option<String> {
    let tokens = match super::storage::read_oauth_tokens_from_config(provider_id) {
        Ok(Some(t)) => t,
        _ => return None,
    };

    if !tokens.needs_refresh(chrono::Duration::minutes(1)) {
        return None;
    }

    let refresh_token = match &tokens.refresh_token {
        Some(rt) => rt.as_str().to_string(),
        None => return None,
    };

    match refresh_oauth_token(config, &refresh_token).await {
        Ok(new_tokens) => {
            let new_access = new_tokens.access_token.as_str().to_string();
            if let Err(e) = super::storage::write_oauth_tokens_to_config(provider_id, &new_tokens) {
                tracing::warn!("failed to write refreshed tokens for {provider_id}: {e}");
            }
            Some(new_access)
        }
        Err(e) => {
            tracing::warn!("token refresh failed for {provider_id}: {e}");
            None
        }
    }
}

/// Fetch OAuth profile from Anthropic API to retrieve account_uuid.
///
/// Used after successful OAuth login to get the account UUID needed for
/// rate limit attribution in API requests.
pub async fn fetch_oauth_profile_uuid(access_token: &str) -> Option<String> {
    let client = reqwest::Client::new();
    let resp = client
        .get("https://api.anthropic.com/api/oauth/profile")
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let body: serde_json::Value = resp.json().await.ok()?;
    body.get("account")
        .and_then(|a| a.get("uuid"))
        .and_then(|u| u.as_str())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    fn claude_code_config() -> OAuthConfig {
        use std::collections::HashMap;
        OAuthConfig {
            auth_url: Url::parse("https://claude.ai/oauth/authorize").unwrap(),
            token_url: Url::parse("https://console.anthropic.com/v1/oauth/token").unwrap(),
            client_id: "9d1c250a-e61b-44d9-88ed-5944d1962f5e".to_string(),
            scopes: vec!["openid".to_string()],
            redirect_uri: Some("https://console.anthropic.com/oauth/code/callback".to_string()),
            use_pkce: true,
            token_refresh_url: None,
            extra_auth_params: Some({
                let mut m = HashMap::new();
                m.insert("code".to_string(), "true".to_string());
                m
            }),
        }
    }

    fn codex_config() -> OAuthConfig {
        OAuthConfig {
            auth_url: Url::parse("https://auth.openai.com/oauth/authorize").unwrap(),
            token_url: Url::parse("https://auth.openai.com/oauth/token").unwrap(),
            client_id: "app_EMoamEEZ73f0CkXaXp7hrann".to_string(),
            scopes: vec![
                "openid".to_string(),
                "profile".to_string(),
                "email".to_string(),
                "offline_access".to_string(),
            ],
            redirect_uri: Some("http://localhost:1455/auth/callback".to_string()),
            use_pkce: true,
            token_refresh_url: None,
            extra_auth_params: None,
        }
    }

    #[test]
    fn anthropic_auth_url_state_equals_code_verifier() {
        let config = claude_code_config();
        let params = build_anthropic_auth_url(&config).unwrap();
        // state must equal code_verifier (Anthropic non-standard PKCE quirk)
        assert_eq!(params.state, params.code_verifier.as_deref().unwrap());
    }

    #[test]
    fn anthropic_auth_url_has_state_query_param() {
        let config = claude_code_config();
        let params = build_anthropic_auth_url(&config).unwrap();
        let url = Url::parse(&params.auth_url).unwrap();
        let state_param = url
            .query_pairs()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.to_string());
        assert_eq!(state_param.as_deref(), Some(params.state.as_str()));
    }

    #[test]
    fn anthropic_auth_url_includes_extra_auth_params() {
        let config = claude_code_config();
        let params = build_anthropic_auth_url(&config).unwrap();
        // extra_auth_params has "code=true"
        assert!(
            params.auth_url.contains("code=true"),
            "Missing extra_auth_params in URL"
        );
    }

    #[test]
    fn anthropic_auth_url_has_pkce_challenge() {
        let config = claude_code_config();
        let params = build_anthropic_auth_url(&config).unwrap();
        assert!(
            params.auth_url.contains("code_challenge="),
            "Missing code_challenge"
        );
        assert!(
            params.auth_url.contains("code_challenge_method=S256"),
            "Missing S256 method"
        );
    }

    #[test]
    fn standard_auth_url_has_pkce_verifier() {
        let config = codex_config();
        let params = build_standard_auth_url(&config).unwrap();
        assert!(
            params.code_verifier.is_some(),
            "Codex should have PKCE verifier"
        );
        // State should NOT equal verifier (unlike Anthropic)
        assert_ne!(
            params.state,
            params.code_verifier.as_deref().unwrap(),
            "Standard OAuth state should be random, not equal to verifier"
        );
    }

    #[test]
    fn standard_auth_url_includes_extra_params() {
        use std::collections::HashMap;
        let mut config = codex_config();
        config.extra_auth_params = Some({
            let mut m = HashMap::new();
            m.insert("originator".to_string(), "codex_cli_rs".to_string());
            m
        });
        let params = build_standard_auth_url(&config).unwrap();
        assert!(
            params.auth_url.contains("originator=codex_cli_rs"),
            "Missing extra auth params in URL"
        );
    }

    #[test]
    fn token_response_expiry_falls_back_to_one_hour() {
        let resp = OAuthTokenResponse {
            access_token: "tok".to_string(),
            refresh_token: None,
            expires_in: None,
            expires_at: None,
            token_type: "Bearer".to_string(),
            scope: None,
        };
        let tokens = token_response_to_oauth_tokens(resp, None);
        let now = Utc::now();
        let diff = tokens.expires_at - now;
        // Should be approximately 1 hour (within 5 seconds of test execution)
        assert!(
            diff.num_minutes() >= 59 && diff.num_minutes() <= 61,
            "Expiry should be ~1 hour from now, got {} minutes",
            diff.num_minutes()
        );
    }

    #[test]
    fn token_response_uses_expires_in_when_present() {
        let resp = OAuthTokenResponse {
            access_token: "tok".to_string(),
            refresh_token: None,
            expires_in: Some(3600),
            expires_at: None,
            token_type: "Bearer".to_string(),
            scope: None,
        };
        let tokens = token_response_to_oauth_tokens(resp, None);
        let now = Utc::now();
        let diff = tokens.expires_at - now;
        assert!(diff.num_minutes() >= 59 && diff.num_minutes() <= 61);
    }
}
