use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// A single event in a persisted conversation timeline.
///
/// This replaces the old `SessionEvent` enum and adds tool-call tracking
/// that was previously missing entirely.  Each variant mirrors what the
/// daemon already streams via `DaemonMessage`/`StreamChunk`, but stores
/// aggregated (non-streaming) content suitable for persistence and replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConversationEvent {
    /// User submitted a prompt (AI mode `? …`).
    UserPrompt {
        text: String,
        request_id: String,
    },
    /// User ran a shell command in the terminal.
    ShellCommand {
        command: String,
        output: String,
        exit_code: i32,
        cwd: String,
    },
    /// Final aggregated assistant text for one request.
    AssistantText {
        request_id: String,
        text: String,
    },
    /// Aggregated reasoning/thinking content for one request.
    AssistantReasoning {
        request_id: String,
        text: String,
        #[serde(default)]
        duration_secs: f32,
    },
    /// A tool invocation the model made during a request.
    ToolCall {
        request_id: String,
        tool_name: String,
        arguments: String,
    },
    /// The result returned by a tool invocation.
    ToolResult {
        request_id: String,
        tool_name: String,
        result: String,
    },
    /// An error that occurred during a request.
    Error {
        request_id: String,
        message: String,
    },
    /// Token usage for a single request (persisted for session replay).
    TokenUsage {
        request_id: String,
        input_tokens: usize,
        output_tokens: usize,
        cache_read_tokens: usize,
    },
    /// Compacted summary replacing older conversation history.
    Summary {
        text: String,
        compacted_event_count: usize,
    },
}

/// Metadata for a stored session (one row in the `sessions` table).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub name: Option<String>,
    pub parent_id: Option<String>,
    pub cwd: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Parse an ISO 8601 / SQLite datetime string and return a human-friendly
/// relative time string ("just now", "5m ago", "2h ago", "yesterday", etc.).
pub fn relative_time(timestamp: &str) -> String {
    // SQLite datetime format: "YYYY-MM-DD HH:MM:SS"
    let parsed = parse_sqlite_datetime(timestamp);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let ts = match parsed {
        Some(t) => t,
        None => return timestamp.to_string(),
    };
    let delta = now.saturating_sub(ts);
    if delta < 60 {
        "just now".to_string()
    } else if delta < 3600 {
        format!("{}m ago", delta / 60)
    } else if delta < 86400 {
        format!("{}h ago", delta / 3600)
    } else if delta < 172800 {
        "yesterday".to_string()
    } else if delta < 604800 {
        format!("{} days ago", delta / 86400)
    } else {
        // Show abbreviated date for older sessions
        // Extract "Mon DD" from the timestamp
        timestamp.get(..10).unwrap_or(timestamp).to_string()
    }
}

/// Parse "YYYY-MM-DD HH:MM:SS" into Unix epoch seconds (UTC).
fn parse_sqlite_datetime(s: &str) -> Option<u64> {
    // Minimal parser — avoids pulling in chrono just for this.
    let s = s.trim();
    if s.len() < 19 {
        return None;
    }
    let year: u64 = s[0..4].parse().ok()?;
    let month: u64 = s[5..7].parse().ok()?;
    let day: u64 = s[8..10].parse().ok()?;
    let hour: u64 = s[11..13].parse().ok()?;
    let min: u64 = s[14..16].parse().ok()?;
    let sec: u64 = s[17..19].parse().ok()?;

    // Days from year 1970 to start of `year`
    let mut days: u64 = 0;
    for y in 1970..year {
        days += if is_leap(y) { 366 } else { 365 };
    }
    let month_days = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for m in 1..month {
        days += month_days[m as usize];
        if m == 2 && is_leap(year) {
            days += 1;
        }
    }
    days += day - 1;
    Some(days * 86400 + hour * 3600 + min * 60 + sec)
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}
