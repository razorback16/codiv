//! Command index: scans PATH directories, bash builtins, aliases, and functions.
//! Also provides input classification for the shell dispatcher.

use std::collections::{HashMap, HashSet};
use std::os::unix::fs::PermissionsExt;

use super::bash_coprocess::BashCoprocess;

// ---------------------------------------------------------------------------
// CommandType / CommandInfo
// ---------------------------------------------------------------------------

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandType {
    Executable,
    Builtin,
    Alias,
    Function,
    Unknown,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct CommandInfo {
    pub cmd_type: CommandType,
    pub path: String,
    pub description: String,
}

// ---------------------------------------------------------------------------
// CommandIndex
// ---------------------------------------------------------------------------

pub struct CommandIndex {
    index: HashMap<String, CommandInfo>,
}

#[allow(dead_code)]
impl CommandIndex {
    pub fn new() -> Self {
        Self {
            index: HashMap::new(),
        }
    }

    /// Build the full index: scan PATH, then query bash for builtins, aliases,
    /// and functions.
    pub fn build(&mut self, bash: &BashCoprocess) {
        self.index.clear();
        self.scan_path_directories();
        self.query_builtins(bash);
        self.query_aliases(bash);
        self.query_functions(bash);
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
                // First entry wins — do not overwrite
                if !self.index.contains_key(&name) {
                    self.index.insert(
                        name,
                        CommandInfo {
                            cmd_type: CommandType::Executable,
                            path: entry.path().to_string_lossy().into_owned(),
                            description: String::new(),
                        },
                    );
                }
            }
        }
    }

    fn query_builtins(&mut self, bash: &BashCoprocess) {
        let result = bash.execute("compgen -b", 30000);
        let output = result.output;

        for line in output.lines() {
            let name = line.trim_end_matches(['\r', '\n']);
            if name.is_empty() {
                continue;
            }
            // Builtins override executables (same as C++ version)
            self.index.insert(
                name.to_string(),
                CommandInfo {
                    cmd_type: CommandType::Builtin,
                    path: String::new(),
                    description: "shell builtin".to_string(),
                },
            );
        }
    }

    fn query_aliases(&mut self, bash: &BashCoprocess) {
        let result = bash.execute("alias 2>/dev/null", 30000);
        let output = result.output;

        for line in output.lines() {
            let line = line.trim_end_matches(['\r', '\n']);
            // Lines look like: alias name='value'
            let rest = match line.find("alias ") {
                Some(pos) => &line[pos + 6..],
                None => continue,
            };

            let eq = match rest.find('=') {
                Some(pos) => pos,
                None => continue,
            };

            let name = &rest[..eq];
            let mut value = rest[eq + 1..].to_string();

            // Strip surrounding single quotes
            if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
                value = value[1..value.len() - 1].to_string();
            }

            self.index.insert(
                name.to_string(),
                CommandInfo {
                    cmd_type: CommandType::Alias,
                    path: String::new(),
                    description: value,
                },
            );
        }
    }

    fn query_functions(&mut self, bash: &BashCoprocess) {
        let result = bash.execute("compgen -A function", 30000);
        let output = result.output;

        for line in output.lines() {
            let name = line.trim_end_matches(['\r', '\n']);
            if name.is_empty() {
                continue;
            }
            // Functions only added if not already present
            if !self.index.contains_key(name) {
                self.index.insert(
                    name.to_string(),
                    CommandInfo {
                        cmd_type: CommandType::Function,
                        path: String::new(),
                        description: "shell function".to_string(),
                    },
                );
            }
        }
    }

    /// Check whether `name` is a known command.
    pub fn is_known(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    /// Look up detailed information for a command.
    pub fn lookup(&self, name: &str) -> Option<&CommandInfo> {
        self.index.get(name)
    }

    /// Return all command names that start with `prefix`, sorted alphabetically.
    pub fn complete(&self, prefix: &str) -> Vec<String> {
        let mut results: Vec<String> = self
            .index
            .keys()
            .filter(|name| prefix.is_empty() || name.starts_with(prefix))
            .cloned()
            .collect();
        results.sort();
        results
    }

    /// Number of indexed commands.
    pub fn size(&self) -> usize {
        self.index.len()
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
    /// User wants to exit the shell.
    Exit,
    /// Empty input line.
    Empty,
}

/// Set of commands that require interactive / PTY passthrough handling.
fn interactive_commands() -> HashSet<&'static str> {
    [
        "vim", "vi", "nvim", "nano", "emacs", "htop", "top", "less", "more", "man", "ssh",
        "tmux", "screen", "python", "python3", "node", "irb", "ghci",
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

    if trimmed.starts_with('?') {
        return InputAction::AiQuery;
    }

    let first_word = trimmed.split_whitespace().next().unwrap_or("");

    if interactive_commands().contains(first_word) {
        return InputAction::Interactive;
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
            "ls should be found in PATH (index size: {})",
            idx.size()
        );
        assert!(
            idx.is_known("echo"),
            "echo should be found in PATH (index size: {})",
            idx.size()
        );
    }

    #[test]
    fn complete_prefix() {
        let idx = path_only_index();
        let results = idx.complete("gi");
        assert!(
            results.contains(&"git".to_string()),
            "complete(\"gi\") should contain \"git\", got: {:?}",
            results
        );
        let mut sorted = results.clone();
        sorted.sort();
        assert_eq!(results, sorted, "complete() results must be sorted");
    }

    #[test]
    fn unknown_returns_none() {
        let idx = path_only_index();
        assert!(
            idx.lookup("xyzzy_definitely_not_a_command").is_none(),
            "lookup of a nonsense name must return None"
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
        assert_eq!(
            classify_input("vim foo.txt", &idx),
            InputAction::Interactive
        );
        assert_eq!(classify_input("htop", &idx), InputAction::Interactive);
        assert_eq!(classify_input("ssh user@host", &idx), InputAction::Interactive);
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
}
