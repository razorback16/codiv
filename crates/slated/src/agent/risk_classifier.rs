use slate_common::messages::RiskLevel;
use std::sync::LazyLock;
use regex::Regex;

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
    // Read-only commands that are safe in Manual mode
    static READONLY_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:ls|cat|head|tail|git\s+(?:status|log|diff|branch|show)|pwd|echo|which|env|find|tree|file|stat|wc|date|uname|hostname|whoami|id|groups|df|du|free|uptime|printenv|realpath|basename|dirname|readlink|test|true|false|\[)(?:\s|$)").unwrap()
    });
    let trimmed = command.trim();
    READONLY_PATTERN.is_match(trimmed)
}

fn classify_bash_risk(args: &serde_json::Value) -> RiskLevel {
    let command = args.get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if command.trim().is_empty() {
        return RiskLevel::Low;
    }

    // Critical patterns - destructive/irreversible operations
    static CRITICAL_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| vec![
        Regex::new(r"rm\s+-rf\s").unwrap(),
        Regex::new(r"rm\s+-fr\s").unwrap(),
        Regex::new(r"rm\s+(-[a-z]*r[a-z]*\s+)?(-[a-z]*f[a-z]*\s+)").unwrap(),
        Regex::new(r"git\s+push\s+--force").unwrap(),
        Regex::new(r"git\s+push\s+-f\b").unwrap(),
        Regex::new(r"git\s+reset\s+--hard").unwrap(),
        Regex::new(r"(?i)DROP\s+(TABLE|DATABASE)").unwrap(),
        Regex::new(r"(?i)TRUNCATE\s").unwrap(),
        Regex::new(r"sudo\s+").unwrap(),
        Regex::new(r"\bsu\s+-").unwrap(),
        Regex::new(r"mkfs\.").unwrap(),
        Regex::new(r"dd\s+if=").unwrap(),
        Regex::new(r">\s*/dev/").unwrap(),
        Regex::new(r"chmod\s+-R\s").unwrap(),
        Regex::new(r"chown\s+-R\s").unwrap(),
    ]);

    // High patterns - potentially dangerous but not necessarily destructive
    static HIGH_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| vec![
        Regex::new(r"\brm\s+").unwrap(),
        Regex::new(r"git\s+push\b").unwrap(),
        Regex::new(r"git\s+reset\b").unwrap(),
        Regex::new(r"git\s+clean\b").unwrap(),
        Regex::new(r"git\s+checkout\s+--\s").unwrap(),
        Regex::new(r"git\s+restore\s").unwrap(),
        Regex::new(r"chmod\s+777").unwrap(),
        Regex::new(r"curl.*-X\s*(POST|PUT|DELETE|PATCH)").unwrap(),
        Regex::new(r"wget\s").unwrap(),
        Regex::new(r"pip\s+install\b").unwrap(),
        Regex::new(r"npm\s+install\b").unwrap(),
        Regex::new(r"cargo\s+install\b").unwrap(),
        Regex::new(r"brew\s+install\b").unwrap(),
        Regex::new(r"apt\s+install\b").unwrap(),
        Regex::new(r"mv\s+").unwrap(),
        Regex::new(r"cp\s+-r").unwrap(),
    ]);

    // Low patterns - safe/readonly operations
    static LOW_PATTERNS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:ls|cat|head|tail|less|more|git\s+(?:status|log|diff|branch|show|stash\s+list)|pwd|echo|printf|which|where|env|printenv|find|tree|file|stat|wc|date|uname|hostname|whoami|id|groups|df|du|free|uptime|realpath|basename|dirname|readlink|test|true|false|\[|cargo\s+(?:build|test|check|clippy|fmt|doc|bench)|npm\s+(?:test|run|start)|yarn\s+(?:test|run|start)|make(?:\s|$)|cmake\s|python\s+-c|node\s+-e|grep|rg|ag|sed\s+-n|awk|sort|uniq|cut|tr|tee|diff|comm|join|paste|column|fold|fmt|expand|unexpand|nl|pr|od|xxd|hexdump|strings|md5|sha\d*sum|shasum|cksum|base64|rev|seq|yes|cal|bc|expr|factor|numfmt)(?:\s|$)").unwrap()
    });

    // Check critical first
    for pat in CRITICAL_PATTERNS.iter() {
        if pat.is_match(command) {
            return RiskLevel::Critical;
        }
    }

    // Then high
    for pat in HIGH_PATTERNS.iter() {
        if pat.is_match(command) {
            return RiskLevel::High;
        }
    }

    // Then low (known-safe commands)
    if LOW_PATTERNS.is_match(command) {
        return RiskLevel::Low;
    }

    // Unknown bash commands default to Medium
    RiskLevel::Medium
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
        assert_eq!(classify_risk("bash", &json!({"command": "docker ps"})), RiskLevel::Medium);
        assert_eq!(classify_risk("bash", &json!({"command": "node malicious.js"})), RiskLevel::Medium);
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
