//! Scrollback buffer for the linear scroll-down terminal UI.
//!
//! Stores styled lines and provides viewport windowing with scroll support.

/// Visual style for a line in the scroll buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineStyle {
    /// Default output text.
    Normal,
    /// User-typed command (bold + cyan).
    Command,
    /// Error output (red).
    Error,
    /// System messages (dim / gray).
    System,
    /// Prompt line (green + bold).
    Prompt,
}

/// A single styled line in the scroll buffer.
#[derive(Debug, Clone)]
pub struct StyledLine {
    pub content: String,
    pub style: LineStyle,
}

impl StyledLine {
    pub fn new(content: impl Into<String>, style: LineStyle) -> Self {
        Self {
            content: content.into(),
            style,
        }
    }
}

/// A scrollback buffer that stores lines and provides a viewport window.
///
/// The viewport is anchored to the bottom by default. When the user scrolls
/// up, `viewport_offset` increases (counting lines from the bottom). New
/// content resets the viewport to the bottom (offset 0) so the user always
/// sees fresh output.
pub struct ScrollBuffer {
    lines: Vec<StyledLine>,
    /// Number of lines scrolled up from the bottom. 0 means "at bottom".
    viewport_offset: usize,
    /// Maximum number of lines to retain (scrollback limit).
    max_lines: usize,
}

#[allow(dead_code)]
impl ScrollBuffer {
    /// Create a new scroll buffer with the given scrollback limit.
    pub fn new(max_lines: usize) -> Self {
        Self {
            lines: Vec::new(),
            viewport_offset: 0,
            max_lines,
        }
    }

    /// Push a single styled line. Trims oldest lines if over `max_lines`.
    /// Auto-scrolls to bottom (resets viewport_offset to 0).
    pub fn push(&mut self, line: StyledLine) {
        self.lines.push(line);
        if self.lines.len() > self.max_lines {
            let excess = self.lines.len() - self.max_lines;
            self.lines.drain(..excess);
        }
        // Auto-scroll: new content always brings us to the bottom.
        self.viewport_offset = 0;
    }

    /// Convenience: split `content` on newlines and push each as a `StyledLine`
    /// with the given style. An empty string pushes one empty line.
    pub fn push_str(&mut self, content: &str, style: LineStyle) {
        if content.is_empty() {
            self.push(StyledLine::new("", style.clone()));
            return;
        }
        for line in content.split('\n') {
            self.push(StyledLine::new(line, style.clone()));
        }
    }

    /// Return the slice of lines visible in a viewport of the given `height`.
    ///
    /// The viewport is anchored from the bottom: offset 0 shows the most
    /// recent lines, higher offsets show older lines.
    pub fn viewport(&self, height: usize) -> &[StyledLine] {
        if self.lines.is_empty() || height == 0 {
            return &[];
        }

        let total = self.lines.len();
        // The "end" index is total minus viewport_offset, clamped.
        let end = total.saturating_sub(self.viewport_offset).max(1);
        let start = end.saturating_sub(height);

        &self.lines[start..end]
    }

    /// Scroll up by `n` lines (show older content).
    /// `viewport_height` is the number of visible lines so we clamp correctly.
    pub fn scroll_up(&mut self, n: usize, viewport_height: usize) {
        let max_offset = self.lines.len().saturating_sub(viewport_height);
        self.viewport_offset = (self.viewport_offset + n).min(max_offset);
    }

    /// Scroll down by `n` lines (show newer content).
    pub fn scroll_down(&mut self, n: usize) {
        self.viewport_offset = self.viewport_offset.saturating_sub(n);
    }

    /// Reset viewport to the bottom (most recent content).
    pub fn scroll_to_bottom(&mut self) {
        self.viewport_offset = 0;
    }

    /// Returns `true` if the viewport is at the bottom.
    pub fn is_at_bottom(&self) -> bool {
        self.viewport_offset == 0
    }

