//! Command index: scans PATH directories for executable commands.
//! Also provides input classification for the shell dispatcher.

use std::collections::HashSet;
use std::os::unix::fs::PermissionsExt;

// ---------------------------------------------------------------------------
// CommandIndex
// ---------------------------------------------------------------------------

pub struct CommandIndex {
    index: HashSet<String>,
}

impl CommandIndex {
    pub fn new() -> Self {
        Self {
            index: HashSet::new(),
        }
    }

    /// Scan all directories listed in `$PATH` for executable files.
    /// First entry wins (mimics shell lookup order).
    pub fn scan_path_directories(&mut self) {
        let path_env = match std::env::var("PATH") {
            Ok(p) => p,
            Err(_) => return,
        };

        for dir in path_env.split(':') {
            if dir.is_empty() {
                continue;
            }

            let entries = match std::fs::read_dir(dir) {
                Ok(e) => e,
                Err(_) => continue,
            };

            for entry in entries.flatten() {
                let ft = match entry.file_type() {
                    Ok(ft) => ft,
                    Err(_) => continue,
                };

                if !ft.is_file() && !ft.is_symlink() {
                    continue;
                }

                let metadata = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                let mode = metadata.permissions().mode();
                // Check any execute bit (owner, group, others)
                if mode & 0o111 == 0 {
                    continue;
                }

                let name = entry.file_name().to_string_lossy().into_owned();
                self.index.insert(name);
            }
        }
    }

    /// Check whether `name` is a known command.
    pub fn is_known(&self, name: &str) -> bool {
        self.index.contains(name)
    }

    /// Insert shell builtin names into the index so they appear in tab completion.
    /// Detects the user's shell from `$SHELL` and adds the appropriate builtins.
    pub fn add_builtins(&mut self) {
        for name in shell_builtins() {
            self.index.insert(name.to_string());
        }
    }

}

impl Default for CommandIndex {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Input classification
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub enum InputAction {
    /// Known command — fast-pass to bash.
    Execute,
    /// Interactive program (vim, htop, ssh, …) — needs PTY passthrough.
    Interactive,
    /// AI query (prefixed with `?`).
    AiQuery,
    /// Unknown command — first word returned.
    NotFound(String),
    /// Clear screen and scrollback.
    Clear,
    /// Full session reset (screen, scrollback, history, completions).
    Reset,
    /// User wants to exit the shell.
    Exit,
    /// Empty input line.
    Empty,
}

/// Return builtins for the user's shell (detected from `$SHELL`).
/// Falls back to bash builtins if detection fails.
fn shell_builtins() -> HashSet<&'static str> {
    let shell = std::env::var("SHELL").unwrap_or_default();
    if shell.ends_with("/zsh") {
        zsh_builtins()
    } else {
        bash_builtins()
    }
}

/// Complete set of bash builtins from bash-builtins(7).
/// `exit`/`logout` excluded (handled as `InputAction::Exit`).
fn bash_builtins() -> HashSet<&'static str> {
    [
        ":", ".", "[", "alias", "bg", "bind", "break", "builtin", "cd", "command", "compgen",
        "complete", "compopt", "continue", "declare", "dirs", "disown", "echo", "enable", "eval",
        "exec", "export", "fc", "fg", "getopts", "hash", "help", "history", "jobs", "kill", "let",
        "local", "logout", "mapfile", "popd", "printf", "pushd", "pwd", "read", "readarray",
        "readonly", "return", "set", "shift", "shopt", "source", "suspend", "test", "times",
        "trap", "type", "typeset", "ulimit", "umask", "unalias", "unset", "wait", "caller",
    ]
    .into_iter()
    .collect()
}

/// Common ZSH builtins from zshbuiltins(1).
/// `exit`/`logout` excluded (handled as `InputAction::Exit`).
fn zsh_builtins() -> HashSet<&'static str> {
    [
        ":", ".", "[", "alias", "autoload", "bg", "bindkey", "break", "builtin", "cd", "chdir",
        "command", "compctl", "compadd", "compdef", "continue", "declare", "dirs", "disable",
        "disown", "echo", "echotc", "emulate", "enable", "eval", "exec", "export", "false", "fc",
        "fg", "float", "functions", "getln", "getopts", "hash", "history", "integer", "jobs",
        "kill", "let", "limit", "local", "log", "noglob", "popd", "print", "printf", "pushd",
        "pushln", "pwd", "read", "readonly", "rehash", "return", "sched", "set", "setopt", "shift",
        "source", "suspend", "test", "times", "trap", "true", "ttyctl", "type", "typeset",
        "ulimit", "umask", "unalias", "unfunction", "unhash", "unlimit", "unset", "unsetopt",
        "vared", "wait", "whence", "where", "which", "zcompile", "zle", "zmodload", "zparseopts",
        "zstyle",
    ]
    .into_iter()
    .collect()
}

/// Detect variable assignments (`NAME=VALUE` where NAME is `[A-Za-z_][A-Za-z0-9_]*`).
fn is_variable_assignment(first_word: &str) -> bool {
    if let Some(eq_pos) = first_word.find('=') {
        let name = &first_word[..eq_pos];
        if !name.is_empty() {
            let mut chars = name.chars();
            let first = chars.next().unwrap();
            if (first.is_ascii_alphabetic() || first == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return true;
            }
        }
    }
    false
}

