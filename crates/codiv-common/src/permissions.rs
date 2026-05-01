use serde::{Deserialize, Serialize};

/// Permission mode controls how tool calls are gated.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionMode {
    /// Hybrid rule-based + LLM evaluator. Low/Medium: allow, High: LLM evaluate, Critical: prompt user.
    #[default]
    Auto,
    /// Read-only free, writes need approval. Low (read-only): allow, Medium/High/Critical: prompt user.
    Manual,
    /// Everything except Critical allowed. Low/Medium/High: allow, Critical: prompt user.
    Bypass,
}

impl std::fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => write!(f, "auto"),
            Self::Manual => write!(f, "manual"),
            Self::Bypass => write!(f, "bypass"),
        }
    }
}

impl PermissionMode {
    /// Cycle to the next mode: Auto → Manual → Bypass → Auto.
    pub fn next(self) -> Self {
        match self {
            Self::Auto => Self::Manual,
            Self::Manual => Self::Bypass,
            Self::Bypass => Self::Auto,
        }
    }
}

/// The result of evaluating a permission check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    /// Allow the tool call to proceed.
    Allow,
    /// Prompt the user for confirmation.
    Prompt,
    /// Invoke the LLM evaluator to decide (Auto mode, High risk only).
    LlmEvaluate,
    /// Deny the tool call outright.
    Deny,
}
