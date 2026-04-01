mod tools;
pub mod login;

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
}

pub fn dispatch(cmd: Commands) {
    match cmd {
        Commands::Login(args) => login::handle_login(args),
        other => tools::handle(other),
    }
}
