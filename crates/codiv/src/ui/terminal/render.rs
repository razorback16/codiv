use std::time::Duration;

use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use tui_term::widget::{Cursor as PtCursor, PseudoTerminal};

use super::state::{TerminalState, TokenUsage};
use super::utils::format_tokens;
use super::utils::true_scrollback_len;
use super::{PROMPT_GUTTER_WIDTH, STATUS_BAR_HEIGHT};
use crate::shell::bash_coprocess::GitInfo;
use crate::ui::theme::Theme;
use codiv_common::permissions::PermissionMode;

use crate::{
    ui::blocks::{Block, InputMode},
    VERSION,
};

/// Read-only data needed to render the status bar.
pub(crate) struct StatusBarInfo<'a> {
    pub cwd: &'a str,
    pub daemon_connected: bool,
    pub daemon_timestamp: u64,
    pub is_executing: bool,
    pub git_info: Option<&'a GitInfo>,
    pub model_alias: &'a str,
    pub token_usage: &'a TokenUsage,
    pub anim: &'a super::animation::AnimationState,
    pub thinking_enabled: bool,
    pub permission_mode: PermissionMode,
    pub session_name: Option<&'a str>,
}

#[cfg(test)]
fn input_rendered_cols(input_text: &str) -> usize {
    input_text.chars().count()
}

#[inline]
fn completion_anchor_x(term_area_left: u16, cursor_col: usize) -> u16 {
    let cursor_col = u16::try_from(cursor_col).unwrap_or(u16::MAX);
    term_area_left
        .saturating_add(PROMPT_GUTTER_WIDTH)
        .saturating_add(cursor_col)
}

/// How long a contextual hint stays visible after its trigger.
pub(super) const HINT_TIMEOUT: Duration = Duration::from_secs(5);

/// Minimum input length before the "thinking" hint appears.
pub(super) const HINT_INPUT_THRESHOLD: usize = 3;

/// Returns the contextual hint text based on current UI state, or None if
/// all hints have expired (each hint is visible for `HINT_TIMEOUT`).
pub(super) fn current_hint(state: &TerminalState) -> Option<&str> {
    let timeout = HINT_TIMEOUT;

    // 1. Transient drag hint (within 5s, supported terminal)
    if state.term_supports_option_select {
        if let Some(t) = state.last_mouse_drag {
            if t.elapsed() < timeout {
                return Some("Option+drag: select text");
            }
        }
    }

    // Remaining hints are gated by hint_shown_at
    let shown_at = state.hint_shown_at?;
    if shown_at.elapsed() >= timeout {
        return None;
    }

    // 2. User has typed enough → thinking hint
    if state.input.content().len() >= HINT_INPUT_THRESHOLD {
        return if state.thinking_enabled {
            Some("Ctrl+T: thinking on")
        } else {
            Some("Ctrl+T: thinking off")
        };
    }
    // 3. Empty input → mode switch hint
    Some(match state.input_mode {
        InputMode::Ai => "Tab or ! to switch to terminal",
        InputMode::Command => "Tab or ? to switch to AI",
    })
}

