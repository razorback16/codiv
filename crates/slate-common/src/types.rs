use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentRole {
    TeamLead,
    Engineer,
    Reviewer,
    Researcher,
    Security,
}
