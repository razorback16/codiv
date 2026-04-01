mod tools;
pub mod login;
pub mod migrate;

use clap::Subcommand;

#[derive(Subcommand)]
pub enum Commands {
    /// Read file contents with line numbers
    Read(tools::ReadArgs),
    /// Write content to a file
    Write(tools::WriteArgs),
    /// Find and replace a unique string in a file
    Edit(tools::EditArgs),
    /// Find files matching a glob pattern
    Glob(tools::GlobArgs),
    /// Search file contents with regex
    Grep(tools::GrepArgs),
    /// Execute a bash command
    Bash(tools::BashArgs),
    /// Authenticate with an AI provider
    Login(login::LoginArgs),
    /// Migrate env var API keys (ANTHROPIC_API_KEY etc.) into config.toml
    MigrateEnv(migrate::MigrateEnvArgs),
}

pub fn dispatch(cmd: Commands) {
    match cmd {
        Commands::Login(args) => login::handle_login(args),
        Commands::MigrateEnv(args) => migrate::handle_migrate_env(args),
        other => tools::handle(other),
    }
}