pub(crate) fn render_frame(
    term: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    parser: &mut vt100::Parser,
    state: &mut TerminalState,
    daemon_connected: bool,
    theme: &Theme,
) -> Result<(), Box<dyn std::error::Error>> {
    // Write live prompt into the vt100 parser (only when scrolled to bottom
    // and no command is currently executing or agent streaming, and not in alt screen).
    let in_alt_screen = parser.screen().alternate_screen();
    let is_executing = state.pending_command.is_some();
    let is_thinking = state.thinking_start.is_some();
    let has_inline_picker = state.pending_session_picker.is_some() || state.pending_confirmation.is_some();
    if state.scroll_offset == 0 && !is_executing && !state.agent_streaming && !in_alt_screen && !has_inline_picker {
        let lines: Vec<&str> = state.input.lines().collect();
        let line_count = lines.len() as u16;
        let screen_rows = parser.screen().size().0;

        // Determine the anchor row (first row of the prompt).
        // On the first render of a live prompt, record the cursor position as the anchor.
        // On subsequent renders, reuse the anchor so the start stays fixed.
        let first_row = if let Some(anchor) = state.prompt_anchor_row {
            // Anchor exists — ensure we have room for all lines below it.
            let last_row = anchor + line_count + 1; // +1 for hint row
            if last_row >= screen_rows {
                // Need to scroll to make room at the bottom
                let overflow = last_row - screen_rows + 1;
                for _ in 0..overflow {
                    parser.process(b"\n");
                }
                // Anchor shifts up by the number of scrolled lines
                let new_anchor = anchor.saturating_sub(overflow);
                state.prompt_anchor_row = Some(new_anchor);
                new_anchor
            } else {
                anchor
            }
        } else {
            // First render — cursor is at the line where the prompt starts.
            let (cursor_row, _) = parser.screen().cursor_position();
            // Ensure room for prompt lines + bottom ruler + hint
            let last_row = cursor_row + line_count + 1; // +1 for hint row
            let first_row = if last_row >= screen_rows {
                let overflow = last_row - screen_rows + 1;
                for _ in 0..overflow {
                    parser.process(b"\n");
                }
                cursor_row.saturating_sub(overflow)
            } else {
                cursor_row
            };
            state.prompt_anchor_row = Some(first_row);
            first_row
        };

        // Clear and render each line
        for (i, line) in lines.iter().enumerate() {
            let row_1based = first_row + i as u16 + 1;
            parser.process(format!("\x1b[{};1H\x1b[K{}", row_1based, line).as_bytes());
        }

        // Clear any leftover lines below the prompt (from previous renders with more lines)
        let after_last = first_row + line_count;
        if after_last < screen_rows {
            parser.process(format!("\x1b[{};1H\x1b[K", after_last + 1).as_bytes());
        }

        // Position cursor at the correct (row, col)
        let (crow, ccol) = state.input.cursor_row_col();
        let cursor_row_1based = first_row + crow as u16 + 1;
        parser.process(format!("\x1b[{};{}H", cursor_row_1based, ccol + 1).as_bytes());

        if !state.prompt_is_live {
            // First time prompt goes live — show hint
            state.hint_shown_at = Some(std::time::Instant::now());
        }
        state.prompt_is_live = true;
    } else {
        // Not rendering a live prompt — clear the anchor
        state.prompt_anchor_row = None;
    }

    term.draw(|frame| {
        let area = frame.area();

        if in_alt_screen {
            // Fullscreen: render PseudoTerminal over the entire area (no status bar).
            let pseudo_term =
                PseudoTerminal::new(parser.screen()).cursor(PtCursor::default().visibility(true));
            frame.render_widget(pseudo_term, area);
        } else {
            // Normal: terminal area + status bar.
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(1),                    // PseudoTerminal (now includes prompt)
                    Constraint::Length(STATUS_BAR_HEIGHT), // Status bar
                ])
                .split(area);

            let term_area = chunks[0];
            let status_area = chunks[1];
            // Content area is 2 cols narrower (left gutter for `>` marker).
            let content_area = Rect {
                x: term_area.x + PROMPT_GUTTER_WIDTH,
                width: term_area.width.saturating_sub(PROMPT_GUTTER_WIDTH),
                ..term_area
            };

            // --- Render pseudoterminal ---
            let cursor_visible = state.scroll_offset == 0;
            let pseudo_term = PseudoTerminal::new(parser.screen())
                .cursor(PtCursor::default().visibility(cursor_visible));
            frame.render_widget(pseudo_term, content_area);

            // --- Render `>` prompt gutter for past and live prompts ---
            render_prompt_gutter(
                frame.buffer_mut(), parser, state, is_thinking, theme, term_area, content_area,
            );

            // --- Render status bar ---
            let status_info = StatusBarInfo {
                cwd: state.cwd.as_str(),
                daemon_connected,
                daemon_timestamp: state.last_daemon_timestamp,
                is_executing,
                git_info: state.git_info.as_ref(),
                model_alias: &state.model_alias,
                token_usage: &state.token_usage,
                anim: &state.anim,
                thinking_enabled: state.thinking_enabled,
                permission_mode: state.permission_mode,
                session_name: state.session_name.as_deref(),
            };
            render_status_bar(frame, status_area, &status_info);

            // --- Render completion popup ---
            if state.completion_popup.is_visible() {
                let (cursor_row, _cursor_col) = parser.screen().cursor_position();
                let anchor_x = completion_anchor_x(term_area.left(), state.input.cursor_position());
                let anchor_y = term_area.top() + cursor_row;
                state.completion_popup.render(frame, anchor_x, anchor_y, theme);
            }

            // --- Render block selection overlay ---
            render_block_selection_overlay(frame, parser, state, theme, term_area);

            // --- Render contextual hint below live prompt ---
            render_contextual_hint(frame.buffer_mut(), state, theme, term_area);

            // --- Render tool result modal ---
            if state.tool_result_modal.is_visible() {
                state.tool_result_modal.render(frame, area, theme);
            }
        }
    })?;

    Ok(())
}

