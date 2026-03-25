//! Block registry and types for the terminal block UI.
//!
//! Each tool call or user prompt is represented as a [`Block`] in the
//! [`BlockRegistry`]. Tool calls render as 2-line summaries (header + summary)
//! that can be expanded via a modal overlay.

use std::collections::{HashMap, VecDeque};

use serde_json::Value;

use super::diff::generate_unified_diff;

const MAX_PREVIEW_LINES: usize = 10;

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
    pub start_index: u64,
    pub height: u16,
    /// ANSI-escaped lines ready for `parser.process()`, used by `rerender_all()`.
    pub rendered_lines: Vec<String>,
    /// Stores the file_path for Edit blocks so we can detect consecutive edits.
    edit_file_path: Option<String>,
    /// Original file content before the first edit in a merge chain.
    edit_original_content: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputMode {
    Command,
    Ai,
}

pub struct PromptBlock {
    #[allow(dead_code)]
    pub id: usize,
    #[allow(dead_code)]
    pub text: String,
    pub start_index: u64,
    pub height: u16,
    pub mode: InputMode,
    /// ANSI-escaped rendered prompt line(s).
    pub rendered_lines: Vec<String>,
}

pub struct CmdResponseBlock {
    #[allow(dead_code)]
    pub id: usize,
    #[allow(dead_code)]
    pub command: String,
    pub start_index: u64,
    pub height: u16,
    #[allow(dead_code)]
    pub exit_code: i32,
    /// Raw VT100 bytes from the PTY, for replay via `parser.process()`.
    pub raw_bytes: Vec<u8>,
}

pub struct AiResponseBlock {
    #[allow(dead_code)]
    pub id: usize,
    pub start_index: u64,
    pub height: u16,
    /// ANSI-escaped rendered lines from the markdown stream.
    pub rendered_lines: Vec<String>,
}

pub struct ThinkingBlock {
    pub id: usize,
    pub content: String,
    pub duration_secs: f32,
    pub start_index: u64,
    pub height: u16, // always 1 (the "Thought for Ns" summary line)
    /// ANSI-escaped rendered line (the "Thought for Ns" summary).
    pub rendered_lines: Vec<String>,
}

pub enum Block {
    Prompt(PromptBlock),
    CmdResponse(CmdResponseBlock),
    AiResponse(AiResponseBlock),
    Tool(ToolBlock),
    Thinking(ThinkingBlock),
}

// ---------------------------------------------------------------------------
// Action returned by record_tool_result
// ---------------------------------------------------------------------------

pub enum ToolResultAction {
    /// Block was merged into previous entry — erase back to `start_index` and redraw.
    Merged {
        start_index: u64,
        header: String,
        summary: String,
        preview_lines: Vec<String>,
    },
    /// Render the header + summary + preview lines to VT100.
    Summary { header: String, summary: String, preview_lines: Vec<String> },
}

// ---------------------------------------------------------------------------
// Pending tool call (saved between ToolCall and ToolResult events)
// ---------------------------------------------------------------------------

struct PendingToolCall {
    #[allow(dead_code)]
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
    pending_tool_calls: VecDeque<PendingToolCall>,
    pending_tool_name: Option<String>,
    pending_tool_start_index: Option<u64>,
    pending_edit_old_contents: HashMap<String, String>,
    replay_mode: bool,
}

impl BlockRegistry {
    pub fn new() -> Self {
        Self {
            blocks: Vec::new(),
            focused_index: None,
            next_id: 0,
            pending_tool_calls: VecDeque::new(),
            pending_tool_name: None,
            pending_tool_start_index: None,
            pending_edit_old_contents: HashMap::new(),
            replay_mode: false,
        }
    }

    pub fn set_replay_mode(&mut self, replay: bool) {
        self.replay_mode = replay;
    }

    pub fn pending_tool(&self) -> Option<&str> {
        self.pending_tool_name.as_deref()
    }

    pub fn pending_tool_start_index(&self) -> Option<u64> {
        self.pending_tool_start_index
    }

    // -- recording ----------------------------------------------------------

