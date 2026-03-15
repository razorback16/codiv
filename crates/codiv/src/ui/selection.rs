//! Text selection state for click-drag copy in the terminal UI.

/// Tracks a rectangular text selection defined by screen coordinates.
pub struct TextSelection {
    /// Starting position (column, row) in absolute screen coordinates.
    start: (u16, u16),
    /// Current end position (column, row) in absolute screen coordinates.
    end: (u16, u16),
    /// Whether a completed (non-dragging) selection exists.
    active: bool,
    /// Whether the user is currently dragging (mouse button held).
    dragging: bool,
}

impl TextSelection {
    pub fn new() -> Self {
        Self {
            start: (0, 0),
            end: (0, 0),
            active: false,
            dragging: false,
        }
    }

    /// Begin a new selection at the given screen coordinate.
    pub fn start_at(&mut self, col: u16, row: u16) {
        self.start = (col, row);
        self.end = (col, row);
        self.dragging = true;
        self.active = false;
    }

    /// Update the end position while dragging.
    pub fn update(&mut self, col: u16, row: u16) {
        if self.dragging {
            self.end = (col, row);
            self.active = true;
        }
    }

    /// Finalize the selection (mouse released).
    pub fn finish(&mut self) {
        self.dragging = false;
        // Only keep the selection active if it spans at least one cell.
        if self.start == self.end {
            self.active = false;
        }
    }

    /// Clear any selection.
    pub fn clear(&mut self) {
        self.active = false;
        self.dragging = false;
    }

    /// Returns `true` if there is a visible selection (dragging or finalized).
    pub fn is_active(&self) -> bool {
        self.active || self.dragging
    }

    /// Return the selection range normalized so that `start <= end` in
    /// reading order (top-to-bottom, left-to-right).
    /// Returns `((start_col, start_row), (end_col, end_row))`.
    pub fn normalized_range(&self) -> ((u16, u16), (u16, u16)) {
        let (s, e) = (self.start, self.end);
        if s.1 < e.1 || (s.1 == e.1 && s.0 <= e.0) {
            (s, e)
        } else {
            (e, s)
        }
    }

    /// Extract the selected text from the vt100 screen.
    ///
    /// `term_area_top` is the screen row where the terminal area begins
    /// (usually 0). The vt100 screen already has `set_scrollback` applied,
    /// so no separate scroll offset is needed. `content_area_left` is the
    /// terminal content x-offset (used to skip the prompt gutter).
    pub fn extract_text(
        &self,
        screen: &vt100::Screen,
        term_area_top: u16,
        term_area_height: u16,
        content_area_left: u16,
    ) -> String {
        if !self.is_active() {
            return String::new();
        }

        let ((sc, sr), (ec, er)) = self.normalized_range();
        let screen_size = screen.size();
        let total_cols = screen_size.1;
        let content_right = content_area_left.saturating_add(total_cols.saturating_sub(1));

        let mut result = String::new();

        for screen_row in sr..=er {
            // Only extract from the terminal area.
            if screen_row < term_area_top
                || screen_row >= term_area_top.saturating_add(term_area_height)
            {
                continue;
            }

            let vt_row = screen_row - term_area_top;

            if vt_row >= screen_size.0 {
                continue;
            }

            let row_col_start = if screen_row == sr {
                sc
            } else {
                content_area_left
            };
            let row_col_end = if screen_row == er { ec } else { content_right };
            let col_start = row_col_start.max(content_area_left);
            let col_end = row_col_end.min(content_right);
            if col_start > col_end {
                continue;
            }

            let mut line = String::new();
            for screen_col in col_start..=col_end {
                let vt_col = screen_col.saturating_sub(content_area_left);
                let cell = screen.cell(vt_row, vt_col);
                if let Some(cell) = cell {
                    line.push(cell.contents().chars().next().unwrap_or(' '));
                } else {
                    line.push(' ');
                }
            }

            // Trim trailing spaces from each line.
            let trimmed = line.trim_end();
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(trimmed);
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::TextSelection;

    #[test]
    fn extract_text_applies_content_left_offset() {
        let mut parser = vt100::Parser::new(1, 10, 0);
        parser.process(b"abcdefghij");

        let mut selection = TextSelection::new();
        selection.start_at(4, 0);
        selection.update(6, 0);
        selection.finish();

        let text = selection.extract_text(parser.screen(), 0, 1, 2);
        assert_eq!(text, "cde");
    }

    #[test]
    fn extract_text_clamps_left_edge_to_content_area() {
        let mut parser = vt100::Parser::new(1, 10, 0);
        parser.process(b"abcdefghij");

        let mut selection = TextSelection::new();
        selection.start_at(0, 0);
        selection.update(3, 0);
        selection.finish();

        let text = selection.extract_text(parser.screen(), 0, 1, 2);
        assert_eq!(text, "ab");
    }
}