/// Render the `>`, `$`, `●` gutter markers for all blocks and the live prompt,
/// plus spinner indicators for pending tool calls, streaming, and thinking.
fn render_prompt_gutter(
    buf: &mut Buffer,
    parser: &mut vt100::Parser,
    state: &TerminalState,
    is_thinking: bool,
    theme: &Theme,
    term_area: Rect,
    content_area: Rect,
) {
    let screen_rows = parser.screen().size().0 as u64;
    let sb_len = true_scrollback_len(parser) as u64;
    let abs_bottom = sb_len + screen_rows;
    let abs_top = abs_bottom
        .saturating_sub(screen_rows)
        .saturating_sub(state.scroll_offset as u64);
    let abs_view_bottom = abs_top + screen_rows;

    for block in state.tracker.blocks() {
        let (scrollback_line, gutter_char, gutter_fg) = match block {
            Block::Prompt(pb) => {
                let (ch, fg) = match pb.mode {
                    InputMode::Command => ('$', theme.gutter_cmd),
                    InputMode::Ai => ('>', theme.gutter_ai),
                };
                (pb.start_index, ch, fg)
            }
            Block::AiResponse(ab) => (ab.start_index, '\u{25CF}', theme.ai_bullet),
            Block::Thinking(tk) => (tk.start_index, '\u{25CB}', theme.thinking_text),
            Block::Tool(tb) => (tb.start_index, '\u{25CF}', theme.tool_bullet),
            Block::CmdResponse(cb) => (cb.start_index, '$', theme.gutter_cmd),
        };
        if scrollback_line >= abs_top && scrollback_line < abs_view_bottom {
            let screen_row = (scrollback_line - abs_top) as u16;
            let row = term_area.top() + screen_row;
            if row < term_area.bottom() {
                buf[(term_area.left(), row)]
                    .set_char(gutter_char)
                    .set_fg(gutter_fg);
            }
        }
    }
    // Live prompt: draw `>` at the anchor row.
    if state.prompt_is_live && state.scroll_offset == 0 {
        if let Some(anchor) = state.prompt_anchor_row {
            let (live_char, live_fg) = match state.input_mode {
                InputMode::Command => ('$', theme.gutter_cmd),
                InputMode::Ai => ('>', theme.gutter_ai),
            };
            let prompt_lines = state.input.line_count() as u16;
            for i in 0..prompt_lines {
                let row = term_area.top() + anchor + i;
                if row < term_area.bottom() {
                    let ch = if i == 0 { live_char } else { '·' };
                    buf[(term_area.left(), row)]
                        .set_char(ch)
                        .set_fg(live_fg);
                }
            }
        }
    } else if state.tracker.pending_tool().is_some() && state.scroll_offset == 0 {
        // Gutter-only spinner for pending tool (header is in VT100 content)
        if let Some(pending_sl) = state.tracker.pending_tool_start_index() {
            log::debug!(
                "pending_tool gutter: pending_sl={}, abs_top={}, abs_view_bottom={}",
                pending_sl,
                abs_top,
                abs_view_bottom
            );
            if pending_sl >= abs_top && pending_sl < abs_view_bottom {
                let screen_row = (pending_sl - abs_top) as u16;
                let row = term_area.top() + screen_row;
                if row < term_area.bottom() {
                    buf[(term_area.left(), row)]
                        .set_char(state.anim.spinner_char())
                        .set_fg(theme.spinner);
                }
            }
        }
    } else if state.agent_streaming && state.scroll_offset == 0 {
        let (cursor_row, _) = parser.screen().cursor_position();
        let row = term_area.top() + cursor_row;
        if row < term_area.bottom() {
            buf[(term_area.left(), row)]
                .set_char(state.anim.spinner_char())
                .set_fg(theme.spinner);
        }
    } else if is_thinking && state.scroll_offset == 0 {
        let (cursor_row, _) = parser.screen().cursor_position();
        let row = term_area.top() + cursor_row;
        if row < term_area.bottom() {
            buf[(term_area.left(), row)]
                .set_char(state.anim.spinner_char())
                .set_fg(theme.spinner);
        }
        // Overwrite the placeholder line (one above cursor) with animated dots.
        let placeholder_row = row.saturating_sub(1);
        if placeholder_row >= term_area.top() && placeholder_row < term_area.bottom() {
            let text = format!("Thinking{}", state.anim.thinking_dots());
            for (i, ch) in text.chars().enumerate() {
                let col = content_area.left() + i as u16;
                if col < content_area.right() {
                    buf[(col, placeholder_row)]
                        .set_char(ch)
                        .set_fg(theme.thinking_text);
                }
            }
            // Clear any leftover characters from longer previous text
            let clear_start = content_area.left() + text.len() as u16;
            for col in clear_start..content_area.right() {
                let ch = buf[(col, placeholder_row)].symbol();
                if ch == " " || ch.is_empty() {
                    break;
                }
                buf[(col, placeholder_row)]
                    .set_char(' ')
                    .set_fg(Color::Reset);
            }
        }
    }
}

