//! Block registry and types for the terminal block UI.
//!
//! Each tool call or user prompt is represented as a [`Block`] in the
//! [`BlockRegistry`]. Tool calls render as 2-line summaries (header + summary)
//! that can be expanded via a modal overlay.

use std::collections::{HashMap, VecDeque};

use serde_json::Value;

use super::tool_presenters::{self, json_str, truncate_str, short_filename, summarize_args};

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
    /// Available content width for header truncation (updated on resize).
    width: u16,
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
            width: 120,
        }
    }

    /// Set the available content width (called on init and resize).
    pub fn set_width(&mut self, width: u16) {
        self.width = width;
    }

    /// Get the current content width.
    pub fn width(&self) -> u16 {
        self.width
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
    ///
    /// `pre_edit_content` is the file content captured by the daemon *before*
    /// the edit tool executes.  When present it is used instead of reading
    /// from disk, avoiding a race where the file has already been modified by
    /// the time the client processes the ToolCall message.
    pub fn record_tool_call(&mut self, name: &str, arguments: &str, pre_edit_content: Option<&str>) {
        // Stash old file content for Edit tools to compute file-level diff later.
        if canonical_tool_name(name) == "Edit"
            && !self.replay_mode
        {
            if let Ok(args) = serde_json::from_str::<Value>(arguments) {
                if let Some(path) = json_str(&args, "file_path") {
                    let content = match pre_edit_content {
                        Some(c) => c.to_string(),
                        None => std::fs::read_to_string(&path).unwrap_or_default(),
                    };
                    self.pending_edit_old_contents.insert(path, content);
                }
            }
        }
        // Do NOT set pending_tool_name here — only record_tool_call_delta does that,
        // because only that path writes a spinner placeholder to overwrite.
        self.pending_tool_calls.push_back(PendingToolCall {
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
            .unwrap_or_else(|| {
                log::debug!("record_tool_result: pending_tool_calls empty for tool '{}'", name);
                String::new()
            });
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
    #[allow(clippy::too_many_arguments)]
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
        // Gather merge baseline from previous Edit blocks on the same file.
        let file_path = json_str(args, "file_path").unwrap_or_default();

        // Remove stashed content for this file (consumed by the presenter).
        let stashed = if result.starts_with("Error:") {
            self.pending_edit_old_contents.remove(&file_path);
            None
        } else {
            self.pending_edit_old_contents.remove(&file_path)
        };

        // Find merge baseline from previous Edit block on the same file.
        let merge_baseline = if !self.replay_mode && !result.starts_with("Error:") {
            let mut candidate = None;
            for block in self.blocks.iter().rev() {
                match block {
                    Block::Thinking(_) => continue,
                    Block::Tool(prev)
                        if prev.tool_name == "Edit"
                            && prev.edit_file_path.as_deref() == Some(&file_path) =>
                    {
                        candidate = Some(prev.edit_original_content.as_deref().unwrap_or_default().to_string());
                        break;
                    }
                    _ => break,
                }
            }
            candidate
        } else {
            None
        };

        let ep = tool_presenters::present_edit(args, result, self.replay_mode, stashed, merge_baseline, self.width);

        if ep.merging {
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
            prev.full_content = ep.full_content;
            prev.summary = ep.summary.clone();
            prev.height = 2 + ep.preview_lines.len() as u16;
            ToolResultAction::Merged { start_index, header, summary: ep.summary, preview_lines: ep.preview_lines }
        } else {
            let id = self.next_id();
            let height = 2 + ep.preview_lines.len() as u16;
            self.blocks.push(Block::Tool(ToolBlock {
                id,
                tool_name: "Edit".to_string(),
                header: ep.header.clone(),
                summary: ep.summary.clone(),
                full_content: ep.full_content,
                is_diff: ep.is_diff,
                start_index,
                height,
                rendered_lines: vec![],
                edit_file_path: ep.edit_file_path,
                edit_original_content: ep.edit_original_content,
            }));
            ToolResultAction::Summary { header: ep.header, summary: ep.summary, preview_lines: ep.preview_lines }
        }
    }

    fn record_read(&mut self, args: &Value, result: &str, start_index: u64) -> ToolResultAction {
        let p = tool_presenters::present_read(args, result, self.width);
        self.push_tool_block("Read", p.header, p.summary, p.full_content, p.is_diff, start_index, p.preview_lines)
    }

    fn record_write(&mut self, args: &Value, result: &str, start_index: u64) -> ToolResultAction {
        let p = tool_presenters::present_write(args, result, self.width);
        self.push_tool_block("Write", p.header, p.summary, p.full_content, p.is_diff, start_index, p.preview_lines)
    }

    fn record_bash(&mut self, args: &Value, result: &str, start_index: u64) -> ToolResultAction {
        let p = tool_presenters::present_bash(args, result, self.width);
        self.push_tool_block("Bash", p.header, p.summary, p.full_content, p.is_diff, start_index, p.preview_lines)
    }

    fn record_grep(&mut self, args: &Value, result: &str, start_index: u64) -> ToolResultAction {
        let p = tool_presenters::present_grep(args, result, self.width);
        self.push_tool_block("Grep", p.header, p.summary, p.full_content, p.is_diff, start_index, p.preview_lines)
    }

    fn record_glob(&mut self, args: &Value, result: &str, start_index: u64) -> ToolResultAction {
        let p = tool_presenters::present_glob(args, result, self.width);
        self.push_tool_block("Glob", p.header, p.summary, p.full_content, p.is_diff, start_index, p.preview_lines)
    }

    fn record_other(&mut self, tool_name: &str, args: &Value, result: &str, start_index: u64) -> ToolResultAction {
        let p = tool_presenters::present_other(tool_name, args, result, self.width);
        self.push_tool_block(tool_name, p.header, p.summary, p.full_content, p.is_diff, start_index, p.preview_lines)
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

// Helper functions are now in tool_presenters.rs — re-exported above.
