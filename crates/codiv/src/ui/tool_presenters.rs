//! Tool-specific result handlers extracted from BlockRegistry.
//!
//! Each function takes the tool arguments, result string, and contextual info,
//! and returns a `(header, summary, full_content, is_diff, preview_lines)` tuple
//! plus optional edit metadata. The BlockRegistry calls into these functions
//! from `record_tool_result` to keep block storage separate from tool-specific
//! presentation logic.

use serde_json::Value;

use super::blocks::canonical_tool_name;
use super::diff::generate_unified_diff;

const MAX_PREVIEW_LINES: usize = 10;

// ---------------------------------------------------------------------------
// Per-tool result presenters
// ---------------------------------------------------------------------------

/// Present an Edit tool result — computes diffs, handles merge chains.
///
/// Returns `(action, edit_file_path, edit_original_content)` where the latter
/// two are metadata the caller should store on the ToolBlock.
pub(crate) fn present_edit(
    args: &Value,
    result: &str,
    replay_mode: bool,
    stashed_content: Option<String>,
    merge_baseline: Option<String>,
    width: u16,
) -> EditPresentation {
    if result.starts_with("Error:") {
        let header = build_tool_header("Edit", args, width);
        let summary = format!("  \u{2514} Edit failed: {}", result);
        return EditPresentation {
            header,
            summary,
            full_content: result.to_string(),
            is_diff: false,
            preview_lines: vec![],
            edit_file_path: json_str(args, "file_path"),
            edit_original_content: None,
            merging: false,
        };
    }

    if replay_mode {
        let old_string = json_str(args, "old_string").unwrap_or_default();
        let new_string = json_str(args, "new_string").unwrap_or_default();
        let header = build_tool_header("Edit", args, width);
        let summary = format!("  \u{2514} {}", edit_args_summary(&old_string, &new_string));
        return EditPresentation {
            header,
            summary,
            full_content: String::new(),
            is_diff: true,
            preview_lines: vec![],
            edit_file_path: json_str(args, "file_path"),
            edit_original_content: None,
            merging: false,
        };
    }

    let file_path = json_str(args, "file_path").unwrap_or_default();
    let stashed = stashed_content.unwrap_or_default();
    let new_content = std::fs::read_to_string(&file_path).unwrap_or_default();

    let merging = merge_baseline.is_some();
    let baseline = merge_baseline.unwrap_or_else(|| stashed.clone());

    let diff = generate_unified_diff(&file_path, &baseline, &new_content);

    if diff.is_empty() {
        let old_string = json_str(args, "old_string").unwrap_or_default();
        let new_string = json_str(args, "new_string").unwrap_or_default();
        let header = build_tool_header("Edit", args, width);
        let summary = format!("  \u{2514} {}", edit_args_summary(&old_string, &new_string));
        return EditPresentation {
            header,
            summary,
            full_content: String::new(),
            is_diff: true,
            preview_lines: vec![],
            edit_file_path: Some(file_path),
            edit_original_content: Some(stashed),
            merging,
        };
    }

    let diff_preview = generate_diff_preview_lines(&diff);
    let summary = format!("  \u{2514} {}", diff_summary(&diff));
    let header = build_tool_header("Edit", args, width);

    EditPresentation {
        header,
        summary,
        full_content: diff,
        is_diff: true,
        preview_lines: diff_preview,
        edit_file_path: Some(file_path),
        edit_original_content: Some(stashed),
        merging,
    }
}

/// Presentation data returned by `present_edit`.
pub(crate) struct EditPresentation {
    pub header: String,
    pub summary: String,
    pub full_content: String,
    pub is_diff: bool,
    pub preview_lines: Vec<String>,
    pub edit_file_path: Option<String>,
    pub edit_original_content: Option<String>,
    pub merging: bool,
}

/// Present a Read tool result.
pub(crate) fn present_read(args: &Value, result: &str, width: u16) -> ToolPresentation {
    let header = build_tool_header("Read", args, width);
    if result.starts_with("Error:") {
        let summary = format!("  \u{2514} Read failed: {}", result);
        ToolPresentation {
            header,
            summary,
            full_content: result.to_string(),
            is_diff: false,
            preview_lines: vec![],
        }
    } else {
        let line_count = result.lines().count();
        let summary = format!("  \u{2514} Read {} lines", line_count);
        ToolPresentation {
            header,
            summary,
            full_content: result.to_string(),
            is_diff: false,
            preview_lines: vec![],
        }
    }
}

