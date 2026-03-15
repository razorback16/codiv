//! Completion popup — floating dropdown for tab-completion candidates.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};

/// Maximum number of visible rows in the popup.
const MAX_VISIBLE_ROWS: usize = 10;

/// State of the tab-completion popup.
pub struct CompletionPopup {
    /// The candidate strings to display.
    candidates: Vec<String>,
    /// Currently selected index (0-based).
    selected: usize,
    /// The byte range in the input buffer that completions replace.
    replace_start: usize,
    replace_end: usize,
    /// Whether the popup is currently visible.
    visible: bool,
}

impl CompletionPopup {
    pub fn new() -> Self {
        Self {
            candidates: Vec::new(),
            selected: 0,
            replace_start: 0,
            replace_end: 0,
            visible: false,
        }
    }

    /// Open the popup with the given candidates and replacement range.
    pub fn open(&mut self, candidates: Vec<String>, replace_start: usize, replace_end: usize) {
        if candidates.is_empty() {
            self.visible = false;
            return;
        }
        self.candidates = candidates;
        self.selected = 0;
        self.replace_start = replace_start;
        self.replace_end = replace_end;
        self.visible = true;
    }

    /// Close the popup without selecting anything.
    pub fn dismiss(&mut self) {
        self.visible = false;
        self.candidates.clear();
    }

    /// Is the popup currently visible?
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Move selection up (wraps to bottom).
    pub fn select_prev(&mut self) {
        if !self.visible || self.candidates.is_empty() {
            return;
        }
        if self.selected > 0 {
            self.selected -= 1;
        } else {
            self.selected = self.candidates.len() - 1;
        }
    }

    /// Move selection down (wraps to top).
    pub fn select_next(&mut self) {
        if !self.visible || self.candidates.is_empty() {
            return;
        }
        if self.selected + 1 < self.candidates.len() {
            self.selected += 1;
        } else {
            self.selected = 0;
        }
    }

    /// Confirm the current selection. Returns (candidate, replace_start, replace_end).
    pub fn confirm(&mut self) -> Option<(String, usize, usize)> {
        if !self.visible || self.candidates.is_empty() {
            return None;
        }
        let result = self.candidates[self.selected].clone();
        let start = self.replace_start;
        let end = self.replace_end;
        self.dismiss();
        Some((result, start, end))
    }

    /// Render the popup as a floating overlay.
    ///
    /// `anchor_x` is the column of the word start, `anchor_y` is the row
    /// just below where the popup should appear (typically the input line row).
    pub fn render(&self, frame: &mut Frame, anchor_x: u16, anchor_y: u16) {
        if !self.visible || self.candidates.is_empty() {
            return;
        }

        let visible_count = self.candidates.len().min(MAX_VISIBLE_ROWS);
        let scroll_offset = self.scroll_offset(visible_count);

        let max_candidate_width = self
            .candidates
            .iter()
            .map(|c| c.len())
            .max()
            .unwrap_or(10)
            .clamp(10, 60);

        let popup_width = max_candidate_width as u16 + 4; // padding + border
        let popup_height = visible_count as u16 + 2; // border top + bottom

        let screen = frame.area();

        // Place below the anchor point. If not enough space below, place above.
        let space_below = screen.height.saturating_sub(anchor_y + 1);
        let y = if space_below >= popup_height {
            anchor_y + 1
        } else if anchor_y >= popup_height {
            anchor_y - popup_height
        } else {
            // Neither direction has enough space; place below and clip.
            anchor_y + 1
        };

        let x = anchor_x.min(screen.width.saturating_sub(popup_width));

        let area = Rect::new(
            x,
            y,
            popup_width.min(screen.width.saturating_sub(x)),
            popup_height.min(screen.height.saturating_sub(y)),
        );

        // Clear background behind popup.
        frame.render_widget(Clear, area);

        let items: Vec<ListItem> = self
            .candidates
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(visible_count)
            .map(|(i, candidate)| {
                let style = if i == self.selected {
                    Style::default().bg(Color::Blue).fg(Color::White)
                } else {
                    Style::default().fg(Color::White).bg(Color::DarkGray)
                };
                ListItem::new(Line::from(Span::styled(format!(" {} ", candidate), style)))
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Gray))
                .style(Style::default().bg(Color::DarkGray)),
        );

        frame.render_widget(list, area);
    }

    /// Compute scroll offset so the selected item is always visible.
    fn scroll_offset(&self, visible_count: usize) -> usize {
        if self.selected < visible_count {
            0
        } else {
            self.selected - visible_count + 1
        }
    }
}

impl Default for CompletionPopup {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_and_visible() {
        let mut popup = CompletionPopup::new();
        assert!(!popup.is_visible());
        popup.open(vec!["alpha".into(), "beta".into()], 0, 2);
        assert!(popup.is_visible());
    }

    #[test]
    fn open_empty_not_visible() {
        let mut popup = CompletionPopup::new();
        popup.open(vec![], 0, 0);
        assert!(!popup.is_visible());
    }

    #[test]
    fn navigate_down_and_up() {
        let mut popup = CompletionPopup::new();
        popup.open(vec!["a".into(), "b".into(), "c".into()], 0, 1);
        assert_eq!(popup.selected, 0);

        popup.select_next();
        assert_eq!(popup.selected, 1);

        popup.select_next();
        assert_eq!(popup.selected, 2);

        // Wrap to top.
        popup.select_next();
        assert_eq!(popup.selected, 0);
    }

    #[test]
    fn navigate_up_wraps_to_bottom() {
        let mut popup = CompletionPopup::new();
        popup.open(vec!["a".into(), "b".into(), "c".into()], 0, 1);
        popup.select_prev();
        assert_eq!(popup.selected, 2);
    }

    #[test]
    fn confirm_returns_selected() {
        let mut popup = CompletionPopup::new();
        popup.open(vec!["alpha".into(), "beta".into()], 5, 7);
        popup.select_next(); // select "beta"
        let (val, start, end) = popup.confirm().unwrap();
        assert_eq!(val, "beta");
        assert_eq!(start, 5);
        assert_eq!(end, 7);
        assert!(!popup.is_visible());
    }

    #[test]
    fn confirm_empty_returns_none() {
        let mut popup = CompletionPopup::new();
        assert!(popup.confirm().is_none());
    }

    #[test]
    fn dismiss_clears_state() {
        let mut popup = CompletionPopup::new();
        popup.open(vec!["a".into()], 0, 1);
        popup.dismiss();
        assert!(!popup.is_visible());
        assert!(popup.candidates.is_empty());
    }

    #[test]
    fn scroll_offset_basic() {
        let popup = CompletionPopup {
            candidates: (0..20).map(|i| format!("item{}", i)).collect(),
            selected: 15,
            replace_start: 0,
            replace_end: 0,
            visible: true,
        };
        let offset = popup.scroll_offset(10);
        assert_eq!(offset, 6); // 15 - 10 + 1
    }

    #[test]
    fn scroll_offset_no_scroll_needed() {
        let popup = CompletionPopup {
            candidates: vec!["a".into(), "b".into(), "c".into()],
            selected: 2,
            replace_start: 0,
            replace_end: 0,
            visible: true,
        };
        let offset = popup.scroll_offset(10);
        assert_eq!(offset, 0);
    }
}
