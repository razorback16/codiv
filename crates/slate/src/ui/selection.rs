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
    /// so no separate scroll offset is needed.
    pub fn extract_text(
        &self,
        screen: &vt100::Screen,
        term_area_top: u16,
        term_area_height: u16,
    ) -> String {
        if !self.is_active() {
            return String::new();
        }

        let ((sc, sr), (ec, er)) = self.normalized_range();
        let screen_size = screen.size();
        let total_cols = screen_size.1;

        let mut result = String::new();

        for screen_row in sr..=er {
            // Only extract from the terminal area.
            if screen_row < term_area_top || screen_row >= term_area_top + term_area_height {
                continue;
            }

            let vt_row = (screen_row - term_area_top) as u16;

            if vt_row >= screen_size.0 {
                continue;
            }

            let col_start = if screen_row == sr { sc } else { 0 };
            let col_end = if screen_row == er {
                ec
            } else {
                total_cols.saturating_sub(1)
            };

            let mut line = String::new();
            for col in col_start..=col_end {
                if col >= total_cols {
                    break;
                }
                let cell = screen.cell(vt_row, col);
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