/// Render separator rules around the focused block or live prompt.
fn render_block_selection_overlay(
    frame: &mut Frame,
    parser: &mut vt100::Parser,
    state: &TerminalState,
    theme: &Theme,
    term_area: Rect,
) {
    if let Some(focused) = state.tracker.focused() {
        let (start_index, height) = match focused {
            Block::Tool(tb) => (tb.start_index, tb.height),
            Block::Prompt(pb) => (pb.start_index, pb.height),
            Block::CmdResponse(cb) => (cb.start_index, cb.height),
            Block::AiResponse(ab) => (ab.start_index, ab.height),
            Block::Thinking(tk) => (tk.start_index, tk.height),
        };
        let sb_len = true_scrollback_len(parser) as u64;
        let screen_rows = parser.screen().size().0 as u64;
        let abs_bottom = sb_len + screen_rows;
        let abs_top = abs_bottom
            .saturating_sub(screen_rows)
            .saturating_sub(state.scroll_offset as u64);
        if start_index >= abs_top && start_index < abs_bottom {
            let screen_row = (start_index - abs_top) as u16 + term_area.top();
            let top_rule = screen_row.saturating_sub(1);
            let bottom_rule = screen_row + height;
            let buf = frame.buffer_mut();
            render_horizontal_rules(buf, &[top_rule, bottom_rule], term_area, theme);
            // Show "(press Enter to expand)" hint for ToolBlocks and thinking
            if matches!(focused, Block::Tool(_) | Block::Thinking(_)) {
                let hint = " (press Enter to expand)";
                let hint_row = if matches!(focused, Block::Thinking(_)) {
                    screen_row
                } else {
                    screen_row + height - 1
                };
                if hint_row >= term_area.top() && hint_row < term_area.bottom() {
                    let buf = frame.buffer_mut();
                    let mut text_end = term_area.left();
                    for col in term_area.left()..term_area.right() {
                        let ch = buf[(col, hint_row)].symbol();
                        if ch != " " && !ch.is_empty() {
                            text_end = col + 1;
                        }
                    }
                    for (i, ch) in hint.chars().enumerate() {
                        let col = text_end + i as u16;
                        if col < term_area.right() {
                            buf[(col, hint_row)].set_char(ch).set_fg(theme.hint_text);
                        }
                    }
                }
            }
        }
    } else if state.prompt_is_live && state.scroll_offset == 0 {
        if let Some(anchor) = state.prompt_anchor_row {
            let prompt_lines = state.input.line_count() as u16;
            let first_prompt_row = term_area.top() + anchor;
            let top_rule = first_prompt_row.saturating_sub(1);
            let bottom_rule = first_prompt_row + prompt_lines;
            let buf = frame.buffer_mut();
            render_horizontal_rules(buf, &[top_rule, bottom_rule], term_area, theme);
        }
    }
}

