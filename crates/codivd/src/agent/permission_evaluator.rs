use codiv_common::messages::RiskLevel;
use codiv_common::permissions::{PermissionDecision, PermissionMode};
use codiv_common::tools::tool_names;

use super::risk_classifier::is_readonly_bash;

/// Evaluate what permission decision to make based on mode, tool name, args, and risk level.
/// Implements the decision matrix:
///
/// | Mode       | Low     | Medium  | High                | Critical     |
/// |------------|---------|---------|---------------------|--------------|
/// | Auto       | Allow   | Allow   | LLM evaluator       | Prompt user  |
/// | Manual     | Allow*  | Prompt  | Prompt              | Prompt       |
/// | Bypass     | Allow   | Allow   | Allow               | Prompt user  |
///
/// *Manual: Allow only for read-only tools (read, glob, grep) and read-only bash commands.
pub fn evaluate_permission(
    mode: PermissionMode,
    tool_name: &str,
    args: &serde_json::Value,
    risk: RiskLevel,
) -> PermissionDecision {
    match mode {
        PermissionMode::Auto => match risk {
            RiskLevel::Low => PermissionDecision::Allow,
            RiskLevel::Medium => PermissionDecision::Allow,
            RiskLevel::High => PermissionDecision::LlmEvaluate,
            RiskLevel::Critical => PermissionDecision::Prompt,
        },
        PermissionMode::Manual => {
            // In manual mode, allow read-only tools and read-only bash commands
            let is_readonly = match tool_name {
                tool_names::READ | tool_names::GLOB | tool_names::GREP => true,
                tool_names::BASH => {
                    let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
                    is_readonly_bash(cmd)
                }
                _ => false,
            };
            if is_readonly {
                PermissionDecision::Allow
            } else {
                PermissionDecision::Prompt
            }
        },
        PermissionMode::Bypass => match risk {
            RiskLevel::Low => PermissionDecision::Allow,
            RiskLevel::Medium => PermissionDecision::Allow,
            RiskLevel::High => PermissionDecision::Allow,
            RiskLevel::Critical => PermissionDecision::Prompt,
        },
    }
}

/// Extract the subcommand pattern from a bash command for permission matching.
/// Uses the first two words (e.g. "git log", "cargo build") so permissions
/// are granular rather than blanket. Falls back to a single word for commands
/// without a subcommand (e.g. "ls"). Returns `None` for non-bash tools.
pub fn extract_args_pattern(tool_name: &str, args: &serde_json::Value) -> Option<String> {
    if tool_name == "bash" {
        args.get("command").and_then(|v| v.as_str())
            .and_then(|cmd| {
                let mut words = cmd.split_whitespace();
                let first = words.next().filter(|w| !w.is_empty())?;
                match words.next().filter(|w| !w.starts_with('-')) {
                    Some(second) => Some(format!("{} {}", first, second)),
                    None => Some(first.to_string()),
                }
            })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -- Auto mode tests --
    #[test]
    fn auto_low_allows() {
        assert_eq!(
            evaluate_permission(PermissionMode::Auto, "read", &json!({}), RiskLevel::Low),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn auto_medium_allows() {
        assert_eq!(
            evaluate_permission(PermissionMode::Auto, "write", &json!({}), RiskLevel::Medium),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn auto_high_evaluates() {
        assert_eq!(
            evaluate_permission(PermissionMode::Auto, "bash", &json!({"command": "git push"}), RiskLevel::High),
            PermissionDecision::LlmEvaluate
        );
    }

    #[test]
    fn auto_critical_prompts() {
        assert_eq!(
            evaluate_permission(PermissionMode::Auto, "bash", &json!({"command": "rm -rf /"}), RiskLevel::Critical),
            PermissionDecision::Prompt
        );
    }

    // -- Manual mode tests --
    #[test]
    fn manual_read_allows() {
        assert_eq!(
            evaluate_permission(PermissionMode::Manual, "read", &json!({}), RiskLevel::Low),
            PermissionDecision::Allow
        );
        assert_eq!(
            evaluate_permission(PermissionMode::Manual, "glob", &json!({}), RiskLevel::Low),
            PermissionDecision::Allow
        );
        assert_eq!(
            evaluate_permission(PermissionMode::Manual, "grep", &json!({}), RiskLevel::Low),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn manual_readonly_bash_allows() {
        assert_eq!(
            evaluate_permission(PermissionMode::Manual, "bash", &json!({"command": "ls -la"}), RiskLevel::Low),
            PermissionDecision::Allow
        );
        assert_eq!(
            evaluate_permission(PermissionMode::Manual, "bash", &json!({"command": "git status"}), RiskLevel::Low),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn manual_write_prompts() {
        assert_eq!(
            evaluate_permission(PermissionMode::Manual, "write", &json!({}), RiskLevel::Medium),
            PermissionDecision::Prompt
        );
    }

    #[test]
    fn manual_bash_write_prompts() {
        assert_eq!(
            evaluate_permission(PermissionMode::Manual, "bash", &json!({"command": "cargo build"}), RiskLevel::Low),
            PermissionDecision::Prompt
        );
    }

    // -- Bypass mode tests --
    #[test]
    fn bypass_high_allows() {
        assert_eq!(
            evaluate_permission(PermissionMode::Bypass, "bash", &json!({"command": "git push"}), RiskLevel::High),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn bypass_critical_prompts() {
        assert_eq!(
            evaluate_permission(PermissionMode::Bypass, "bash", &json!({"command": "rm -rf /"}), RiskLevel::Critical),
            PermissionDecision::Prompt
        );
    }

    #[test]
    fn extract_args_pattern_bash() {
        let args = serde_json::json!({"command": "cargo build"});
        assert_eq!(extract_args_pattern("bash", &args), Some("cargo build".to_string()));
    }

    #[test]
    fn extract_args_pattern_bash_single_word() {
        let args = serde_json::json!({"command": "ls"});
        assert_eq!(extract_args_pattern("bash", &args), Some("ls".to_string()));
    }

    #[test]
    fn extract_args_pattern_bash_skips_flags() {
        // Flag as second word is skipped, falls back to single word
        let args = serde_json::json!({"command": "git --no-pager log"});
        assert_eq!(extract_args_pattern("bash", &args), Some("git".to_string()));
        let args2 = serde_json::json!({"command": "ls -la"});
        assert_eq!(extract_args_pattern("bash", &args2), Some("ls".to_string()));
    }

    #[test]
    fn extract_args_pattern_non_bash() {
        let args = serde_json::json!({"file_path": "/foo"});
        assert_eq!(extract_args_pattern("read", &args), None);
    }

    #[test]
    fn extract_args_pattern_empty_command() {
        let args = serde_json::json!({"command": ""});
        assert_eq!(extract_args_pattern("bash", &args), None);
    }
}
