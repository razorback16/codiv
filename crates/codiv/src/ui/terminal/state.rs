use std::time::Instant;

/// Tracks a command that has been submitted to bash but hasn't completed yet.
pub(crate) struct PendingCommand {
    pub(crate) sentinel: String,
    pub(crate) accumulated: String,
    pub(crate) command: String,
    pub(crate) last_activity: Instant,
    pub(crate) needs_env_refresh: bool,
}

const ENV_MODIFIERS: &[&str] = &[
    "cd", "source", ".", "pushd", "popd", "export", "unset",
    "eval", "direnv", "nvm", "pyenv", "rbenv", "conda", "activate",
];

pub(crate) fn command_modifies_env(cmd: &str) -> bool {
    for segment in cmd.split([';', '|', '&']) {
        let first_word = segment.split_whitespace().next().unwrap_or("");
        let base = first_word.rsplit('/').next().unwrap_or(first_word);
        if ENV_MODIFIERS.contains(&base) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_cd() {
        assert!(command_modifies_env("cd /tmp"));
        assert!(command_modifies_env("cd"));
    }

    #[test]
    fn detects_source() {
        assert!(command_modifies_env("source ~/.bashrc"));
        assert!(command_modifies_env(". ~/.bashrc"));
    }

    #[test]
    fn detects_export() {
        assert!(command_modifies_env("export FOO=bar"));
    }

    #[test]
    fn detects_in_compound() {
        assert!(command_modifies_env("echo hi; cd /tmp"));
        assert!(command_modifies_env("make && cd build"));
    }

    #[test]
    fn normal_commands_false() {
        assert!(!command_modifies_env("ls -la"));
        assert!(!command_modifies_env("cargo build"));
        assert!(!command_modifies_env("git status"));
    }
}

/// Tracks a pending permission confirmation prompt from the daemon.
#[allow(dead_code)]
pub(crate) struct PendingConfirmation {
    pub(crate) request_id: String,
    pub(crate) description: String,
    pub(crate) risk: codiv_common::messages::RiskLevel,
    pub(crate) tool_name: String,
    pub(crate) tool_args: String,
    pub(crate) prompt_lines: u16,
    pub(crate) selected_index: usize,
    pub(crate) option_count: usize,
}

/// Tracks a pending session picker (inline list of saved sessions).
pub(crate) struct PendingSessionPicker {
    pub(crate) sessions: Vec<codiv_common::conversation::SessionInfo>,
    pub(crate) selected_index: usize,
    pub(crate) prompt_lines: u16,
}

/// Default scrollback limit (number of lines retained).
pub(crate) const MAX_SCROLLBACK: usize = 10_000;