/// Draw `─────` separator rules on the given rows if they are blank.
fn render_horizontal_rules(buf: &mut Buffer, rows: &[u16], term_area: Rect, theme: &Theme) {
    for &row in rows {
        if row >= term_area.top() && row < term_area.bottom() {
            let is_blank = (term_area.left()..term_area.right()).all(|col| {
                let ch = buf[(col, row)].symbol();
                ch == " " || ch.is_empty()
            });
            if is_blank {
                for col in term_area.left()..term_area.right() {
                    buf[(col, row)].set_char('\u{2500}').set_fg(theme.separator);
                }
            }
        }
    }
}

/// Render the contextual hint text below the live prompt.
fn render_contextual_hint(buf: &mut Buffer, state: &TerminalState, theme: &Theme, term_area: Rect) {
    if state.prompt_is_live && state.scroll_offset == 0 {
        if let (Some(anchor), Some(hint)) = (state.prompt_anchor_row, current_hint(state)) {
            let prompt_lines = state.input.line_count() as u16;
            let bottom_rule = term_area.top() + anchor + prompt_lines;
            let hint_row = bottom_rule + 1;
            if hint_row >= term_area.top() && hint_row < term_area.bottom() {
                let start_x = term_area.left() + PROMPT_GUTTER_WIDTH;
                for (i, ch) in hint.chars().enumerate() {
                    let x = start_x + i as u16;
                    if x < term_area.right() {
                        buf[(x, hint_row)].set_char(ch).set_fg(theme.hint_text);
                    }
                }
            }
        }
    }
}

/// Render the status bar at the bottom of the screen.
pub(crate) fn render_status_bar(
    frame: &mut Frame,
    area: Rect,
    info: &StatusBarInfo,
) {
    let width = area.width as usize;

    let daemon_status = if info.daemon_connected && info.daemon_timestamp > 0 {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let age_s = now_ms.saturating_sub(info.daemon_timestamp) / 1000;
        if age_s < 30 {
            "daemon: connected"
        } else {
            "daemon: stale"
        }
    } else if info.daemon_connected {
        "daemon: connected"
    } else {
        "daemon: offline"
    };

    let running_indicator = if info.is_executing {
        format!(" {} running", info.anim.spinner_char())
    } else {
        String::new()
    };
    let model_part = if !info.model_alias.is_empty() && info.token_usage.context_window > 0 {
        format!(
            "{} {}/{} | ",
            info.model_alias,
            format_tokens(info.token_usage.context_used),
            format_tokens(info.token_usage.context_window)
        )
    } else if !info.model_alias.is_empty() {
        format!("{} | ", info.model_alias)
    } else {
        String::new()
    };
    let thinking_part = if info.thinking_enabled { "thinking | " } else { "" };
    let mode_part = match info.permission_mode {
        PermissionMode::Auto => "AUTO | ",
        PermissionMode::Manual => "MANUAL | ",
        PermissionMode::Bypass => "BYPASS | ",
    };
    let session_part = match info.session_name {
        Some(name) => format!("{} | ", name),
        None => String::new(),
    };
    let right = format!(
        " {}{}{}{}{} | v{} ",
        session_part, model_part, thinking_part, mode_part, daemon_status, VERSION
    );
    let left = match info.git_info {
        Some(git) => {
            let branch_part = format!("({})", git.branch);
            let stats_part = if git.files_changed > 0 {
                format!(
                    " ~{} +{} -{}",
                    git.files_changed, git.insertions, git.deletions
                )
            } else {
                String::new()
            };
            format!(
                " {} {}{}{} ",
                info.cwd, branch_part, stats_part, running_indicator
            )
        }
        None => format!(" {}{} ", info.cwd, running_indicator),
    };

    // Pad the middle so right-side text is right-aligned.
    let pad = width.saturating_sub(left.len() + right.len());
    let bar = format!("{}{}{}", left, " ".repeat(pad), right);

    let paragraph = Paragraph::new(Line::from(Span::styled(
        bar,
        Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
    )));

    frame.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::{completion_anchor_x, input_rendered_cols};

    #[test]
    fn input_rendered_cols_counts_characters_not_bytes() {
        assert_eq!(input_rendered_cols("abc"), 3);
        assert_eq!(input_rendered_cols("éñ"), 2);
        assert_eq!(input_rendered_cols("a🙂b"), 3);
    }

    #[test]
    fn completion_anchor_x_includes_gutter() {
        assert_eq!(completion_anchor_x(0, 0), 2);
        assert_eq!(completion_anchor_x(0, 5), 7);
        assert_eq!(completion_anchor_x(3, 4), 9);
    }
}
