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

use super::storage::write_oauth_tokens_to_config;
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

/// Build a standard OAuth authorization URL (Codex — no PKCE).
pub fn build_standard_auth_url(config: &OAuthConfig) -> Result<AuthParams> {
    use oauth2::{AuthUrl, ClientId, CsrfToken, basic::BasicClient};
    let client = BasicClient::new(ClientId::new(config.client_id.clone()))
        .set_auth_uri(AuthUrl::new(config.auth_url.to_string())?);

    let mut request = client.authorize_url(CsrfToken::new_random);
    for scope in &config.scopes {
        request = request.add_scope(oauth2::Scope::new(scope.clone()));
    }
    if let Some(redirect_uri) = &config.redirect_uri {
        request = request.set_redirect_uri(
            std::borrow::Cow::Owned(oauth2::RedirectUrl::new(redirect_uri.clone())?)
        );
    }
    let (auth_url, csrf_state) = request.url();

    Ok(AuthParams {
        auth_url: auth_url.to_string(),
        state: csrf_state.secret().to_string(),
        code_verifier: None,
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
pub async fn exchange_standard_code(
    config: &OAuthConfig,
    code: &str,
    _verifier: Option<&str>, // Codex: no PKCE
) -> Result<OAuthTokenResponse> {
    use oauth2::{AuthorizationCode, AuthUrl, ClientId, TokenResponse, TokenUrl, basic::BasicClient};

    let client = BasicClient::new(ClientId::new(config.client_id.clone()))
        .set_auth_uri(AuthUrl::new(config.auth_url.to_string())?)
        .set_token_uri(TokenUrl::new(config.token_url.to_string())?);

    let http_client = reqwest::Client::new();
    let token = client
        .exchange_code(AuthorizationCode::new(code.to_string()))
        .request_async(&http_client)
        .await
        .map_err(|e| anyhow::anyhow!("Token exchange failed: {e}"))?;

    Ok(OAuthTokenResponse {
        access_token: token.access_token().secret().clone(),
        refresh_token: token.refresh_token().map(|t| t.secret().clone()),
        expires_in: token.expires_in().map(|d| d.as_secs()),
        expires_at: None,
        token_type: "Bearer".to_string(),
        scope: None,
    })
}

/// Convert an OAuthTokenResponse into OAuthTokens, computing expiry.
///
/// Fallback: if expires_in is absent, assumes 1 hour from now.
pub fn token_response_to_oauth_tokens(resp: OAuthTokenResponse) -> OAuthTokens {
    let expires_at = resp
        .expires_at
        .or_else(|| {
            resp.expires_in
                .map(|secs| Utc::now() + Duration::seconds(secs as i64))
        })
        .unwrap_or_else(|| Utc::now() + Duration::hours(1));

    OAuthTokens::new(
        AccessToken::new(resp.access_token),
        resp.refresh_token.map(RefreshToken::new),
        expires_at,
    )
}

/// Run the full OAuth code flow for a provider.
///
/// This function handles only the HTTP/PKCE layer — NOT interactive prompts.
/// The caller (login CLI) must:
///   1. Display the auth_url to the user (returned via the `on_url` callback)
///   2. Prompt the user to paste the authorization code
///   3. Pass the code to this function
///
/// Returns the stored OAuthTokens on success.
pub async fn run_oauth_code_flow(
    config: &OAuthConfig,
    provider_id: &str,
    on_url: impl FnOnce(&str),
    code: &str,
) -> Result<OAuthTokens> {
    // Build auth URL and invoke callback so caller can display/open it
    let params = if config.use_pkce {
        build_anthropic_auth_url(config)?
    } else {
        build_standard_auth_url(config)?
    };
    on_url(&params.auth_url);

    // Exchange the code for tokens
    let token_resp = if config.use_pkce {
        exchange_anthropic_code(config, code, params.code_verifier.as_deref()).await?
    } else {
        exchange_standard_code(config, code, None).await?
    };

    let tokens = token_response_to_oauth_tokens(token_resp);
    write_oauth_tokens_to_config(provider_id, &tokens)?;
    Ok(tokens)
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
            custom_headers: None,
            extra_auth_params: Some({
                let mut m = HashMap::new();
                m.insert("code".to_string(), "true".to_string());
                m
            }),
        }
    }

    fn codex_config() -> OAuthConfig {
        OAuthConfig {
            auth_url: Url::parse("https://auth.openai.com/api/accounts/deviceauth/usercode")
                .unwrap(),
            token_url: Url::parse("https://auth.openai.com/oauth/token").unwrap(),
            client_id: "app_EMoamEEZ73f0CkXaXp7hrann".to_string(),
            scopes: vec!["openid".to_string()],
            redirect_uri: None,
            use_pkce: false,
            token_refresh_url: None,
            custom_headers: None,
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
    fn standard_auth_url_has_no_code_verifier() {
        let config = codex_config();
        let params = build_standard_auth_url(&config).unwrap();
        assert!(
            params.code_verifier.is_none(),
            "Codex should not have PKCE verifier"
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
        let tokens = token_response_to_oauth_tokens(resp);
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
        let tokens = token_response_to_oauth_tokens(resp);
        let now = Utc::now();
        let diff = tokens.expires_at - now;
        assert!(diff.num_minutes() >= 59 && diff.num_minutes() <= 61);
    }
}
