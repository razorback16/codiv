use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use tui_term::widget::{Cursor as PtCursor, PseudoTerminal};

use super::utils::format_tokens;
use super::utils::true_scrollback_len;
use super::{PROMPT_GUTTER_WIDTH, STATUS_BAR_HEIGHT};
use crate::shell::bash_coprocess::GitInfo;
use crate::ui::completion_popup::CompletionPopup;
use crate::ui::input::InputLine;
use crate::ui::selection::TextSelection;
use crate::ui::tool_modal::ToolResultModal;
use crate::{
    ui::blocks::{Block, BlockRegistry},
    VERSION,
};

#[inline]
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

pub(crate) fn render_frame(
    term: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    parser: &mut vt100::Parser,
    input: &InputLine,
    cwd: &str,
    daemon_connected: bool,
    daemon_timestamp: u64,
    scroll_offset: usize,
    prompt_is_live: &mut bool,
    is_executing: bool,
    agent_streaming: bool,
    completion_popup: &CompletionPopup,
    selection: &TextSelection,
    git_info: Option<&GitInfo>,
    model_alias: &str,
    context_usage: (usize, usize),
    tracker: &BlockRegistry,
    tool_result_modal: &ToolResultModal,
) -> Result<(), Box<dyn std::error::Error>> {
    // Write live prompt into the vt100 parser (only when scrolled to bottom
    // and no command is currently executing or agent streaming, and not in alt screen).
    let in_alt_screen = parser.screen().alternate_screen();
    if scroll_offset == 0 && !is_executing && !agent_streaming && !in_alt_screen {
        // If cursor is on the last row, scroll up to make room for bottom ruler line.
        let screen_rows = parser.screen().size().0;
        let (cursor_row, _) = parser.screen().cursor_position();
        if cursor_row + 1 >= screen_rows {
            parser.process(b"\n\x1b[A");
        }

        let input_text = input.content();

        // Write the input text; visual indent comes from content_area offset.
        parser.process(format!("\r\x1b[K{}", input_text).as_bytes());

        // Position cursor: move back from end if cursor isn't at end of input.
        let target_col = input.cursor_position();
        let current_col = input_rendered_cols(input_text);
        if current_col > target_col {
            parser.process(format!("\x1b[{}D", current_col - target_col).as_bytes());
        }

        *prompt_is_live = true;
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
            let cursor_visible = scroll_offset == 0;
            let pseudo_term = PseudoTerminal::new(parser.screen())
                .cursor(PtCursor::default().visibility(cursor_visible));
            frame.render_widget(pseudo_term, content_area);

            // --- Render `>` prompt gutter for past and live prompts ---
            {
                // Compute the absolute line range currently visible on screen.
                let screen_rows = parser.screen().size().0 as u64;
                let sb_len = true_scrollback_len(parser) as u64;
                // abs_bottom = first absolute line BELOW the visible area (at scroll_offset==0)
                let abs_bottom = sb_len + screen_rows;
                // when scrolled up, abs_top shifts back by scroll_offset rows
                let abs_top = abs_bottom
                    .saturating_sub(screen_rows)
                    .saturating_sub(scroll_offset as u64);
                let abs_view_bottom = abs_top + screen_rows;

                let buf = frame.buffer_mut();
                for block in tracker.blocks() {
                    let (scrollback_line, gutter_char, gutter_fg) = match block {
                        Block::Prompt(pb) => (pb.scrollback_line, '>', Color::Cyan),
                        Block::AiResponse(ab) => (ab.scrollback_line, '\u{25CF}', Color::White),
                        Block::Tool(tb) => (tb.scrollback_line, '\u{25CF}', Color::Green),
                        Block::CmdResponse(cb) => (cb.scrollback_line, '$', Color::White),
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
                // Live prompt: draw `>` at the current cursor row.
                if *prompt_is_live && scroll_offset == 0 {
                    let (cursor_row, _) = parser.screen().cursor_position();
                    let row = term_area.top() + cursor_row;
                    if row < term_area.bottom() {
                        buf[(term_area.left(), row)]
                            .set_char('>')
                            .set_fg(Color::Cyan);
                    }
                }
            }

            // --- Render selection highlight ---
            if selection.is_active() {
                let ((sc, sr), (ec, er)) = selection.normalized_range();
                let buf = frame.buffer_mut();
                for row in sr..=er {
                    if row < term_area.top() || row >= term_area.bottom() {
                        continue;
                    }
                    let col_start = if row == sr { sc } else { content_area.left() };
                    let col_end = if row == er {
                        ec
                    } else {
                        content_area.right().saturating_sub(1)
                    };
                    for col in col_start..=col_end {
                        if col < content_area.left() {
                            continue;
                        }
                        if col >= content_area.right() {
                            break;
                        }
                        let cell = &mut buf[(col, row)];
                        let fg = cell.fg;
                        let bg = cell.bg;
                        cell.fg = if bg == Color::Reset { Color::Black } else { bg };
                        cell.bg = if fg == Color::Reset { Color::White } else { fg };
                    }
                }
            }

            // --- Render status bar ---
            render_status_bar(
                frame,
                cwd,
                daemon_connected,
                daemon_timestamp,
                is_executing,
                status_area,
                git_info,
                model_alias,
                context_usage,
            );

            // --- Render completion popup ---
            if completion_popup.is_visible() {
                let (cursor_row, _cursor_col) = parser.screen().cursor_position();
                let anchor_x = completion_anchor_x(term_area.left(), input.cursor_position());
                let anchor_y = term_area.top() + cursor_row;
                completion_popup.render(frame, anchor_x, anchor_y);
            }

            // --- Render block selection overlay ---
            if let Some(focused) = tracker.focused() {
                let (scrollback_line, line_count) = match focused {
                    Block::Tool(tb) => (tb.scrollback_line, tb.line_count),
                    Block::Prompt(pb) => (pb.scrollback_line, 1),
                    Block::CmdResponse(cb) => (cb.scrollback_line, cb.line_count),
                    Block::AiResponse(ab) => (ab.scrollback_line, ab.line_count),
                };
                let sb_len = true_scrollback_len(parser) as u64;
                let screen_rows = parser.screen().size().0 as u64;
                let abs_bottom = sb_len + screen_rows;
                let abs_top = abs_bottom
                    .saturating_sub(screen_rows)
                    .saturating_sub(scroll_offset as u64);
                if scrollback_line >= abs_top && scrollback_line < abs_bottom {
                    let screen_row = (scrollback_line - abs_top) as u16 + term_area.top();
                    let top_rule = screen_row.saturating_sub(1);
                    let bottom_rule = screen_row + line_count;
                    let buf = frame.buffer_mut();
                    for row in [top_rule, bottom_rule] {
                        if row >= term_area.top() && row < term_area.bottom() {
                            let is_blank = (term_area.left()..term_area.right()).all(|col| {
                                let ch = buf[(col, row)].symbol();
                                ch == " " || ch == ""
                            });
                            if is_blank {
                                for col in term_area.left()..term_area.right() {
                                    let cell = &mut buf[(col, row)];
                                    cell.set_char('\u{2500}');
                                    cell.set_fg(Color::DarkGray);
                                }
                            }
                        }
                    }
                    // Show "(press Enter to expand)" hint for ToolBlocks
                    if let Block::Tool(_) = focused {
                        let hint = " (press Enter to expand)";
                        let hint_row = screen_row + 1; // summary line
                        if hint_row >= term_area.top() && hint_row < term_area.bottom() {
                            // Find end of existing text
                            let buf = frame.buffer_mut();
                            let mut text_end = term_area.left();
                            for col in term_area.left()..term_area.right() {
                                let ch = buf[(col, hint_row)].symbol();
                                if ch != " " && ch != "" {
                                    text_end = col + 1;
                                }
                            }
                            for (i, ch) in hint.chars().enumerate() {
                                let col = text_end + i as u16;
                                if col < term_area.right() {
                                    buf[(col, hint_row)].set_char(ch).set_fg(Color::DarkGray);
                                }
                            }
                        }
                    }
                }
            } else if *prompt_is_live && scroll_offset == 0 {
                // Live prompt is implicitly selected — draw ───── rules
                let (cursor_row, _) = parser.screen().cursor_position();
                let screen_row = term_area.top() + cursor_row;
                let top_rule = screen_row.saturating_sub(1);
                let bottom_rule = screen_row + 1;
                let buf = frame.buffer_mut();
                for row in [top_rule, bottom_rule] {
                    if row >= term_area.top() && row < term_area.bottom() {
                        let is_blank = (term_area.left()..term_area.right()).all(|col| {
                            let ch = buf[(col, row)].symbol();
                            ch == " " || ch == ""
                        });
                        if is_blank {
                            for col in term_area.left()..term_area.right() {
                                buf[(col, row)].set_char('\u{2500}').set_fg(Color::DarkGray);
                            }
                        }
                    }
                }
            }

            // --- Render tool result modal ---
            if tool_result_modal.is_visible() {
                tool_result_modal.render(frame, area);
            }
        }
    })?;

    Ok(())
}

