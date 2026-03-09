use slate_common::messages::RiskLevel;
use slate_common::permissions::{PermissionDecision, PermissionMode};

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
                "read" | "glob" | "grep" => true,
                "bash" => {
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
}
