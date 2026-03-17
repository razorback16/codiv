//! Line editor with history for the terminal input line.
//!
//! Handles character insertion, cursor movement, backspace/delete,
//! history navigation (up/down), and basic tab-completion.

/// A single-line input editor with command history.
pub struct InputLine {
    /// The current input buffer.
    buffer: String,
    /// Cursor position as a byte offset into `buffer`.
    cursor: usize,
    /// Command history (oldest first).
    history: Vec<String>,
    /// When browsing history, the index into `history`. `None` means we are
    /// editing the current (new) input, not a history entry.
    history_index: Option<usize>,
    /// The user's in-progress input saved when they start browsing history,
    /// so it can be restored when they press Down past the newest entry.
    saved_input: String,
}

impl InputLine {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor: 0,
            history: Vec::new(),
            history_index: None,
            saved_input: String::new(),
        }
    }

    /// Insert a character at the current cursor position.
    pub fn insert(&mut self, ch: char) {
        self.buffer.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    /// Delete the character before the cursor (backspace).
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        // Find the previous character boundary.
        let prev = self.prev_char_boundary();
        self.buffer.drain(prev..self.cursor);
        self.cursor = prev;
    }

    /// Delete the character at the cursor position (delete key).
    pub fn delete(&mut self) {
        if self.cursor >= self.buffer.len() {
            return;
        }
        let next = self.next_char_boundary();
        self.buffer.drain(self.cursor..next);
    }

    /// Move cursor one character to the left.
    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = self.prev_char_boundary();
        }
    }

    /// Move cursor one character to the right.
    pub fn move_right(&mut self) {
        if self.cursor < self.buffer.len() {
            self.cursor = self.next_char_boundary();
        }
    }

    /// Move cursor to the beginning of the current line.
    pub fn home(&mut self) {
        let before_cursor = &self.buffer[..self.cursor];
        self.cursor = match before_cursor.rfind('\n') {
            Some(pos) => pos + 1,
            None => 0,
        };
    }

    /// Move cursor to the end of the current line.
    pub fn end(&mut self) {
        let after_cursor = &self.buffer[self.cursor..];
        self.cursor = match after_cursor.find('\n') {
            Some(pos) => self.cursor + pos,
            None => self.buffer.len(),
        };
    }

    /// Clear the buffer and reset cursor to 0.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
    }

    /// Navigate to an older history entry (Up arrow).
    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }

        match self.history_index {
            None => {
                // First press: save current input, go to newest history entry.
                self.saved_input = self.buffer.clone();
                let idx = self.history.len() - 1;
                self.history_index = Some(idx);
                self.buffer = self.history[idx].clone();
                self.cursor = self.buffer.len();
            }
            Some(idx) if idx > 0 => {
                // Go to older entry.
                let new_idx = idx - 1;
                self.history_index = Some(new_idx);
                self.buffer = self.history[new_idx].clone();
                self.cursor = self.buffer.len();
            }
            _ => {
                // Already at oldest entry — do nothing.
            }
        }
    }

    /// Navigate to a newer history entry (Down arrow).
    pub fn history_down(&mut self) {
        match self.history_index {
            None => {
                // Not browsing history — do nothing.
            }
            Some(idx) => {
                if idx + 1 < self.history.len() {
                    // Go to newer entry.
                    let new_idx = idx + 1;
                    self.history_index = Some(new_idx);
                    self.buffer = self.history[new_idx].clone();
                    self.cursor = self.buffer.len();
                } else {
                    // Past newest entry — restore saved input.
                    self.history_index = None;
                    self.buffer = self.saved_input.clone();
                    self.cursor = self.buffer.len();
                }
            }
        }
    }

    /// Submit the current input: take the buffer, add it to history (if
    /// non-empty), clear the editor, and return the submitted string.
    pub fn submit(&mut self) -> String {
        let input = std::mem::take(&mut self.buffer);
        self.cursor = 0;
        self.history_index = None;
        self.saved_input.clear();

        let trimmed = input.trim();
        if !trimmed.is_empty() {
            // Avoid consecutive duplicates in history.
            if self.history.last().is_none_or(|last| last != trimmed) {
                self.history.push(trimmed.to_string());
            }
        }

        input
    }

    /// Return a reference to the current buffer content.
    pub fn content(&self) -> &str {
        &self.buffer
    }

    /// Return the cursor position as a character index (not byte offset).
    pub fn cursor_position(&self) -> usize {
        self.buffer[..self.cursor].chars().count()
    }

    /// Replace a byte range [start..end) in the buffer with `replacement`.
    /// Cursor is positioned at the end of the replacement text.
    pub fn replace_range(&mut self, start: usize, end: usize, replacement: &str) {
        let after = self.buffer[end..].to_string();
        self.buffer.truncate(start);
        self.buffer.push_str(replacement);
        self.cursor = self.buffer.len();
        self.buffer.push_str(&after);
    }

    /// Return the cursor position as a byte offset into the buffer.
    pub fn cursor_byte_offset(&self) -> usize {
        self.cursor
    }

    /// Clear all command history and reset the history browsing state.
    pub fn clear_history(&mut self) {
        self.history.clear();
        self.history_index = None;
        self.saved_input.clear();
    }

    /// Insert a newline character at the current cursor position.
    pub fn insert_newline(&mut self) {
        self.insert('\n');
    }

    /// Returns the number of lines in the buffer.
    pub fn line_count(&self) -> usize {
        self.buffer.split('\n').count()
    }

    /// Compute the (row, col) of the cursor within the multiline buffer.
    /// `row` is the zero-based line index, `col` is the character offset
    /// within that line.
    pub fn cursor_row_col(&self) -> (usize, usize) {
        let before_cursor = &self.buffer[..self.cursor];
        let row = before_cursor.matches('\n').count();
        let col = match before_cursor.rfind('\n') {
            Some(pos) => before_cursor[pos + 1..].chars().count(),
            None => before_cursor.chars().count(),
        };
        (row, col)
    }

    /// Returns an iterator over the lines of the buffer, split by `'\n'`.
    pub fn lines(&self) -> impl Iterator<Item = &str> + '_ {
        self.buffer.split('\n')
    }

    /// Move cursor up one row, keeping the same column (clamped to the
    /// previous line's length). Returns `true` if the cursor moved.
    pub fn move_up(&mut self) -> bool {
        let (row, col) = self.cursor_row_col();
        if row == 0 {
            return false;
        }
        self.set_cursor_to_row_col(row - 1, col);
        true
    }

    /// Move cursor down one row, keeping the same column (clamped to the
    /// next line's length). Returns `true` if the cursor moved.
    pub fn move_down(&mut self) -> bool {
        let (row, col) = self.cursor_row_col();
        if row + 1 >= self.line_count() {
            return false;
        }
        self.set_cursor_to_row_col(row + 1, col);
        true
    }

    /// Position cursor at the given (row, col), clamped to valid bounds.
    pub fn set_cursor_to_row_col(&mut self, row: usize, col: usize) {
        let lines: Vec<&str> = self.buffer.split('\n').collect();
        let row = row.min(lines.len().saturating_sub(1));
        // Compute byte offset of the start of the target row.
        let mut byte_offset = 0;
        for line in &lines[..row] {
            byte_offset += line.len() + 1; // +1 for the '\n'
        }
        // Clamp col to the char count of the target line.
        let target_line = lines[row];
        let clamped_col = col.min(target_line.chars().count());
        // Advance by clamped_col characters worth of bytes.
        let col_bytes: usize = target_line.chars().take(clamped_col).map(|c| c.len_utf8()).sum();
        self.cursor = byte_offset + col_bytes;
    }

    // --- Private helpers ---

    /// Find the byte offset of the previous character boundary before `self.cursor`.
    fn prev_char_boundary(&self) -> usize {
        let mut pos = self.cursor;
        if pos == 0 {
            return 0;
        }
        pos -= 1;
        while pos > 0 && !self.buffer.is_char_boundary(pos) {
            pos -= 1;
        }
        pos
    }

    /// Find the byte offset of the next character boundary after `self.cursor`.
    fn next_char_boundary(&self) -> usize {
        let mut pos = self.cursor + 1;
        while pos < self.buffer.len() && !self.buffer.is_char_boundary(pos) {
            pos += 1;
        }
        pos.min(self.buffer.len())
    }
}