/// Present a Write tool result.
pub(crate) fn present_write(args: &Value, result: &str, width: u16) -> ToolPresentation {
    let file_path = json_str(args, "file_path").unwrap_or_default();
    let content = json_str(args, "content").unwrap_or_default();
    let header = build_tool_header("Write", args, width);

    if result.starts_with("Error:") {
        let summary = format!("  \u{2514} Write failed: {}", result);
        ToolPresentation {
            header,
            summary,
            full_content: result.to_string(),
            is_diff: false,
            preview_lines: vec![],
        }
    } else {
        let line_count = content.lines().count();
        let short_path = short_filename(&file_path);
        let summary = format!("  \u{2514} Wrote {} lines to {}", line_count, short_path);
        let preview = generate_preview_lines(&content, MAX_PREVIEW_LINES, Some(1));
        ToolPresentation {
            header,
            summary,
            full_content: result.to_string(),
            is_diff: false,
            preview_lines: preview,
        }
    }
}

/// Present a Bash tool result.
pub(crate) fn present_bash(args: &Value, result: &str, width: u16) -> ToolPresentation {
    let exit_code = parse_bash_exit_code(result);
    let header = build_tool_header("Bash", args, width);
    let summary = if exit_code == 0 {
        "  \u{2514} exit 0".to_string()
    } else {
        let stderr_first = extract_stderr_first_line(result);
        if stderr_first.is_empty() {
            format!("  \u{2514} exit {}", exit_code)
        } else {
            format!("  \u{2514} exit {}: {}", exit_code, stderr_first)
        }
    };
    let stdout = extract_bash_stdout(result);
    let preview = generate_preview_lines(&stdout, MAX_PREVIEW_LINES, None);
    ToolPresentation {
        header,
        summary,
        full_content: result.to_string(),
        is_diff: false,
        preview_lines: preview,
    }
}

/// Present a Grep tool result.
pub(crate) fn present_grep(args: &Value, result: &str, width: u16) -> ToolPresentation {
    let match_count = result.lines().filter(|l| !l.is_empty()).count();
    let header = build_tool_header("Grep", args, width);
    let noun = if match_count == 1 { "line" } else { "lines" };
    let summary = format!("  \u{2514} Found {} {}", match_count, noun);
    ToolPresentation {
        header,
        summary,
        full_content: result.to_string(),
        is_diff: false,
        preview_lines: vec![],
    }
}

/// Present a Glob tool result.
pub(crate) fn present_glob(args: &Value, result: &str, width: u16) -> ToolPresentation {
    let file_count = result.lines().filter(|l| !l.is_empty()).count();
    let header = build_tool_header("Glob", args, width);
    let noun = if file_count == 1 { "file" } else { "files" };
    let summary = format!("  \u{2514} Found {} {}", file_count, noun);
    ToolPresentation {
        header,
        summary,
        full_content: result.to_string(),
        is_diff: false,
        preview_lines: vec![],
    }
}

/// Present an unknown/other tool result.
pub(crate) fn present_other(tool_name: &str, args: &Value, result: &str, width: u16) -> ToolPresentation {
    let header = build_tool_header(tool_name, args, width);
    let summary = "  \u{2514} completed".to_string();
    ToolPresentation {
        header,
        summary,
        full_content: result.to_string(),
        is_diff: false,
        preview_lines: vec![],
    }
}

/// Generic presentation data for non-Edit tools.
pub(crate) struct ToolPresentation {
    pub header: String,
    pub summary: String,
    pub full_content: String,
    pub is_diff: bool,
    pub preview_lines: Vec<String>,
}

// ---------------------------------------------------------------------------
// Helper functions (moved from blocks.rs)
// ---------------------------------------------------------------------------