/// Render the status bar at the bottom of the screen.
pub(crate) fn render_status_bar(
    frame: &mut Frame,
    cwd: &str,
    daemon_connected: bool,
    daemon_timestamp: u64,
    is_executing: bool,
    area: Rect,
    git_info: Option<&GitInfo>,
    model_alias: &str,
    context_usage: (usize, usize),
) {
    let width = area.width as usize;

    let daemon_status = if daemon_connected && daemon_timestamp > 0 {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let age_s = now_ms.saturating_sub(daemon_timestamp) / 1000;
        if age_s < 30 {
            "daemon: connected"
        } else {
            "daemon: stale"
        }
    } else if daemon_connected {
        "daemon: connected"
    } else {
        "daemon: offline"
    };

    let running_indicator = if is_executing { " [running]" } else { "" };
    let model_part = if !model_alias.is_empty() {
        format!(
            "{} {}/{} | ",
            model_alias,
            format_tokens(context_usage.0),
            format_tokens(context_usage.1)
        )
    } else {
        String::new()
    };
    let right = format!(" {}{} | {} ", model_part, daemon_status, VERSION);
    let left = match git_info {
        Some(info) => {
            let branch_part = format!("({})", info.branch);
            let stats_part = if info.files_changed > 0 {
                format!(
                    " ~{} +{} -{}",
                    info.files_changed, info.insertions, info.deletions
                )
            } else {
                String::new()
            };
            format!(
                " {} {}{}{} ",
                cwd, branch_part, stats_part, running_indicator
            )
        }
        None => format!(" {}{} ", cwd, running_indicator),
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
