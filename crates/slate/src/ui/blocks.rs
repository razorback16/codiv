//! Block registry and types for the terminal block UI.
//!
//! Each tool call or user prompt is represented as a [`Block`] in the
//! [`BlockRegistry`]. Tool calls render as 2-line summaries (header + summary)
//! that can be expanded via a modal overlay.

use serde_json::Value;

use super::diff::generate_unified_diff;

// ---------------------------------------------------------------------------
// Block types
// ---------------------------------------------------------------------------

pub struct ToolBlock {
    pub id: usize,
    pub tool_name: String,
    pub header: String,
    pub summary: String,
    pub full_content: String,
    pub is_diff: bool,
    pub scrollback_line: u64,
    pub line_count: u16,
    /// Track the number of merged edits for consecutive-edit merging.
    merge_count: usize,
    /// Stores the file_path for Edit blocks so we can detect consecutive edits.
    edit_file_path: Option<String>,
}

#[allow(dead_code)]
pub struct PromptBlock {
    pub id: usize,
    pub text: String,
    pub scrollback_line: u64,
}

#[allow(dead_code)]
pub struct CmdResponseBlock {
    pub id: usize,
    pub command: String,
    pub scrollback_line: u64,
    pub line_count: u16,
    pub exit_code: i32,
}

#[allow(dead_code)]
pub struct AiResponseBlock {
    pub id: usize,
    pub scrollback_line: u64,
    pub line_count: u16,
}

pub struct ThinkingBlock {
    pub id: usize,
    pub scrollback_line: u64,
    pub line_count: u16,        // 1 (the summary line)
    pub full_content: String,   // accumulated reasoning text
    pub duration_secs: f32,
}

pub enum Block {
    Prompt(PromptBlock),
    CmdResponse(CmdResponseBlock),
    AiResponse(AiResponseBlock),
    Thinking(ThinkingBlock),
    Tool(ToolBlock),
}

// ---------------------------------------------------------------------------
// Action returned by record_tool_result
// ---------------------------------------------------------------------------

pub enum ToolResultAction {
    /// Render nothing new to VT100 — block was merged into previous entry.
    Merged,
    /// Render the 2-line summary to VT100.
    Summary { header: String, summary: String },
}

// ---------------------------------------------------------------------------
// Pending tool call (saved between ToolCall and ToolResult events)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
struct PendingToolCall {
    name: String,
    arguments: String,
}

// ---------------------------------------------------------------------------
// BlockRegistry
// ---------------------------------------------------------------------------

pub struct BlockRegistry {
    blocks: Vec<Block>,
    focused_index: Option<usize>,
    next_id: usize,
    pending_tool_call: Option<PendingToolCall>,
}

impl BlockRegistry {
    pub fn new() -> Self {
        Self {
            blocks: Vec::new(),
            focused_index: None,
            next_id: 0,
            pending_tool_call: None,
        }
    }

    // -- recording ----------------------------------------------------------

    /// Save the pending tool call so its arguments are available when
    /// `record_tool_result` is called.
    pub fn record_tool_call(&mut self, name: &str, arguments: &str) {
        self.pending_tool_call = Some(PendingToolCall {
            name: name.to_string(),
            arguments: arguments.to_string(),
        });
    }

    /// Create (or merge) a [`ToolBlock`] for the given tool result.
    ///
    /// Returns a [`ToolResultAction`] telling the caller whether to render
    /// new lines or whether this result was merged into an existing block.
    pub fn record_tool_result(
        &mut self,
        name: &str,
        result: &str,
        scrollback_line: u64,
    ) -> ToolResultAction {
        let args_json = self
            .pending_tool_call
            .take()
            .map(|p| p.arguments)
            .unwrap_or_default();
        let args: Value = serde_json::from_str(&args_json).unwrap_or(Value::Null);

        let canonical = canonical_tool_name(name);

        match canonical {
            "Edit" => self.record_edit(&args, result, scrollback_line),
            "Read" => self.record_read(&args, result, scrollback_line),
            "Write" => self.record_write(&args, result, scrollback_line),
            "Bash" => self.record_bash(&args, result, scrollback_line),
            "Grep" => self.record_grep(&args, result, scrollback_line),
            "Glob" => self.record_glob(&args, result, scrollback_line),
            other => self.record_other(other, &args, result, scrollback_line),
        }
    }