impl Default for InputLine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_empty() {
        let input = InputLine::new();
        assert_eq!(input.content(), "");
        assert_eq!(input.cursor_position(), 0);
    }

    #[test]
    fn insert_chars() {
        let mut input = InputLine::new();
        input.insert('h');
        input.insert('i');
        assert_eq!(input.content(), "hi");
        assert_eq!(input.cursor_position(), 2);
    }

    #[test]
    fn insert_unicode() {
        let mut input = InputLine::new();
        input.insert('é');
        input.insert('ñ');
        assert_eq!(input.content(), "éñ");
        assert_eq!(input.cursor_position(), 2);
    }

    #[test]
    fn backspace() {
        let mut input = InputLine::new();
        input.insert('a');
        input.insert('b');
        input.insert('c');
        input.backspace();
        assert_eq!(input.content(), "ab");
        assert_eq!(input.cursor_position(), 2);
    }

    #[test]
    fn backspace_at_start_is_noop() {
        let mut input = InputLine::new();
        input.backspace();
        assert_eq!(input.content(), "");
    }

    #[test]
    fn delete_key() {
        let mut input = InputLine::new();
        input.insert('a');
        input.insert('b');
        input.insert('c');
        input.home();
        input.delete();
        assert_eq!(input.content(), "bc");
        assert_eq!(input.cursor_position(), 0);
    }

    #[test]
    fn delete_at_end_is_noop() {
        let mut input = InputLine::new();
        input.insert('a');
        input.delete();
        assert_eq!(input.content(), "a");
    }

    #[test]
    fn move_left_right() {
        let mut input = InputLine::new();
        input.insert('a');
        input.insert('b');
        input.insert('c');
        input.move_left();
        assert_eq!(input.cursor_position(), 2);
        input.move_left();
        assert_eq!(input.cursor_position(), 1);
        input.move_right();
        assert_eq!(input.cursor_position(), 2);
    }

    #[test]
    fn home_end() {
        let mut input = InputLine::new();
        input.insert('a');
        input.insert('b');
        input.insert('c');
        input.home();
        assert_eq!(input.cursor_position(), 0);
        input.end();
        assert_eq!(input.cursor_position(), 3);
    }

    #[test]
    fn insert_in_middle() {
        let mut input = InputLine::new();
        input.insert('a');
        input.insert('c');
        input.move_left();
        input.insert('b');
        assert_eq!(input.content(), "abc");
        assert_eq!(input.cursor_position(), 2);
    }

    #[test]
    fn submit_clears_and_returns() {
        let mut input = InputLine::new();
        input.insert('l');
        input.insert('s');
        let submitted = input.submit();
        assert_eq!(submitted, "ls");
        assert_eq!(input.content(), "");
        assert_eq!(input.cursor_position(), 0);
    }

    #[test]
    fn submit_adds_to_history() {
        let mut input = InputLine::new();
        input.insert('l');
        input.insert('s');
        input.submit();
        input.insert('p');
        input.insert('w');
        input.insert('d');
        input.submit();

        // Navigate history
        input.history_up();
        assert_eq!(input.content(), "pwd");
        input.history_up();
        assert_eq!(input.content(), "ls");
    }

    #[test]
    fn history_down_restores_input() {
        let mut input = InputLine::new();
        input.insert('l');
        input.insert('s');
        input.submit();

        // Type something, then navigate up, then back down.
        input.insert('n');
        input.insert('e');
        input.insert('w');
        input.history_up();
        assert_eq!(input.content(), "ls");
        input.history_down();
        assert_eq!(input.content(), "new");
    }

    #[test]
    fn history_up_on_empty_history_is_noop() {
        let mut input = InputLine::new();
        input.insert('x');
        input.history_up();
        assert_eq!(input.content(), "x");
    }

    #[test]
    fn history_avoids_consecutive_duplicates() {
        let mut input = InputLine::new();
        input.insert('l');
        input.insert('s');
        input.submit();
        input.insert('l');
        input.insert('s');
        input.submit();

        // Only one "ls" in history.
        input.history_up();
        assert_eq!(input.content(), "ls");
        // Going further up should stay (only one entry).
        input.history_up();
        assert_eq!(input.content(), "ls");
    }

    #[test]
    fn empty_submit_not_added_to_history() {
        let mut input = InputLine::new();
        input.submit(); // empty
        input.insert('l');
        input.insert('s');
        input.submit();

        input.history_up();
        assert_eq!(input.content(), "ls");
        input.history_up();
        // Still "ls" — no empty entry before it.
        assert_eq!(input.content(), "ls");
    }

    #[test]
    fn clear() {
        let mut input = InputLine::new();
        input.insert('h');
        input.insert('i');
        input.clear();
        assert_eq!(input.content(), "");
        assert_eq!(input.cursor_position(), 0);
    }

    #[test]
    fn replace_range_basic() {
        let mut input = InputLine::new();
        for c in "ls gi".chars() {
            input.insert(c);
        }
        input.replace_range(3, 5, "git ");
        assert_eq!(input.content(), "ls git ");
        assert_eq!(input.cursor_byte_offset(), 7);
    }

    #[test]
    fn replace_range_preserves_trailing_text() {
        let mut input = InputLine::new();
        for c in "ls foo bar".chars() {
            input.insert(c);
        }
        // Move cursor back to end of "foo" (byte 6)
        input.move_left(); // r
        input.move_left(); // a
        input.move_left(); // b
        input.move_left(); // ' '
        // cursor is now at byte 6, after "foo"
        input.replace_range(3, 6, "foobar");
        assert_eq!(input.content(), "ls foobar bar");
    }

    #[test]
    fn replace_range_at_start() {
        let mut input = InputLine::new();
        for c in "gi".chars() {
            input.insert(c);
        }
        input.replace_range(0, 2, "git ");
        assert_eq!(input.content(), "git ");
        assert_eq!(input.cursor_byte_offset(), 4);
    }

    #[test]
    fn cursor_byte_offset_matches_cursor() {
        let mut input = InputLine::new();
        for c in "hello".chars() {
            input.insert(c);
        }
        assert_eq!(input.cursor_byte_offset(), 5);
        input.move_left();
        assert_eq!(input.cursor_byte_offset(), 4);
        input.home();
        assert_eq!(input.cursor_byte_offset(), 0);
    }

    // --- Multiline tests ---

    #[test]
    fn insert_newline_basic() {
        let mut input = InputLine::new();
        for c in "hello".chars() {
            input.insert(c);
        }
        input.insert_newline();
        for c in "world".chars() {
            input.insert(c);
        }
        assert_eq!(input.content(), "hello\nworld");
    }

    #[test]
    fn line_count_single() {
        let mut input = InputLine::new();
        for c in "hello".chars() {
            input.insert(c);
        }
        assert_eq!(input.line_count(), 1);
    }

    #[test]
    fn line_count_multi() {
        let mut input = InputLine::new();
        for c in "a".chars() {
            input.insert(c);
        }
        input.insert_newline();
        for c in "b".chars() {
            input.insert(c);
        }
        input.insert_newline();
        for c in "c".chars() {
            input.insert(c);
        }
        assert_eq!(input.line_count(), 3);
    }

    #[test]
    fn cursor_row_col_first_line() {
        let mut input = InputLine::new();
        for c in "hello".chars() {
            input.insert(c);
        }
        assert_eq!(input.cursor_row_col(), (0, 5));
        input.move_left();
        input.move_left();
        assert_eq!(input.cursor_row_col(), (0, 3));
    }

    #[test]
    fn cursor_row_col_second_line() {
        let mut input = InputLine::new();
        for c in "hello".chars() {
            input.insert(c);
        }
        input.insert_newline();
        for c in "ab".chars() {
            input.insert(c);
        }
        assert_eq!(input.cursor_row_col(), (1, 2));
    }

    #[test]
    fn move_up_from_second_line() {
        let mut input = InputLine::new();
        for c in "hello".chars() {
            input.insert(c);
        }
        input.insert_newline();
        for c in "ab".chars() {
            input.insert(c);
        }
        assert!(input.move_up());
        assert_eq!(input.cursor_row_col(), (0, 2));
    }

    #[test]
    fn move_up_from_first_line() {
        let mut input = InputLine::new();
        for c in "hello".chars() {
            input.insert(c);
        }
        assert!(!input.move_up());
    }

    #[test]
    fn move_down_from_first_line() {
        let mut input = InputLine::new();
        for c in "hello".chars() {
            input.insert(c);
        }
        input.insert_newline();
        for c in "world".chars() {
            input.insert(c);
        }
        // Move to first line
        input.set_cursor_to_row_col(0, 3);
        assert!(input.move_down());
        assert_eq!(input.cursor_row_col(), (1, 3));
    }

    #[test]
    fn move_down_from_last_line() {
        let mut input = InputLine::new();
        for c in "hello".chars() {
            input.insert(c);
        }
        input.insert_newline();
        for c in "world".chars() {
            input.insert(c);
        }
        assert!(!input.move_down());
    }

    #[test]
    fn move_up_clamps_col() {
        let mut input = InputLine::new();
        for c in "ab".chars() {
            input.insert(c);
        }
        input.insert_newline();
        for c in "longline".chars() {
            input.insert(c);
        }
        // Cursor is at (1, 8). Moving up should clamp col to 2.
        assert!(input.move_up());
        assert_eq!(input.cursor_row_col(), (0, 2));
    }

    #[test]
    fn home_multiline() {
        let mut input = InputLine::new();
        for c in "hello".chars() {
            input.insert(c);
        }
        input.insert_newline();
        for c in "world".chars() {
            input.insert(c);
        }
        // Cursor is at end of "world". Home should go to start of "world".
        input.home();
        assert_eq!(input.cursor_row_col(), (1, 0));
        // Should NOT be at byte 0 (absolute start).
        assert_eq!(input.cursor_byte_offset(), 6); // "hello\n" = 6 bytes
    }

    #[test]
    fn end_multiline() {
        let mut input = InputLine::new();
        for c in "hello".chars() {
            input.insert(c);
        }
        input.insert_newline();
        for c in "world".chars() {
            input.insert(c);
        }
        // Move to start of first line.
        input.set_cursor_to_row_col(0, 0);
        // End should go to end of first line, not absolute end.
        input.end();
        assert_eq!(input.cursor_row_col(), (0, 5));
        assert_eq!(input.cursor_byte_offset(), 5);
    }

    #[test]
    fn set_cursor_to_row_col_basic() {
        let mut input = InputLine::new();
        for c in "hello".chars() {
            input.insert(c);
        }
        input.insert_newline();
        for c in "world".chars() {
            input.insert(c);
        }
        input.set_cursor_to_row_col(0, 3);
        assert_eq!(input.cursor_row_col(), (0, 3));
        input.set_cursor_to_row_col(1, 2);
        assert_eq!(input.cursor_row_col(), (1, 2));
    }

    #[test]
    fn set_cursor_to_row_col_clamped() {
        let mut input = InputLine::new();
        for c in "hi".chars() {
            input.insert(c);
        }
        input.insert_newline();
        for c in "there".chars() {
            input.insert(c);
        }
        // Row out of bounds: clamp to last row.
        input.set_cursor_to_row_col(99, 1);
        assert_eq!(input.cursor_row_col(), (1, 1));
        // Col out of bounds: clamp to line length.
        input.set_cursor_to_row_col(0, 99);
        assert_eq!(input.cursor_row_col(), (0, 2));
    }
}