    /// Save the pending tool call so its arguments are available when
    /// `record_tool_result` is called.
    pub fn record_tool_call(&mut self, name: &str, arguments: &str) {
        // Stash old file content for Edit tools to compute file-level diff later.
        if canonical_tool_name(name) == "Edit" {
            if !self.replay_mode {
                if let Ok(args) = serde_json::from_str::<Value>(arguments) {
                    if let Some(path) = json_str(&args, "file_path") {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            self.pending_edit_old_contents.insert(path, content);
                        }
                    }
                }
            }
        }
        // Do NOT set pending_tool_name here — only record_tool_call_delta does that,
        // because only that path writes a spinner placeholder to overwrite.
        self.pending_tool_calls.push_back(PendingToolCall {
            name: name.to_string(),
            arguments: arguments.to_string(),
        });
    }

    /// Record that a tool call delta has started streaming.
    /// Sets `pending_tool_name` on the first delta only.
    pub fn record_tool_call_delta(&mut self, tool_name: &str, start_index: u64) {
        if self.pending_tool_name.is_none() {
            self.pending_tool_name = Some(tool_name.to_string());
            self.pending_tool_start_index = Some(start_index);
        }
    }

    /// Create (or merge) a [`ToolBlock`] for the given tool result.
    ///
    /// Returns a [`ToolResultAction`] telling the caller whether to render
    /// new lines or whether this result was merged into an existing block.
    pub fn record_tool_result(
        &mut self,
        name: &str,
        result: &str,
        start_index: u64,
    ) -> ToolResultAction {
        self.pending_tool_name = None;
        self.pending_tool_start_index = None;
        let args_json = self
            .pending_tool_calls
            .pop_front()
            .map(|p| p.arguments)
            .unwrap_or_default();
        let args: Value = serde_json::from_str(&args_json).unwrap_or(Value::Null);

        let canonical = canonical_tool_name(name);

        let action = match canonical {
            "Edit" => self.record_edit(&args, result, start_index),
            "Read" => self.record_read(&args, result, start_index),
            "Write" => self.record_write(&args, result, start_index),
            "Bash" => self.record_bash(&args, result, start_index),
            "Grep" => self.record_grep(&args, result, start_index),
            "Glob" => self.record_glob(&args, result, start_index),
            other => self.record_other(other, &args, result, start_index),
        };
        if self.replay_mode {
            match action {
                ToolResultAction::Summary { header, summary, .. } => {
                    if let Some(Block::Tool(tb)) = self.blocks.last_mut() {
                        tb.height = 2;
                    }
                    ToolResultAction::Summary { header, summary, preview_lines: vec![] }
                }
                ToolResultAction::Merged { start_index, header, summary, .. } => {
                    if let Some(Block::Tool(tb)) = self.blocks.last_mut() {
                        tb.height = 2;
                    }
                    ToolResultAction::Merged { start_index, header, summary, preview_lines: vec![] }
                }
            }
        } else {
            action
        }
    }

    /// Record a user prompt.
    pub fn record_prompt(&mut self, text: &str, start_index: u64, mode: InputMode, rendered_lines: Vec<String>) {
        let id = self.next_id();
        let height = rendered_lines.len().max(text.split('\n').count().max(1)) as u16;
        self.blocks.push(Block::Prompt(PromptBlock {
            id,
            text: text.to_string(),
            start_index,
            height,
            mode,
            rendered_lines,
        }));
    }

    /// Record a shell command response block.
    pub fn record_cmd_response(&mut self, command: &str, start_index: u64, height: u16, exit_code: i32, raw_bytes: Vec<u8>) {
        let id = self.next_id();
        self.blocks.push(Block::CmdResponse(CmdResponseBlock {
            id,
            command: command.to_string(),
            start_index,
            height,
            exit_code,
            raw_bytes,
        }));
    }

    /// Record a standalone thinking block.
    pub fn record_thinking_block(
        &mut self,
        content: String,
        duration_secs: f32,
        scrollback_line: u64,
        rendered_line: String,
    ) {
        let id = self.next_id();
        self.blocks.push(Block::Thinking(ThinkingBlock {
            id,
            content,
            duration_secs,
            start_index: scrollback_line,
            height: 1,
            rendered_lines: vec![rendered_line],
        }));
    }

    /// Record an AI response block.
    pub fn record_ai_response(&mut self, start_index: u64, height: u16, rendered_lines: Vec<String>) {
        let id = self.next_id();
        self.blocks.push(Block::AiResponse(AiResponseBlock {
            id,
            start_index,
            height,
            rendered_lines,
        }));
    }

    /// Store rendered ANSI lines on the most recently added/merged ToolBlock.
    pub fn set_last_tool_rendered_lines(&mut self, lines: Vec<String>) {
        if let Some(Block::Tool(tb)) = self.blocks.last_mut() {
            tb.height = lines.len() as u16;
            tb.rendered_lines = lines;
        }
    }

    // -- accessors ----------------------------------------------------------

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    // -- focus / navigation -------------------------------------------------

    fn is_navigable(block: &Block) -> bool {
        matches!(block, Block::Prompt(_) | Block::Tool(_) | Block::AiResponse(_) | Block::Thinking(_))
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

    // -- direct block creation (for replay) ----------------------------------

    /// Add a prompt block with pre-computed rendered lines (for replay).
    pub fn add_prompt_block(&mut self, text: &str, mode: InputMode, rendered_lines: Vec<String>) {
        let id = self.next_id();
        let height = rendered_lines.len().max(1) as u16;
        self.blocks.push(Block::Prompt(PromptBlock {
            id,
            text: text.to_string(),
            start_index: 0,
            height,
            mode,
            rendered_lines,
        }));
    }

    /// Add a command response block with pre-computed raw bytes (for replay).
    pub fn add_cmd_response_block(
        &mut self,
        command: &str,
        exit_code: i32,
        raw_bytes: Vec<u8>,
        height: u16,
    ) {
        let id = self.next_id();
        self.blocks.push(Block::CmdResponse(CmdResponseBlock {
            id,
            command: command.to_string(),
            start_index: 0,
            height,
            exit_code,
            raw_bytes,
        }));
    }

    /// Add an AI response block with pre-computed rendered lines (for replay).
    pub fn add_ai_response_block(&mut self, rendered_lines: Vec<String>) {
        let id = self.next_id();
        let height = rendered_lines.len() as u16;
        self.blocks.push(Block::AiResponse(AiResponseBlock {
            id,
            start_index: 0,
            height,
            rendered_lines,
        }));
    }

    /// Add a thinking block with pre-computed rendered line (for replay).
    pub fn add_thinking_block(&mut self, content: String, duration_secs: f32, rendered_line: String) {
        let id = self.next_id();
        self.blocks.push(Block::Thinking(ThinkingBlock {
            id,
            content,
            duration_secs,
            start_index: 0,
            height: 1,
            rendered_lines: vec![rendered_line],
        }));
    }

    /// Add a tool block with pre-computed rendered lines (for replay).
    pub fn add_tool_block(
        &mut self,
        tool_name: &str,
        header: String,
        summary: String,
        full_content: String,
        is_diff: bool,
        rendered_lines: Vec<String>,
    ) {
        let id = self.next_id();
        let height = rendered_lines.len() as u16;
        self.blocks.push(Block::Tool(ToolBlock {
            id,
            tool_name: tool_name.to_string(),
            header,
            summary,
            full_content,
            is_diff,
            start_index: 0,
            height,
            rendered_lines,
            edit_file_path: None,
            edit_original_content: None,
        }));
    }

    /// Recompute `start_index` on every block as a running sum of heights + separators.
    /// `base` is the absolute line index of the first block (e.g. after the intro line).
    pub fn recompute_positions(&mut self, base: u64) {
        let mut pos = base;
        for block in &mut self.blocks {
            match block {
                Block::Prompt(b) => {
                    b.start_index = pos;
                    pos += b.rendered_lines.len() as u64;
                }
                Block::CmdResponse(b) => {
                    b.start_index = pos;
                    pos += b.height as u64;
                }
                Block::AiResponse(b) => {
                    b.start_index = pos;
                    pos += b.rendered_lines.len() as u64;
                }
                Block::Tool(b) => {
                    b.start_index = pos;
                    pos += b.rendered_lines.len() as u64;
                }
                Block::Thinking(b) => {
                    b.start_index = pos;
                    pos += b.rendered_lines.len() as u64;
                }
            }
            pos += 1; // separator line between blocks
        }
    }

    // -- accessors ----------------------------------------------------------

    pub fn clear(&mut self) {
        self.blocks.clear();
        self.focused_index = None;
        self.next_id = 0;
        self.pending_tool_calls.clear();
        self.pending_tool_name = None;
        self.pending_tool_start_index = None;
        self.pending_edit_old_contents.clear();
        self.replay_mode = false;
    }

    // -- private helpers ----------------------------------------------------

    fn next_id(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Push a standard (non-merge) tool block and return a `Summary` action.
    fn push_tool_block(
        &mut self,
        tool_name: &str,
        header: String,
        summary: String,
        full_content: String,
        is_diff: bool,
        start_index: u64,
        preview_lines: Vec<String>,
    ) -> ToolResultAction {
        let id = self.next_id();
        let height = 2 + preview_lines.len() as u16;
        self.blocks.push(Block::Tool(ToolBlock {
            id,
            tool_name: tool_name.to_string(),
            header: header.clone(),
            summary: summary.clone(),
            full_content,
            is_diff,
            start_index,
            height,
            rendered_lines: vec![],

            edit_file_path: None,
            edit_original_content: None,
        }));
        ToolResultAction::Summary { header, summary, preview_lines }
    }

    // -- per-tool recording -------------------------------------------------

    fn record_edit(
        &mut self,
        args: &Value,
        result: &str,
        start_index: u64,
    ) -> ToolResultAction {
        if result.starts_with("Error:") {
            let header = build_tool_header("Edit", args);
            let summary = format!("  \u{2514} Edit failed: {}", result);
            if let Some(path) = json_str(args, "file_path") {
                self.pending_edit_old_contents.remove(&path);
            }
            return self.push_tool_block("Edit", header, summary, result.to_string(), false, start_index, vec![]);
        }
        // In replay mode, skip file I/O — files have stale content.
        if self.replay_mode {
            let old_string = json_str(args, "old_string").unwrap_or_default();
            let new_string = json_str(args, "new_string").unwrap_or_default();
            let header = build_tool_header("Edit", args);
            let summary = format!("  \u{2514} {}", edit_args_summary(&old_string, &new_string));
            return self.push_tool_block("Edit", header, summary, String::new(), true, start_index, vec![]);
        }
        let file_path = json_str(args, "file_path").unwrap_or_default();
        let stashed = self.pending_edit_old_contents.remove(&file_path).unwrap_or_default();
        let new_content = std::fs::read_to_string(&file_path).unwrap_or_default();

        // Merging? Use the original baseline. Otherwise use the stashed pre-edit content.
        // Skip trailing Thinking blocks to find a previous Edit on the same file.
        let (merging, baseline) = {
            let mut merge_candidate = None;
            for block in self.blocks.iter().rev() {
                match block {
                    Block::Thinking(_) => continue,
                    Block::Tool(prev)
                        if prev.tool_name == "Edit"
                            && prev.edit_file_path.as_deref() == Some(&file_path) =>
                    {
                        merge_candidate = Some(prev.edit_original_content.as_deref().unwrap_or_default().to_string());
                        break;
                    }
                    _ => break,
                }
            }
            match merge_candidate {
                Some(baseline) => (true, baseline),
                None => (false, stashed.clone()),
            }
        };

        let diff = generate_unified_diff(&file_path, &baseline, &new_content);

        // Replay / no-op: diff is empty because the file already has edits applied.
        if diff.is_empty() {
            let old_string = json_str(args, "old_string").unwrap_or_default();
            let new_string = json_str(args, "new_string").unwrap_or_default();
            let header = build_tool_header("Edit", args);
            let summary = format!("  \u{2514} {}", edit_args_summary(&old_string, &new_string));
            if merging {
                // Remove trailing Thinking blocks so the Edit block is last again.
                while matches!(self.blocks.last(), Some(Block::Thinking(_))) {
                    self.blocks.pop();
                }
                let prev = match self.blocks.last_mut() {
                    Some(Block::Tool(tb)) => tb,
                    _ => unreachable!(),
                };
                let start_index = prev.start_index;
                let header = prev.header.clone();
                prev.summary = summary.clone();
                prev.height = 2;
                return ToolResultAction::Merged { start_index, header, summary, preview_lines: vec![] };
            }
            let id = self.next_id();
            self.blocks.push(Block::Tool(ToolBlock {
                id,
                tool_name: "Edit".to_string(),
                header: header.clone(),
                summary: summary.clone(),
                full_content: String::new(),
                is_diff: true,
                start_index,
                height: 2,
                rendered_lines: vec![],
                edit_file_path: Some(file_path),
                edit_original_content: Some(stashed),
            }));
            return ToolResultAction::Summary { header, summary, preview_lines: vec![] };
        }

        let diff_preview = generate_diff_preview_lines(&diff);
        let summary = format!("  \u{2514} {}", diff_summary(&diff));

        if merging {
            // Remove trailing Thinking blocks so the Edit block is last again.
            while matches!(self.blocks.last(), Some(Block::Thinking(_))) {
                self.blocks.pop();
            }
            let prev = match self.blocks.last_mut() {
                Some(Block::Tool(tb)) => tb,
                _ => unreachable!(),
            };
            let start_index = prev.start_index;
            let header = prev.header.clone();
            prev.full_content = diff;
            prev.summary = summary.clone();
            prev.height = 2 + diff_preview.len() as u16;
            ToolResultAction::Merged { start_index, header, summary, preview_lines: diff_preview }
        } else {
            let header = build_tool_header("Edit", args);
            let id = self.next_id();
            self.blocks.push(Block::Tool(ToolBlock {
                id,
                tool_name: "Edit".to_string(),
                header: header.clone(),
                summary: summary.clone(),
                full_content: diff,
                is_diff: true,
                start_index,
                height: 2 + diff_preview.len() as u16,
                rendered_lines: vec![],
                edit_file_path: Some(file_path),
                edit_original_content: Some(stashed),
            }));
            ToolResultAction::Summary { header, summary, preview_lines: diff_preview }
        }
    }

    fn record_read(&mut self, args: &Value, result: &str, start_index: u64) -> ToolResultAction {
        let header = build_tool_header("Read", args);
        if result.starts_with("Error:") {
            let summary = format!("  \u{2514} Read failed: {}", result);
            self.push_tool_block("Read", header, summary, result.to_string(), false, start_index, vec![])
        } else {
            let line_count = result.lines().count();
            let summary = format!("  \u{2514} Read {} lines", line_count);
            self.push_tool_block("Read", header, summary, result.to_string(), false, start_index, vec![])
        }
    }

    fn record_write(&mut self, args: &Value, result: &str, start_index: u64) -> ToolResultAction {
        let file_path = json_str(args, "file_path").unwrap_or_default();
        let content = json_str(args, "content").unwrap_or_default();
        let header = build_tool_header("Write", args);

        if result.starts_with("Error:") {
            let summary = format!("  \u{2514} Write failed: {}", result);
            self.push_tool_block("Write", header, summary, result.to_string(), false, start_index, vec![])
        } else {
            let line_count = content.lines().count();
            let short_path = short_filename(&file_path);
            let summary = format!("  \u{2514} Wrote {} lines to {}", line_count, short_path);
            let preview = generate_preview_lines(&content, MAX_PREVIEW_LINES, Some(1));
            self.push_tool_block("Write", header, summary, result.to_string(), false, start_index, preview)
        }
    }

    fn record_bash(&mut self, args: &Value, result: &str, start_index: u64) -> ToolResultAction {
        let exit_code = parse_bash_exit_code(result);
        let header = build_tool_header("Bash", args);
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
        self.push_tool_block("Bash", header, summary, result.to_string(), false, start_index, preview)
    }

    fn record_grep(&mut self, args: &Value, result: &str, start_index: u64) -> ToolResultAction {
        let match_count = result.lines().filter(|l| !l.is_empty()).count();
        let header = build_tool_header("Grep", args);
        let noun = if match_count == 1 { "line" } else { "lines" };
        let summary = format!("  \u{2514} Found {} {}", match_count, noun);
        self.push_tool_block("Grep", header, summary, result.to_string(), false, start_index, vec![])
    }

    fn record_glob(&mut self, args: &Value, result: &str, start_index: u64) -> ToolResultAction {
        let file_count = result.lines().filter(|l| !l.is_empty()).count();
        let header = build_tool_header("Glob", args);
        let noun = if file_count == 1 { "file" } else { "files" };
        let summary = format!("  \u{2514} Found {} {}", file_count, noun);
        self.push_tool_block("Glob", header, summary, result.to_string(), false, start_index, vec![])
    }

    fn record_other(&mut self, tool_name: &str, args: &Value, result: &str, start_index: u64) -> ToolResultAction {
        let header = build_tool_header(tool_name, args);
        let summary = "  \u{2514} completed".to_string();
        self.push_tool_block(tool_name, header, summary, result.to_string(), false, start_index, vec![])
    }
}

// ---------------------------------------------------------------------------
// Free helpers
// ---------------------------------------------------------------------------

/// Normalize tool names to a canonical form (e.g. "bash" -> "Bash").
pub(crate) fn canonical_tool_name(name: &str) -> &str {
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

/// Build the single-line header for a tool block (e.g. `Edit(foo.rs)`).
/// Returns a summary suitable for the green/red result header.
pub fn build_tool_header(name: &str, args: &Value) -> String {
    // Single-line header is just the first line of the multi-line version.
    build_tool_header_lines(name, args, 0).into_iter().next().unwrap_or_default()
}

/// Format a pattern+path pair for Grep/Glob tool headers.
fn format_search_args(tool: &str, args: &Value) -> String {
    let pattern = json_str(args, "pattern").unwrap_or_default();
    let path = json_str(args, "path").unwrap_or_default();
    let args_preview = if path.is_empty() {
        format!("pattern: \"{}\"", truncate_str(&pattern, 40))
    } else {
        format!(
            "pattern: \"{}\", path: \"{}\"",
            truncate_str(&pattern, 30),
            short_filename(&path)
        )
    };
    format!("{}({})", tool, args_preview)
}

/// Build a multi-line header for a tool block, showing up to `max_lines` of content.
/// Returns a Vec of lines (first line is the header, subsequent lines are indented content).
/// Used for the yellow "in-progress" header where we want to show full command details.
pub fn build_tool_header_lines(name: &str, args: &Value, max_lines: usize) -> Vec<String> {
    let canonical = canonical_tool_name(name);
    match canonical {
        "Bash" => {
            let command = json_str(args, "command").unwrap_or_default();
            let cmd_lines: Vec<&str> = command.lines().collect();
            if cmd_lines.len() <= 1 {
                vec![format!("Bash({})", truncate_str(&command, 60))]
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
                for line in &old_lines[..show_old] {
                    result.push(format!("  \x1b[31m- {}\x1b[33m", truncate_str(line, 100)));
                }
                if old_lines.len() > show_old {
                    result.push(format!("  ... ({} more removed)", old_lines.len() - show_old));
                }
                for line in &new_lines[..show_new] {
                    result.push(format!("  \x1b[32m+ {}\x1b[33m", truncate_str(line, 100)));
                }
                if new_lines.len() > show_new {
                    result.push(format!("  ... ({} more added)", new_lines.len() - show_new));
                }
                result
            }
        }
        "Read" => {
            let file_path = json_str(args, "file_path").unwrap_or_default();
            vec![format!("Read({})", short_filename(&file_path))]
        }
        "Write" => {
            let file_path = json_str(args, "file_path").unwrap_or_default();
            let content = json_str(args, "content").unwrap_or_default();
            let line_count = content.lines().count();
            vec![format!("Write({}) — {} lines", short_filename(&file_path), line_count)]
        }
        "Grep" => vec![format_search_args("Grep", args)],
        "Glob" => vec![format_search_args("Glob", args)],
        other => vec![format!("{}({})", other, summarize_args(args))],
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

/// Generate GitHub-style diff preview lines from a unified diff.
/// Shows line numbers and +/- markers with ANSI colors (red for removals, green for additions).
fn generate_diff_preview_lines(diff: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut old_line: usize = 0;
    let mut new_line: usize = 0;

    for line in diff.lines() {
        if line.starts_with("---") || line.starts_with("+++") {
            // Skip file headers
            continue;
        }
        if line.starts_with("@@") {
            // Parse hunk header: @@ -old_start,old_count +new_start,new_count @@
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
            // Render hunk header as a separator
            result.push(format!("     \x1b[36m{}\x1b[0m", line));
            continue;
        }
        if let Some(content) = line.strip_prefix('-') {
            // Removed line: old line number, no new line number, red
            result.push(format!(
                "     \x1b[31m{:>4}      -{}\x1b[0m",
                old_line, content
            ));
            old_line += 1;
        } else if let Some(content) = line.strip_prefix('+') {
            // Added line: no old line number, new line number, green
            result.push(format!(
                "     \x1b[32m     {:>4} +{}\x1b[0m",
                new_line, content
            ));
            new_line += 1;
        } else if let Some(content) = line.strip_prefix(' ') {
            // Context line: both line numbers
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
/// When `start_line` is Some(n), line numbers are shown starting from n.
/// When `start_line` is None, no line numbers are shown (e.g. Bash output).
fn generate_preview_lines(content: &str, max_lines: usize, start_line: Option<usize>) -> Vec<String> {
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
    // Strip all <stderr>...</stderr> blocks
    while let Some(start) = cleaned.find("<stderr>") {
        if let Some(end) = cleaned[start..].find("</stderr>") {
            cleaned.replace_range(start..start + end + "</stderr>".len(), "");
        } else {
            break;
        }
    }
    // Strip trailing "exit code: N" or "success, exit code: N" line
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