/// Set of commands that require interactive / PTY passthrough handling.
///
/// Only includes REPLs and remote sessions that need a real TTY but do NOT
/// enter alternate screen mode. Fullscreen programs (vim, less, htop, man,
/// etc.) are handled automatically: they run through Execute and get
/// re-launched via Interactive when alternate screen mode is detected.
fn interactive_commands() -> HashSet<&'static str> {
    [
        "ssh", "tmux", "screen", "python", "python3", "node", "irb", "ghci",
        "claude", "ipython", "ruby", "lua", "R", "psql", "mysql", "sqlite3", "docker",
        "sudo",
    ]
    .into_iter()
    .collect()
}

/// Classify a raw input line into an [`InputAction`].
pub fn classify_input(input: &str, index: &CommandIndex) -> InputAction {
    let trimmed = input.trim();

    if trimmed.is_empty() {
        return InputAction::Empty;
    }

    if trimmed == "exit" || trimmed == "quit" {
        return InputAction::Exit;
    }

    if trimmed == "clear" {
        return InputAction::Clear;
    }

    if trimmed == "reset" {
        return InputAction::Reset;
    }

    if trimmed.starts_with('?') {
        return InputAction::AiQuery;
    }

    let first_word = trimmed.split_whitespace().next().unwrap_or("");

    if interactive_commands().contains(first_word) {
        return InputAction::Interactive;
    }

    if shell_builtins().contains(first_word) {
        return InputAction::Execute;
    }

    if is_variable_assignment(first_word) {
        return InputAction::Execute;
    }

    if index.is_known(first_word) {
        return InputAction::Execute;
    }

    InputAction::NotFound(first_word.to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a CommandIndex with only PATH scanning (no bash needed).
    fn path_only_index() -> CommandIndex {
        let mut idx = CommandIndex::new();
        idx.scan_path_directories();
        idx
    }

    #[test]
    fn scan_path_finds_common_commands() {
        let idx = path_only_index();
        assert!(
            idx.is_known("ls"),
            "ls should be found in PATH"
        );
        assert!(
            idx.is_known("echo"),
            "echo should be found in PATH"
        );
    }

    #[test]
    fn unknown_is_not_known() {
        let idx = path_only_index();
        assert!(
            !idx.is_known("xyzzy_definitely_not_a_command"),
            "nonsense name must not be known"
        );
    }

    #[test]
    fn classify_empty() {
        let idx = CommandIndex::new();
        assert_eq!(classify_input("", &idx), InputAction::Empty);
        assert_eq!(classify_input("   ", &idx), InputAction::Empty);
    }

    #[test]
    fn classify_exit() {
        let idx = CommandIndex::new();
        assert_eq!(classify_input("exit", &idx), InputAction::Exit);
        assert_eq!(classify_input("quit", &idx), InputAction::Exit);
    }

    #[test]
    fn classify_interactive() {
        let idx = CommandIndex::new();
        // REPLs and remote sessions are still explicitly Interactive.
        assert_eq!(classify_input("ssh user@host", &idx), InputAction::Interactive);
        assert_eq!(classify_input("python3", &idx), InputAction::Interactive);
        assert_eq!(classify_input("node", &idx), InputAction::Interactive);
    }

    #[test]
    fn classify_fullscreen_not_interactive() {
        let idx = CommandIndex::new();
        // Fullscreen programs (vim, htop, less, man) are no longer hardcoded
        // as Interactive — they go through Execute and get auto-detected via
        // alternate screen mode.
        assert_ne!(classify_input("vim foo.txt", &idx), InputAction::Interactive);
        assert_ne!(classify_input("htop", &idx), InputAction::Interactive);
        assert_ne!(classify_input("less foo.txt", &idx), InputAction::Interactive);
        assert_ne!(classify_input("man ls", &idx), InputAction::Interactive);
    }

    #[test]
    fn classify_ai_query() {
        let idx = CommandIndex::new();
        assert_eq!(classify_input("?what is rust", &idx), InputAction::AiQuery);
    }

    #[test]
    fn classify_known_command() {
        let idx = path_only_index();
        assert_eq!(classify_input("ls -la", &idx), InputAction::Execute);
    }

    #[test]
    fn classify_unknown() {
        let idx = CommandIndex::new();
        assert_eq!(
            classify_input("xyzzy_fake", &idx),
            InputAction::NotFound("xyzzy_fake".to_string())
        );
    }

    #[test]
    fn classify_builtin_cd() {
        let idx = CommandIndex::new();
        assert_eq!(classify_input("cd /tmp", &idx), InputAction::Execute);
    }

    #[test]
    fn classify_builtin_export() {
        let idx = CommandIndex::new();
        assert_eq!(classify_input("export FOO=bar", &idx), InputAction::Execute);
    }

    #[test]
    fn classify_builtin_source() {
        let idx = CommandIndex::new();
        assert_eq!(
            classify_input("source ~/.bashrc", &idx),
            InputAction::Execute
        );
        assert_eq!(classify_input(". ~/.bashrc", &idx), InputAction::Execute);
    }

    #[test]
    fn classify_variable_assignment() {
        let idx = CommandIndex::new();
        assert_eq!(classify_input("FOO=bar", &idx), InputAction::Execute);
    }

    #[test]
    fn classify_truly_unknown_still_not_found() {
        let idx = CommandIndex::new();
        assert_eq!(
            classify_input("frobnicate --all", &idx),
            InputAction::NotFound("frobnicate".to_string())
        );
    }

}