/// Extract a string field from a JSON value.
pub(crate) fn json_str(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

/// Truncate a string to at most `max` characters, appending "..." if truncated.
/// Uses char count (not byte count) to handle multi-byte characters safely.
pub(crate) fn truncate_str(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{}...", truncated)
    }
}

/// Extract just the filename (or last path component) from a full path.
pub(crate) fn short_filename(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// Build a summary like "Added 3 lines, removed 1 line" from a unified diff.
fn diff_summary(diff: &str) -> String {
    let mut added = 0usize;
    let mut removed = 0usize;
    for line in diff.lines() {
        if line.starts_with("---") || line.starts_with("+++") || line.starts_with("@@") {
            continue;
        }
        if line.starts_with('+') {
            added += 1;
        } else if line.starts_with('-') {
            removed += 1;
        }
    }
    match (added, removed) {
        (0, 0) => "No changes".to_string(),
        (a, 0) => {
            let noun = if a == 1 { "line" } else { "lines" };
            format!("Added {} {}", a, noun)
        }
        (0, r) => {
            let noun = if r == 1 { "line" } else { "lines" };
            format!("Removed {} {}", r, noun)
        }
        (a, r) => {
            let an = if a == 1 { "line" } else { "lines" };
            let rn = if r == 1 { "line" } else { "lines" };
            format!("Added {} {}, removed {} {}", a, an, r, rn)
        }
    }
}

/// Build a summary from Edit tool args (old_string/new_string) for replay/no-op cases
/// where the diff is empty because edits are already applied.
fn edit_args_summary(old_string: &str, new_string: &str) -> String {
    let added = new_string.lines().count();
    let removed = old_string.lines().count();
    match (added, removed) {
        (0, 0) => "No changes".to_string(),
        (a, 0) => {
            let noun = if a == 1 { "line" } else { "lines" };
            format!("Added {} {}", a, noun)
        }
        (0, r) => {
            let noun = if r == 1 { "line" } else { "lines" };
            format!("Removed {} {}", r, noun)
        }
        (a, r) => {
            let an = if a == 1 { "line" } else { "lines" };
            let rn = if r == 1 { "line" } else { "lines" };
            format!("Added {} {}, removed {} {}", a, an, r, rn)
        }
    }
}

/// Parse the bash exit code from the result string.
fn parse_bash_exit_code(result: &str) -> i32 {
    let trimmed = result.trim_end();
    if let Some(pos) = trimmed.rfind("exit code: ") {
        let after = &trimmed[pos + "exit code: ".len()..];
        let num_str: String = after
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '-')
            .collect();
        num_str.parse::<i32>().unwrap_or(0)
    } else {
        0
    }
}

/// Extract the first line of stderr from a bash result.
fn extract_stderr_first_line(result: &str) -> String {
    if let Some(start) = result.find("<stderr>") {
        let after = &result[start + "<stderr>".len()..];
        if let Some(end) = after.find("</stderr>") {
            let stderr = &after[..end];
            let first_line = stderr.trim_start().lines().next().unwrap_or("");
            return truncate_str(first_line.trim(), 80);
        }
    }
    String::new()
}

/// Generate GitHub-style diff preview lines from a unified diff.
pub(crate) fn generate_diff_preview_lines(diff: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut old_line: usize = 0;
    let mut new_line: usize = 0;

    for line in diff.lines() {
        if line.starts_with("---") || line.starts_with("+++") {
            continue;
        }
        if line.starts_with("@@") {
            if let Some(rest) = line.strip_prefix("@@ -") {
                let parts: Vec<&str> = rest.splitn(2, ' ').collect();
                if let Some(old_part) = parts.first() {
                    let old_start: &str = old_part.split(',').next().unwrap_or("1");
                    old_line = old_start.parse().unwrap_or(1);
                }
                if let Some(new_part) = parts.get(1) {
                    if let Some(new_spec) = new_part.strip_prefix('+') {
                        let new_start = new_spec.split(',').next().unwrap_or("1");
                        new_line = new_start.parse().unwrap_or(1);
                    }
                }
            }
            result.push(format!("     \x1b[36m{}\x1b[0m", line));
            continue;
        }
        if let Some(content) = line.strip_prefix('-') {
            result.push(format!(
                "     \x1b[31m{:>4}      -{}\x1b[0m",
                old_line, content
            ));
            old_line += 1;
        } else if let Some(content) = line.strip_prefix('+') {
            result.push(format!(
                "     \x1b[32m     {:>4} +{}\x1b[0m",
                new_line, content
            ));
            new_line += 1;
        } else if let Some(content) = line.strip_prefix(' ') {
            result.push(format!(
                "     {:>4} {:>4}  {}",
                old_line, new_line, content
            ));
            old_line += 1;
            new_line += 1;
        }
    }
    result
}