    /// Total number of lines in the buffer.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Returns `true` if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_buffer_is_empty() {
        let buf = ScrollBuffer::new(100);
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
        assert!(buf.is_at_bottom());
    }

    #[test]
    fn push_adds_lines() {
        let mut buf = ScrollBuffer::new(100);
        buf.push(StyledLine::new("hello", LineStyle::Normal));
        buf.push(StyledLine::new("world", LineStyle::Normal));
        assert_eq!(buf.len(), 2);
    }

    #[test]
    fn push_str_splits_on_newlines() {
        let mut buf = ScrollBuffer::new(100);
        buf.push_str("line1\nline2\nline3", LineStyle::Normal);
        assert_eq!(buf.len(), 3);
    }

    #[test]
    fn push_str_empty_pushes_one() {
        let mut buf = ScrollBuffer::new(100);
        buf.push_str("", LineStyle::Normal);
        assert_eq!(buf.len(), 1);
    }

    #[test]
    fn max_lines_trims_oldest() {
        let mut buf = ScrollBuffer::new(3);
        for i in 0..5 {
            buf.push(StyledLine::new(format!("line {}", i), LineStyle::Normal));
        }
        assert_eq!(buf.len(), 3);
        // Oldest lines (0, 1) should be gone; remaining: 2, 3, 4
        let vp = buf.viewport(10);
        assert_eq!(vp[0].content, "line 2");
        assert_eq!(vp[2].content, "line 4");
    }

    #[test]
    fn viewport_returns_tail() {
        let mut buf = ScrollBuffer::new(100);
        for i in 0..10 {
            buf.push(StyledLine::new(format!("line {}", i), LineStyle::Normal));
        }
        let vp = buf.viewport(3);
        assert_eq!(vp.len(), 3);
        assert_eq!(vp[0].content, "line 7");
        assert_eq!(vp[1].content, "line 8");
        assert_eq!(vp[2].content, "line 9");
    }

    #[test]
    fn scroll_up_shows_older() {
        let mut buf = ScrollBuffer::new(100);
        for i in 0..10 {
            buf.push(StyledLine::new(format!("line {}", i), LineStyle::Normal));
        }
        buf.scroll_up(3, 3);
        assert!(!buf.is_at_bottom());
        let vp = buf.viewport(3);
        assert_eq!(vp[0].content, "line 4");
        assert_eq!(vp[2].content, "line 6");
    }

    #[test]
    fn scroll_down_returns_to_bottom() {
        let mut buf = ScrollBuffer::new(100);
        for i in 0..10 {
            buf.push(StyledLine::new(format!("line {}", i), LineStyle::Normal));
        }
        buf.scroll_up(5, 3);
        buf.scroll_down(5);
        assert!(buf.is_at_bottom());
    }

    #[test]
    fn scroll_to_bottom() {
        let mut buf = ScrollBuffer::new(100);
        for i in 0..10 {
            buf.push(StyledLine::new(format!("line {}", i), LineStyle::Normal));
        }
        buf.scroll_up(100, 5);
        buf.scroll_to_bottom();
        assert!(buf.is_at_bottom());
    }

    #[test]
    fn push_auto_scrolls_to_bottom() {
        let mut buf = ScrollBuffer::new(100);
        for i in 0..10 {
            buf.push(StyledLine::new(format!("line {}", i), LineStyle::Normal));
        }
        buf.scroll_up(5, 3);
        assert!(!buf.is_at_bottom());
        buf.push(StyledLine::new("new line", LineStyle::Normal));
        assert!(buf.is_at_bottom());
    }

    #[test]
    fn viewport_empty_height() {
        let mut buf = ScrollBuffer::new(100);
        buf.push(StyledLine::new("hello", LineStyle::Normal));
        assert!(buf.viewport(0).is_empty());
    }

    #[test]
    fn scroll_up_clamps() {
        let mut buf = ScrollBuffer::new(100);
        buf.push(StyledLine::new("only one", LineStyle::Normal));
        buf.scroll_up(1000, 5);
        // Should not panic; offset clamped
        let vp = buf.viewport(5);
        assert_eq!(vp.len(), 1);
    }
}
