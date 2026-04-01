//! `codiv login [provider]` subcommand.
//!
//! Drives interactive authentication for a provider. No IPC/daemon involved —
//! runs entirely in the codiv process, same as tool subcommands.

use anyhow::{Context, Result};
use clap::Args;
use codiv_common::auth::{
    provider_by_id, provider_registry, write_api_key_to_config, AuthMethod,
};

#[derive(Args)]
pub struct LoginArgs {
    /// Provider ID (anthropic, openai, claude_code, codex).
    /// If omitted, lists available providers.
    pub provider: Option<String>,
}

pub fn handle_login(args: LoginArgs) {
    if let Err(e) = run(args) {
        eprintln!("codiv login: {e}");
        std::process::exit(1);
    }
}

fn run(args: LoginArgs) -> Result<()> {
    let provider_id = match args.provider {
        Some(id) => id,
        None => {
            // No provider given — print usage with available providers
            println!("Usage: codiv login <provider>");
            println!();
            println!("Available providers:");
            for p in provider_registry() {
                println!("  {:<14}  {}", p.id, p.display_name);
            }
            return Ok(());
        }
    };

    let entry = provider_by_id(&provider_id).with_context(|| {
        format!(
            "Unknown provider '{}'. Run `codiv login` to see available providers.",
            provider_id
        )
    })?;

    // Use first supported auth method for the provider.
    let method = entry
        .auth_methods
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("Provider '{}' has no auth methods", provider_id))?;

    match method {
        AuthMethod::ApiKey => run_api_key_flow(&provider_id, entry.display_name),
        AuthMethod::OAuthCode(config) => run_oauth_flow(config, &provider_id, entry.display_name),
    }
}

fn run_api_key_flow(provider_id: &str, display_name: &str) -> Result<()> {
    use dialoguer::Password;

    let key = Password::new()
        .with_prompt(format!("Enter your {} API key", display_name))
        .interact()
        .context("Failed to read API key")?;

    let key = key.trim().to_string();
    if key.is_empty() {
        anyhow::bail!("API key cannot be empty");
    }

    // SECURITY: do NOT log or print the raw key
    write_api_key_to_config(provider_id, &key)
        .with_context(|| format!("Failed to save API key for {}", display_name))?;

    println!("API key saved for {}.", display_name);
    Ok(())
}

/// Detect if this provider uses a localhost callback (e.g. Codex on port 1455)
/// vs manual code paste (e.g. Anthropic/Claude Code).
fn uses_localhost_callback(config: &codiv_common::auth::OAuthConfig) -> bool {
    config
        .redirect_uri
        .as_ref()
        .map(|uri| uri.starts_with("http://localhost:") || uri.starts_with("http://127.0.0.1:"))
        .unwrap_or(false)
}

/// Determine if this is an Anthropic-style OAuth (state=verifier, JSON exchange)
/// vs standard OAuth (random state, form-encoded exchange).
fn is_anthropic_style(config: &codiv_common::auth::OAuthConfig) -> bool {
    config
        .auth_url
        .host_str()
        .map(|h| h.contains("claude.ai") || h.contains("anthropic.com"))
        .unwrap_or(false)
}

fn run_oauth_flow(
    config: codiv_common::auth::OAuthConfig,
    provider_id: &str,
    display_name: &str,
) -> Result<()> {
    use codiv_common::auth::{
        build_anthropic_auth_url, build_standard_auth_url, exchange_anthropic_code,
        exchange_standard_code, token_response_to_oauth_tokens, write_oauth_tokens_to_config,
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("Failed to create async runtime")?;

    let anthropic_style = is_anthropic_style(&config);
    let localhost_callback = uses_localhost_callback(&config);

    // Step 1: Build auth URL
    let params = if anthropic_style {
        build_anthropic_auth_url(&config)?
    } else {
        build_standard_auth_url(&config)?
    };

    // Step 2: Get the authorization code
    let auth_code = if localhost_callback {
        // Localhost callback flow (Codex): start server, open browser, wait for redirect
        run_localhost_callback_flow(&params.auth_url, &params.state, display_name)?
    } else {
        // Manual paste flow (Anthropic): open browser, user pastes code
        run_manual_paste_flow(&params.auth_url, display_name)?
    };

    // Step 3: Exchange code for tokens
    let token_resp = rt
        .block_on(async {
            if anthropic_style {
                exchange_anthropic_code(&config, &auth_code, params.code_verifier.as_deref()).await
            } else {
                exchange_standard_code(&config, &auth_code, params.code_verifier.as_deref()).await
            }
        })
        .with_context(|| format!("Token exchange failed for {}", display_name))?;

    // Step 4: Store tokens
    let tokens = token_response_to_oauth_tokens(token_resp);
    write_oauth_tokens_to_config(provider_id, &tokens)
        .with_context(|| format!("Failed to save tokens for {}", display_name))?;

    println!("Authentication successful for {}.", display_name);
    Ok(())
}

/// Manual paste flow: open browser, user copies code from redirect page and pastes it.
fn run_manual_paste_flow(auth_url: &str, display_name: &str) -> Result<String> {
    use dialoguer::Input;

    println!("Opening browser for {} authentication...", display_name);
    println!("If the browser did not open, visit:");
    println!("  {}", auth_url);
    let _ = open::that(auth_url);

    let auth_code = Input::<String>::new()
        .with_prompt("Paste the authorization code from the browser")
        .interact_text()
        .context("Failed to read authorization code")?;

    let auth_code = auth_code.trim().to_string();
    if auth_code.is_empty() {
        anyhow::bail!("Authorization code cannot be empty");
    }
    Ok(auth_code)
}

/// Localhost callback flow: start a local HTTP server, open browser, wait for
/// the OAuth redirect to deliver the authorization code automatically.
fn run_localhost_callback_flow(
    auth_url: &str,
    expected_state: &str,
    display_name: &str,
) -> Result<String> {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    // Bind to localhost:1455 (Codex callback port)
    let listener = TcpListener::bind("127.0.0.1:1455")
        .context("Failed to bind localhost:1455 for OAuth callback. Is another process using it?")?;

    println!("Opening browser for {} authentication...", display_name);
    println!("Waiting for authorization callback on http://localhost:1455 ...");
    println!("If the browser did not open, visit:");
    println!("  {}", auth_url);
    let _ = open::that(auth_url);

    // Wait for the callback request
    let (mut stream, _) = listener
        .accept()
        .context("Failed to accept OAuth callback connection")?;

    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .context("Failed to read callback request")?;

    // Parse GET /auth/callback?code=...&state=... HTTP/1.1
    let path = request_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("Invalid callback request"))?;

    let url = url::Url::parse(&format!("http://localhost{}", path))
        .context("Failed to parse callback URL")?;

    let code = url
        .query_pairs()
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.to_string())
        .ok_or_else(|| anyhow::anyhow!("No 'code' parameter in callback"))?;

    let state = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.to_string());

    // Validate state if present
    if let Some(ref s) = state {
        if s != expected_state {
            anyhow::bail!("State mismatch in OAuth callback — possible CSRF attack");
        }
    }

    // Send success response to browser
    let response_body = "<html><body><h2>Authentication successful!</h2><p>You can close this tab and return to the terminal.</p></body></html>";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_body.len(),
        response_body
    );
    let _ = stream.write_all(response.as_bytes());

    println!("Authorization code received.");
    Ok(code)
}