/// Generate preview lines from content, indented and truncated.
pub(crate) fn generate_preview_lines(content: &str, max_lines: usize, start_line: Option<usize>) -> Vec<String> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return vec![];
    }

    let format_line = |line_idx: usize, content: &str, width: usize| -> String {
        match start_line {
            Some(start) => {
                let num = start + line_idx;
                format!("     {:>width$}  {}", num, truncate_str(content, 120), width = width)
            }
            None => format!("     {}", truncate_str(content, 120)),
        }
    };

    if lines.len() <= max_lines {
        let last_num = start_line.map(|s| s + lines.len() - 1).unwrap_or(0);
        let width = if last_num > 0 { last_num.to_string().len() } else { 0 };
        lines.iter().enumerate().map(|(i, l)| format_line(i, l, width)).collect()
    } else {
        let visible = max_lines - 1;
        let last_num = start_line.map(|s| s + visible - 1).unwrap_or(0);
        let width = if last_num > 0 { last_num.to_string().len() } else { 0 };
        let mut result: Vec<String> = lines[..visible]
            .iter()
            .enumerate()
            .map(|(i, l)| format_line(i, l, width))
            .collect();
        let remaining = lines.len() - visible;
        match start_line {
            Some(_) => {
                let padding = " ".repeat(width + 2);
                result.push(format!("     {}… +{} more lines", padding, remaining));
            }
            None => {
                result.push(format!("     … +{} more lines", remaining));
            }
        }
        result
    }
}

/// Extract stdout from a bash result by stripping stderr blocks and exit code.
fn extract_bash_stdout(result: &str) -> String {
    let mut cleaned = result.to_string();
    while let Some(start) = cleaned.find("<stderr>") {
        if let Some(end) = cleaned[start..].find("</stderr>") {
            cleaned.replace_range(start..start + end + "</stderr>".len(), "");
        } else {
            break;
        }
    }
    let trimmed = cleaned.trim_end();
    if let Some(pos) = trimmed.rfind("\nexit code: ") {
        cleaned = trimmed[..pos].to_string();
    } else if let Some(pos) = trimmed.rfind("\nsuccess, exit code: ") {
        cleaned = trimmed[..pos].to_string();
    } else if trimmed.starts_with("exit code: ") || trimmed.starts_with("success, exit code: ") {
        cleaned = String::new();
    }
    cleaned.trim().to_string()
}

// ---------------------------------------------------------------------------
// Tool header builders (moved from blocks.rs)
// ---------------------------------------------------------------------------

/// Build the single-line header for a tool block (e.g. `Edit(foo.rs)`).
/// Returns a summary suitable for the green/red result header.
/// `width` is the available content width in columns.
pub fn build_tool_header(name: &str, args: &Value, width: u16) -> String {
    // Single-line header is just the first line of the multi-line version.
    build_tool_header_lines(name, args, 0, width).into_iter().next().unwrap_or_default()
}

/// Format a pattern+path pair for Grep/Glob tool headers.
/// `width` is the available content width in columns.
fn format_search_args(tool: &str, args: &Value, width: u16) -> String {
    let pattern = json_str(args, "pattern").unwrap_or_default();
    let path = json_str(args, "path").unwrap_or_default();
    // Overhead: "Tool(pattern: "")" = tool.len() + 14
    let overhead = tool.len() + 14;
    let available = (width as usize).saturating_sub(overhead);
    let args_preview = if path.is_empty() {
        format!("pattern: \"{}\"", truncate_str(&pattern, available))
    } else {
        // Extra overhead for ", path: \"\"" = 10
        let pattern_budget = available.saturating_sub(10) * 2 / 3;
        format!(
            "pattern: \"{}\", path: \"{}\"",
            truncate_str(&pattern, pattern_budget.max(10)),
            short_filename(&path)
        )
    };
    format!("{}({})", tool, args_preview)
}