    /// Record a user prompt.
    pub fn record_prompt(&mut self, text: &str, scrollback_line: u64) {
        let id = self.next_id();
        self.blocks.push(Block::Prompt(PromptBlock {
            id,
            text: text.to_string(),
            scrollback_line,
        }));
    }

    /// Record a shell command response block.
    pub fn record_cmd_response(&mut self, command: &str, scrollback_line: u64, line_count: u16, exit_code: i32) {
        let id = self.next_id();
        self.blocks.push(Block::CmdResponse(CmdResponseBlock {
            id,
            command: command.to_string(),
            scrollback_line,
            line_count,
            exit_code,
        }));
    }

    /// Record an AI response block.
    pub fn record_ai_response(&mut self, scrollback_line: u64, line_count: u16) {
        let id = self.next_id();
        self.blocks.push(Block::AiResponse(AiResponseBlock {
            id,
            scrollback_line,
            line_count,
        }));
    }

    /// Record a thinking/reasoning block.
    pub fn record_thinking(&mut self, scrollback_line: u64, full_content: String, duration_secs: f32) {
        let id = self.next_id();
        self.blocks.push(Block::Thinking(ThinkingBlock {
            id,
            scrollback_line,
            line_count: 1,
            full_content,
            duration_secs,
        }));
    }

    // -- accessors ----------------------------------------------------------

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    // -- focus / navigation -------------------------------------------------

    fn is_navigable(block: &Block) -> bool {
        matches!(block, Block::Prompt(_) | Block::Tool(_) | Block::Thinking(_))
    }

    pub fn focus_last(&mut self) {
        self.focused_index = self.blocks.iter().rposition(Self::is_navigable);
    }

    pub fn focus_prev(&mut self) {
        if let Some(i) = self.focused_index {
            self.focused_index = self.blocks[..i].iter().rposition(Self::is_navigable);
        }
    }

    pub fn focus_next(&mut self) {
        if let Some(i) = self.focused_index {
            self.focused_index = self.blocks[i + 1..]
                .iter()
                .position(Self::is_navigable)
                .map(|rel| i + 1 + rel);
            // None means past last navigable block → unfocused (returns to live prompt)
        }
    }

    pub fn unfocus(&mut self) {
        self.focused_index = None;
    }

    pub fn focused(&self) -> Option<&Block> {
        self.focused_index.and_then(|i| self.blocks.get(i))
    }

    pub fn focused_index(&self) -> Option<usize> {
        self.focused_index
    }

    // -- accessors ----------------------------------------------------------

    #[allow(dead_code)]
    pub fn get_entry(&self, idx: usize) -> Option<&Block> {
        self.blocks.get(idx)
    }

    #[allow(dead_code)]
    pub fn entry_count(&self) -> usize {
        self.blocks.len()
    }

    pub fn clear(&mut self) {
        self.blocks.clear();
        self.focused_index = None;
        self.next_id = 0;
        self.pending_tool_call = None;
    }

    /// Returns a mutable reference to the last [`ToolBlock`], if any.
    pub fn last_tool_block_mut(&mut self) -> Option<&mut ToolBlock> {
        self.blocks.iter_mut().rev().find_map(|b| match b {
            Block::Tool(tb) => Some(tb),
            _ => None,
        })
    }

    // -- private helpers ----------------------------------------------------

