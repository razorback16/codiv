//! Scrollable modal overlay for displaying full tool results.

use ratatui::prelude::*;
use ratatui::widgets::{Block as RatatuiBlock, Borders, Clear, Paragraph, Wrap};

use crate::ui::theme::Theme;

/// A scrollable modal that overlays the terminal to show full tool output.
pub struct ToolResultModal {
    visible: bool,
    title: String,
    lines: Vec<String>,
    is_diff: bool,
    scroll_offset: usize,
    entry_index: usize,
}

impl ToolResultModal {
    pub fn new() -> Self {
        Self {
            visible: false,
            title: String::new(),
            lines: Vec::new(),
            is_diff: false,
            scroll_offset: 0,
            entry_index: 0,
        }
    }

    pub fn open(&mut self, title: &str, content: &str, is_diff: bool, index: usize) {
        self.visible = true;
        self.title = title.to_string();
        self.lines = content.lines().map(|l| l.to_string()).collect();
        self.is_diff = is_diff;
        self.scroll_offset = 0;
        self.entry_index = index;
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
    }

    pub fn scroll_down(&mut self, n: usize) {
        let max = self.lines.len().saturating_sub(1);
        self.scroll_offset = (self.scroll_offset + n).min(max);
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let modal_area = centered_rect(80, 80, area);

        // Blank the background behind the modal.
        frame.render_widget(Clear, modal_area);

        let block = RatatuiBlock::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.tool_modal_border))
            .title(format!(" {} ", self.title))
            .title_bottom(" ↑↓ scroll  Esc close ");

        let inner = block.inner(modal_area);

        let text_lines: Vec<Line> = self
            .lines
            .iter()
            .map(|line| {
                if self.is_diff {
                    diff_styled_line(line)
                } else {
                    Line::from(Span::raw(line.as_str()))
                }
            })
            .collect();

        let paragraph = Paragraph::new(text_lines)
            .block(block)
            .scroll((self.scroll_offset as u16, 0))
            .wrap(Wrap { trim: false });

        frame.render_widget(paragraph, modal_area);

        // Scrollbar hint in top-right corner of inner area.
        if !self.lines.is_empty() {
            let visible_height = inner.height as usize;
            let total = self.lines.len();
            if total > visible_height {
                let pct = if total == 0 {
                    0
                } else {
                    (self.scroll_offset * 100) / total
                };
                let hint = format!(" {}% ", pct);
                let x = modal_area.right().saturating_sub(hint.len() as u16 + 1);
                let y = modal_area.top();
                if x >= modal_area.left() && y < modal_area.bottom() {
                    let buf = frame.buffer_mut();
                    for (i, ch) in hint.chars().enumerate() {
                        let col = x + i as u16;
                        if col < modal_area.right() {
                            buf[(col, y)]
                                .set_char(ch)
                                .set_fg(theme.tool_modal_scroll_hint);
                        }
                    }
                }
            }
        }
    }
}

/// Apply diff-aware styling to a single line.
fn diff_styled_line(line: &str) -> Line<'_> {
    let style = if line.starts_with("---") || line.starts_with("+++") {
        Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::BOLD)
    } else if line.starts_with("@@") {
        Style::default().fg(Color::Cyan)
    } else if line.starts_with('+') {
        Style::default().fg(Color::Green)
    } else if line.starts_with('-') {
        Style::default().fg(Color::Red)
    } else {
        Style::default()
    };
    Line::from(Span::styled(line, style))
}

/// Compute a centered sub-rectangle of `area`.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let width = (area.width as u32 * percent_x as u32 / 100).max(20) as u16;
    let height = (area.height as u32 * percent_y as u32 / 100).max(6) as u16;
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}