/// Build a multi-line header for a tool block, showing up to `max_lines` of content.
/// Returns a Vec of lines (first line is the header, subsequent lines are indented content).
/// Used for the yellow "in-progress" header where we want to show full command details.
/// `width` is the available content width in columns.
pub fn build_tool_header_lines(name: &str, args: &Value, max_lines: usize, width: u16) -> Vec<String> {
    let canonical = canonical_tool_name(name);
    // Available space inside parens: width - "Tool(" - ")" - 1 (margin)
    let paren_overhead = canonical.len() + 2; // "Tool(" + ")"
    let available = (width as usize).saturating_sub(paren_overhead + 1);
    match canonical {
        "Bash" => {
            let command = json_str(args, "command").unwrap_or_default();
            let cmd_lines: Vec<&str> = command.lines().collect();
            if cmd_lines.len() <= 1 {
                vec![format!("Bash({})", truncate_str(&command, available))]
            } else {
                let mut result = vec!["Bash".to_string()];
                let show = cmd_lines.len().min(max_lines);
                for line in &cmd_lines[..show] {
                    result.push(format!("  {}", line));
                }
                if cmd_lines.len() > max_lines {
                    result.push(format!("  ... ({} more lines)", cmd_lines.len() - max_lines));
                }
                result
            }
        }
        "Edit" => {
            let file_path = json_str(args, "file_path").unwrap_or_default();
            let old_string = json_str(args, "old_string").unwrap_or_default();
            let new_string = json_str(args, "new_string").unwrap_or_default();
            let old_lines: Vec<&str> = old_string.lines().collect();
            let new_lines: Vec<&str> = new_string.lines().collect();
            let total = old_lines.len() + new_lines.len();
            if total <= 2 {
                vec![format!("Edit({})", short_filename(&file_path))]
            } else {
                let mut result = vec![format!("Edit({})", short_filename(&file_path))];
                let show_old = old_lines.len().min(max_lines / 2);
                let show_new = new_lines.len().min(max_lines - show_old);
                let content_width = (width as usize).saturating_sub(6); // "  - " prefix + margin
                for line in &old_lines[..show_old] {
                    result.push(format!("  \x1b[31m- {}\x1b[33m", truncate_str(line, content_width)));
                }
                if old_lines.len() > show_old {
                    result.push(format!("  ... ({} more removed)", old_lines.len() - show_old));
                }
                for line in &new_lines[..show_new] {
                    result.push(format!("  \x1b[32m+ {}\x1b[33m", truncate_str(line, content_width)));
                }
                if new_lines.len() > show_new {
                    result.push(format!("  ... ({} more added)", new_lines.len() - show_new));
                }
                result
            }
        }
        "Read" => {
            let file_path = json_str(args, "file_path").unwrap_or_default();
            vec![format!("Read({})", truncate_str(&file_path, available))]
        }
        "Write" => {
            let file_path = json_str(args, "file_path").unwrap_or_default();
            let content = json_str(args, "content").unwrap_or_default();
            let line_count = content.lines().count();
            vec![format!("Write({}) — {} lines", truncate_str(&file_path, available.saturating_sub(15)), line_count)]
        }
        "Grep" => vec![format_search_args("Grep", args, width)],
        "Glob" => vec![format_search_args("Glob", args, width)],
        other => vec![format!("{}({})", other, truncate_str(&summarize_args(args), available))],
    }
}

/// Create a short summary of JSON arguments for "Other" tools.
pub(crate) fn summarize_args(args: &Value) -> String {
    match args {
        Value::Object(map) => {
            let parts: Vec<String> = map
                .iter()
                .take(3)
                .map(|(k, v)| {
                    let val = match v {
                        Value::String(s) => truncate_str(s, 20),
                        other => truncate_str(&other.to_string(), 20),
                    };
                    format!("{}: {}", k, val)
                })
                .collect();
            parts.join(", ")
        }
        _ => "...".to_string(),
    }
}
