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

fn run_oauth_flow(
    config: codiv_common::auth::OAuthConfig,
    provider_id: &str,
    display_name: &str,
) -> Result<()> {
    use codiv_common::auth::{
        build_anthropic_auth_url, build_standard_auth_url, exchange_anthropic_code,
        exchange_standard_code, token_response_to_oauth_tokens, write_oauth_tokens_to_config,
    };
    use dialoguer::Input;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("Failed to create async runtime")?;

    // Step 1: Build auth URL (sync — no runtime needed)
    let params = if config.use_pkce {
        build_anthropic_auth_url(&config)?
    } else {
        build_standard_auth_url(&config)?
    };

    // Step 2: Open browser and show URL
    println!("Opening browser for {} authentication...", display_name);
    println!("If the browser did not open, visit:");
    println!("  {}", params.auth_url);
    let _ = open::that(&params.auth_url);

    // Step 3: Prompt for authorization code (outside runtime — dialoguer uses blocking I/O)
    let auth_code = Input::<String>::new()
        .with_prompt("Paste the authorization code from the browser")
        .interact_text()
        .context("Failed to read authorization code")?;

    let auth_code = auth_code.trim().to_string();
    if auth_code.is_empty() {
        anyhow::bail!("Authorization code cannot be empty");
    }

    // Step 4: Exchange code for tokens
    let token_resp = rt
        .block_on(async {
            if config.use_pkce {
                exchange_anthropic_code(&config, &auth_code, params.code_verifier.as_deref()).await
            } else {
                exchange_standard_code(&config, &auth_code, None).await
            }
        })
        .with_context(|| format!("Token exchange failed for {}", display_name))?;

    // Step 5: Store tokens
    let tokens = token_response_to_oauth_tokens(token_resp);
    write_oauth_tokens_to_config(provider_id, &tokens)
        .with_context(|| format!("Failed to save tokens for {}", display_name))?;

    println!("Authentication successful for {}.", display_name);
    Ok(())
}
