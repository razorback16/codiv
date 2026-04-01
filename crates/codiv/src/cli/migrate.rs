//! `codiv migrate-env` subcommand.
//!
//! Detects API keys set via environment variables (ANTHROPIC_API_KEY,
//! OPENAI_API_KEY, GOOGLE_API_KEY) and migrates them into ~/.codiv/config.toml.
//! Environment variable values are NEVER printed in full — ApiKey Display
//! truncation is used for safe output.

use anyhow::Result;
use clap::Args;
use codiv_common::auth::{write_api_key_to_config, ApiKey};

#[derive(Args)]
pub struct MigrateEnvArgs {}

pub fn handle_migrate_env(_args: MigrateEnvArgs) {
    if let Err(e) = run() {
        eprintln!("codiv migrate-env: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    // Map of (env_var_name, provider_id, human_label)
    let candidates = [
        ("ANTHROPIC_API_KEY", "anthropic", "Anthropic"),
        ("OPENAI_API_KEY",    "openai",    "OpenAI"),
        ("GOOGLE_API_KEY",    "google",    "Google"),
    ];

    let mut migrated = 0usize;

    for (env_var, provider_id, label) in &candidates {
        let Ok(value) = std::env::var(env_var) else {
            continue; // Not set — skip silently
        };
        let value = value.trim().to_string();
        if value.is_empty() {
            continue;
        }

        write_api_key_to_config(provider_id, &value)
            .map_err(|e| anyhow::anyhow!("Failed to write API key for {label}: {e}"))?;

        // SECURITY: never print raw key — use ApiKey Display which truncates
        let display = ApiKey::new(value);
        println!("Migrated {label} key ({display}) → ~/.codiv/config.toml [providers.{provider_id}]");
        migrated += 1;
    }

    if migrated == 0 {
        println!("Nothing to migrate: ANTHROPIC_API_KEY, OPENAI_API_KEY, and GOOGLE_API_KEY are not set.");
    } else {
        println!("\nMigration complete. You may now unset the environment variables.");
    }

    Ok(())
}
