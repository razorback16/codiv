pub const TOOL_NAMES: &[&str] = &["bash", "read", "write", "edit", "glob", "grep"];

pub fn agent_guide(tool_name: &str) -> &'static str {
    match tool_name {
        "bash" => "Execute shell commands. Prefer single commands over chaining with &&. Set timeout_ms for long-running operations. stdout and stderr are combined in output. Check exit codes for error handling.",
        "read" => "Read file contents with numbered lines. Always read before editing. Use offset and limit for large files to reduce token usage. Line numbers are 1-based.",
        "write" => "Write or overwrite file content. Creates parent directories automatically. Prefer edit over write when making small changes to existing files.",
        "edit" => "Find-and-replace in files. The old_string must appear exactly once. Include enough surrounding context in old_string to ensure uniqueness. Prefer over write for targeted changes.",
        "glob" => "Find files by glob pattern. Supports ** for recursive matching. Results are sorted. Use path to limit search scope.",
        "grep" => "Search file contents with regex. Respects .gitignore. Use include to filter by file extension. Maximum 500 matches returned. Use path to narrow search scope.",
        _ => "Unknown tool.",
    }
}