    fn next_id(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    // -- per-tool recording -------------------------------------------------

    fn record_edit(
        &mut self,
        args: &Value,
        _result: &str,
        scrollback_line: u64,
    ) -> ToolResultAction {
        let file_path = json_str(args, "file_path").unwrap_or_default();
        let old_string = json_str(args, "old_string").unwrap_or_default();
        let new_string = json_str(args, "new_string").unwrap_or_default();

        let diff = generate_unified_diff(&file_path, &old_string, &new_string);

        // Check if previous block is an Edit to the same file — merge if so.
        if let Some(Block::Tool(prev)) = self.blocks.last_mut() {
            if prev.tool_name == "Edit" && prev.edit_file_path.as_deref() == Some(&file_path) {
                prev.merge_count += 1;
                prev.summary = format!("  \u{2514} Edited ({} edits)", prev.merge_count);
                prev.full_content.push('\n');
                prev.full_content.push_str(&diff);
                return ToolResultAction::Merged;
            }
        }

        // First edit (or new file) — compute single-edit summary.
        let summary_text = single_edit_summary(&old_string, &new_string);
        let short_path = short_filename(&file_path);
        let header = format!("Edit({})", short_path);
        let summary = format!("  \u{2514} {}", summary_text);

        let id = self.next_id();
        self.blocks.push(Block::Tool(ToolBlock {
            id,
            tool_name: "Edit".to_string(),
            header: header.clone(),
            summary: summary.clone(),
            full_content: diff,
            is_diff: true,
            scrollback_line,
            line_count: 2,
            merge_count: 1,
            edit_file_path: Some(file_path),
        }));

        ToolResultAction::Summary { header, summary }
    }

    fn record_read(
        &mut self,
        args: &Value,
        result: &str,
        scrollback_line: u64,
    ) -> ToolResultAction {
        let file_path = json_str(args, "file_path").unwrap_or_default();
        let line_count = result.lines().count();
        let short_path = short_filename(&file_path);

        let header = format!("Read({})", short_path);
        let summary = format!("  \u{2514} Read {} lines", line_count);

        let id = self.next_id();
        self.blocks.push(Block::Tool(ToolBlock {
            id,
            tool_name: "Read".to_string(),
            header: header.clone(),
            summary: summary.clone(),
            full_content: result.to_string(),
            is_diff: false,
            scrollback_line,
            line_count: 2,
            merge_count: 0,
            edit_file_path: None,
        }));

        ToolResultAction::Summary { header, summary }
    }

    fn record_write(
        &mut self,
        args: &Value,
        result: &str,
        scrollback_line: u64,
    ) -> ToolResultAction {
        let file_path = json_str(args, "file_path").unwrap_or_default();
        let content = json_str(args, "content").unwrap_or_default();
        let line_count = content.lines().count();
        let short_path = short_filename(&file_path);

        let header = format!("Write({})", short_path);
        let summary = format!("  \u{2514} Wrote {} lines to {}", line_count, short_path);

        let id = self.next_id();
        self.blocks.push(Block::Tool(ToolBlock {
            id,
            tool_name: "Write".to_string(),
            header: header.clone(),
            summary: summary.clone(),
            full_content: result.to_string(),
            is_diff: false,
            scrollback_line,
            line_count: 2,
            merge_count: 0,
            edit_file_path: None,
        }));

        ToolResultAction::Summary { header, summary }
    }

    fn record_bash(
        &mut self,
        args: &Value,
        result: &str,
        scrollback_line: u64,
    ) -> ToolResultAction {
        let command = json_str(args, "command").unwrap_or_default();
        let command_preview = truncate_str(&command, 60);
        let exit_code = parse_bash_exit_code(result);

        let header = format!("Bash({})", command_preview);
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

        let id = self.next_id();
        self.blocks.push(Block::Tool(ToolBlock {
            id,
            tool_name: "Bash".to_string(),
            header: header.clone(),
            summary: summary.clone(),
            full_content: result.to_string(),
            is_diff: false,
            scrollback_line,
            line_count: 2,
            merge_count: 0,
            edit_file_path: None,
        }));

        ToolResultAction::Summary { header, summary }
    }

    fn record_grep(
        &mut self,
        args: &Value,
        result: &str,
        scrollback_line: u64,
    ) -> ToolResultAction {
        let pattern = json_str(args, "pattern").unwrap_or_default();
        let path = json_str(args, "path").unwrap_or_default();
        let match_count = result.lines().filter(|l| !l.is_empty()).count();

        let args_preview = if path.is_empty() {
            format!("pattern: \"{}\"", truncate_str(&pattern, 40))
        } else {
            format!(
                "pattern: \"{}\", path: \"{}\"",
                truncate_str(&pattern, 30),
                short_filename(&path)
            )
        };

        let header = format!("Grep({})", args_preview);
        let noun = if match_count == 1 { "line" } else { "lines" };
        let summary = format!("  \u{2514} Found {} {}", match_count, noun);

        let id = self.next_id();
        self.blocks.push(Block::Tool(ToolBlock {
            id,
            tool_name: "Grep".to_string(),
            header: header.clone(),
            summary: summary.clone(),
            full_content: result.to_string(),
            is_diff: false,
            scrollback_line,
            line_count: 2,
            merge_count: 0,
            edit_file_path: None,
        }));

        ToolResultAction::Summary { header, summary }
    }

    fn record_glob(
        &mut self,
        args: &Value,
        result: &str,
        scrollback_line: u64,
    ) -> ToolResultAction {
        let pattern = json_str(args, "pattern").unwrap_or_default();
        let path = json_str(args, "path").unwrap_or_default();
        let file_count = result.lines().filter(|l| !l.is_empty()).count();

        let args_preview = if path.is_empty() {
            format!("pattern: \"{}\"", truncate_str(&pattern, 40))
        } else {
            format!(
                "pattern: \"{}\", path: \"{}\"",
                truncate_str(&pattern, 30),
                short_filename(&path)
            )
        };

        let header = format!("Glob({})", args_preview);
        let noun = if file_count == 1 { "file" } else { "files" };
        let summary = format!("  \u{2514} Found {} {}", file_count, noun);

        let id = self.next_id();
        self.blocks.push(Block::Tool(ToolBlock {
            id,
            tool_name: "Glob".to_string(),
            header: header.clone(),
            summary: summary.clone(),
            full_content: result.to_string(),
            is_diff: false,
            scrollback_line,
            line_count: 2,
            merge_count: 0,
            edit_file_path: None,
        }));

        ToolResultAction::Summary { header, summary }
    }

    fn record_other(
        &mut self,
        tool_name: &str,
        args: &Value,
        result: &str,
        scrollback_line: u64,
    ) -> ToolResultAction {
        let args_preview = summarize_args(args);
        let header = format!("{}({})", tool_name, args_preview);
        let summary = "  \u{2514} completed".to_string();

        let id = self.next_id();
        self.blocks.push(Block::Tool(ToolBlock {
            id,
            tool_name: tool_name.to_string(),
            header: header.clone(),
            summary: summary.clone(),
            full_content: result.to_string(),
            is_diff: false,
            scrollback_line,
            line_count: 2,
            merge_count: 0,
            edit_file_path: None,
        }));

        ToolResultAction::Summary { header, summary }
    }
}

// ---------------------------------------------------------------------------
// Free helpers
// ---------------------------------------------------------------------------

/// Normalize tool names to a canonical form (e.g. "bash" -> "Bash").
fn canonical_tool_name(name: &str) -> &str {
    match name.to_ascii_lowercase().as_str() {
        "edit" => "Edit",
        "read" => "Read",
        "write" => "Write",
        "bash" => "Bash",
        "grep" => "Grep",
        "glob" => "Glob",
        _ => name,
    }
}

/// Extract a string field from a JSON value.
fn json_str(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

/// Truncate a string to at most `max` characters, appending "..." if truncated.
fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut truncated = s[..max].to_string();
        truncated.push_str("...");
        truncated
    }
}

