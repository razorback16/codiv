use codiv_common::messages::RiskLevel;

/// Classify the risk level of a tool call based on tool name and arguments.
pub fn classify_risk(tool_name: &str, args: &serde_json::Value) -> RiskLevel {
    match tool_name {
        "read" | "glob" | "grep" => RiskLevel::Low,
        "write" => RiskLevel::Medium,
        "edit" => RiskLevel::Medium,
        "bash" => classify_bash_risk(args),
        _ => RiskLevel::Medium, // unknown tools default to Medium
    }
}

/// Check if a bash command is read-only (used for Manual mode allowlisting).
pub fn is_readonly_bash(command: &str) -> bool {
    super::ast_classifier::is_readonly(command)
}

fn classify_bash_risk(args: &serde_json::Value) -> RiskLevel {
    let command = args.get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    super::ast_classifier::classify_command(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn read_tools_are_low() {
        assert_eq!(classify_risk("read", &json!({})), RiskLevel::Low);
        assert_eq!(classify_risk("glob", &json!({})), RiskLevel::Low);
        assert_eq!(classify_risk("grep", &json!({})), RiskLevel::Low);
    }

    #[test]
    fn write_tools_are_medium() {
        assert_eq!(classify_risk("write", &json!({})), RiskLevel::Medium);
        assert_eq!(classify_risk("edit", &json!({})), RiskLevel::Medium);
    }

    #[test]
    fn bash_rm_rf_is_critical() {
        assert_eq!(classify_risk("bash", &json!({"command": "rm -rf ./build/"})), RiskLevel::Critical);
        assert_eq!(classify_risk("bash", &json!({"command": "rm -fr /tmp/test"})), RiskLevel::Critical);
    }

    #[test]
    fn bash_force_push_is_critical() {
        assert_eq!(classify_risk("bash", &json!({"command": "git push --force origin main"})), RiskLevel::Critical);
        assert_eq!(classify_risk("bash", &json!({"command": "git push -f origin main"})), RiskLevel::Critical);
    }

    #[test]
    fn bash_reset_hard_is_critical() {
        assert_eq!(classify_risk("bash", &json!({"command": "git reset --hard HEAD~3"})), RiskLevel::Critical);
    }

    #[test]
    fn bash_sudo_is_critical() {
        assert_eq!(classify_risk("bash", &json!({"command": "sudo apt update"})), RiskLevel::Critical);
    }

    #[test]
    fn bash_rm_is_high() {
        assert_eq!(classify_risk("bash", &json!({"command": "rm file.txt"})), RiskLevel::High);
    }

    #[test]
    fn bash_git_push_is_high() {
        assert_eq!(classify_risk("bash", &json!({"command": "git push origin feature"})), RiskLevel::High);
    }

    #[test]
    fn bash_install_is_high() {
        assert_eq!(classify_risk("bash", &json!({"command": "npm install express"})), RiskLevel::High);
        assert_eq!(classify_risk("bash", &json!({"command": "pip install requests"})), RiskLevel::High);
        assert_eq!(classify_risk("bash", &json!({"command": "cargo install ripgrep"})), RiskLevel::High);
    }

    #[test]
    fn bash_mv_is_high() {
        assert_eq!(classify_risk("bash", &json!({"command": "mv src/old.rs src/new.rs"})), RiskLevel::High);
    }

    #[test]
    fn bash_ls_is_low() {
        assert_eq!(classify_risk("bash", &json!({"command": "ls -la"})), RiskLevel::Low);
    }

    #[test]
    fn bash_cat_is_low() {
        assert_eq!(classify_risk("bash", &json!({"command": "cat README.md"})), RiskLevel::Low);
    }

    #[test]
    fn bash_cargo_build_is_low() {
        assert_eq!(classify_risk("bash", &json!({"command": "cargo build"})), RiskLevel::Low);
        assert_eq!(classify_risk("bash", &json!({"command": "cargo test"})), RiskLevel::Low);
    }

    #[test]
    fn bash_git_status_is_low() {
        assert_eq!(classify_risk("bash", &json!({"command": "git status"})), RiskLevel::Low);
        assert_eq!(classify_risk("bash", &json!({"command": "git log --oneline"})), RiskLevel::Low);
        assert_eq!(classify_risk("bash", &json!({"command": "git diff HEAD"})), RiskLevel::Low);
    }

    #[test]
    fn bash_unknown_command_is_medium() {
        assert_eq!(classify_risk("bash", &json!({"command": "python script.py"})), RiskLevel::Medium);
        assert_eq!(classify_risk("bash", &json!({"command": "node malicious.js"})), RiskLevel::Medium);
    }

    #[test]
    fn bash_docker_safe_subcommands_are_low() {
        assert_eq!(classify_risk("bash", &json!({"command": "docker ps"})), RiskLevel::Low);
        assert_eq!(classify_risk("bash", &json!({"command": "docker images"})), RiskLevel::Low);
    }

    #[test]
    fn readonly_bash_detection() {
        assert!(is_readonly_bash("ls -la"));
        assert!(is_readonly_bash("cat file.txt"));
        assert!(is_readonly_bash("git status"));
        assert!(is_readonly_bash("git log --oneline"));
        assert!(is_readonly_bash("git diff HEAD"));
        assert!(is_readonly_bash("pwd"));
        assert!(is_readonly_bash("echo hello"));
        assert!(is_readonly_bash("which rustc"));
        assert!(is_readonly_bash("env"));
        assert!(is_readonly_bash("head -5 file"));
        assert!(is_readonly_bash("tail -f log"));
        assert!(is_readonly_bash("find . -name '*.rs'"));
        assert!(is_readonly_bash("tree src/"));
        assert!(is_readonly_bash("file README.md"));
        assert!(is_readonly_bash("stat file.txt"));
        assert!(is_readonly_bash("wc -l file.txt"));

        assert!(!is_readonly_bash("rm file.txt"));
        assert!(!is_readonly_bash("cargo build"));
        assert!(!is_readonly_bash("npm install"));
        assert!(!is_readonly_bash("git push origin main"));
    }
}