/// Extract just the filename (or last path component) from a full path.
fn short_filename(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// Build a short summary for a single edit based on old/new string comparison.
fn single_edit_summary(old_string: &str, new_string: &str) -> String {
    let old_count = if old_string.is_empty() {
        0
    } else {
        old_string.lines().count()
    };
    let new_count = if new_string.is_empty() {
        0
    } else {
        new_string.lines().count()
    };

    if old_count == 0 && new_count > 0 {
        let noun = if new_count == 1 { "line" } else { "lines" };
        format!("Added {} {}", new_count, noun)
    } else if new_count == 0 && old_count > 0 {
        let noun = if old_count == 1 { "line" } else { "lines" };
        format!("Removed {} {}", old_count, noun)
    } else if new_count > old_count {
        let diff = new_count - old_count;
        let noun = if diff == 1 { "line" } else { "lines" };
        format!("Added {} {}", diff, noun)
    } else if old_count > new_count {
        let diff = old_count - new_count;
        let noun = if diff == 1 { "line" } else { "lines" };
        format!("Removed {} {}", diff, noun)
    } else {
        let noun = if old_count == 1 { "line" } else { "lines" };
        format!("Changed {} {}", old_count, noun)
    }
}

/// Parse the bash exit code from the result string.
///
/// Looks for `exit code: N` at the end of the result. If not present,
/// assumes exit code 0.
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
///
/// Stderr content is wrapped in `<stderr>` and `</stderr>` tags.
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

/// Create a short summary of JSON arguments for "Other" tools.
fn summarize_args(args: &Value) -> String {
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
