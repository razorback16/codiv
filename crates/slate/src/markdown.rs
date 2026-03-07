use std::io::Write;
use std::rc::Rc;
use std::cell::RefCell;

use streamdown_parser::Parser;
use streamdown_render::{Renderer, RenderFeatures, RenderStyle};

/// A shared byte buffer that implements `Write` so we can hand it to
/// `Renderer` while still being able to drain the contents between pushes.
#[derive(Clone)]
struct SharedBuf(Rc<RefCell<Vec<u8>>>);

impl SharedBuf {
    fn new() -> Self {
        Self(Rc::new(RefCell::new(Vec::new())))
    }

    fn drain(&self) -> Vec<u8> {
        std::mem::take(&mut *self.0.borrow_mut())
    }

    fn is_empty(&self) -> bool {
        self.0.borrow().is_empty()
    }
}

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub struct MarkdownStream {
    parser: Parser,
    renderer: Renderer<SharedBuf>,
    buf: SharedBuf,
    line_buffer: String,
    terminal_width: u16,
}

impl MarkdownStream {
    pub fn new(terminal_width: u16) -> Self {
        let buf = SharedBuf::new();
        let renderer = Self::build_renderer(buf.clone(), terminal_width);
        Self {
            parser: Parser::new(),
            renderer,
            buf,
            line_buffer: String::new(),
            terminal_width,
        }
    }

    /// Push a text token. Returns ANSI bytes to feed to vt100 if any
    /// complete lines were rendered.
    pub fn push(&mut self, token: &str) -> Option<Vec<u8>> {
        self.line_buffer.push_str(token);

        if !self.line_buffer.contains('\n') {
            return None;
        }

        let last_newline = self.line_buffer.rfind('\n').unwrap();
        let complete = self.line_buffer[..=last_newline].to_string();
        let remainder = self.line_buffer[last_newline + 1..].to_string();
        self.line_buffer = remainder;

        for line in complete.lines() {
            let events = self.parser.parse_line(line);
            self.renderer.render(&events).ok();
        }

        if self.buf.is_empty() {
            None
        } else {
            let raw = self.buf.drain();
            let output = String::from_utf8_lossy(&raw).replace('\n', "\r\n");
            Some(output.into_bytes())
        }
    }

    /// Flush remaining line buffer and close open blocks.
    pub fn finish(&mut self) -> Vec<u8> {
        if !self.line_buffer.is_empty() {
            let line = std::mem::take(&mut self.line_buffer);
            let events = self.parser.parse_line(&line);
            self.renderer.render(&events).ok();
        }
        let final_events = self.parser.finalize();
        self.renderer.render(&final_events).ok();

        let raw = self.buf.drain();
        if raw.is_empty() {
            return Vec::new();
        }
        let output = String::from_utf8_lossy(&raw).replace('\n', "\r\n");
        output.into_bytes()
    }

    /// Reset for a new streaming session.
    pub fn reset(&mut self) {
        self.parser.reset();
        self.line_buffer.clear();
        self.buf.drain();
        self.renderer = Self::build_renderer(self.buf.clone(), self.terminal_width);
    }

    pub fn set_width(&mut self, width: u16) {
        self.terminal_width = width;
    }

    /// Detect whether the terminal has a light background.
    /// Checks COLORFGBG env var (set by xterm, iTerm2, etc.).
    /// Format: "foreground;background" — higher background values mean lighter.
    fn detect_terminal_colorscheme() -> bool {
        if let Ok(val) = std::env::var("COLORFGBG") {
            if let Some(bg) = val.rsplit(';').next() {
                if let Ok(n) = bg.parse::<u32>() {
                    return n >= 8;
                }
            }
        }
        false
    }

    fn build_renderer(buf: SharedBuf, width: u16) -> Renderer<SharedBuf> {
        let light = Self::detect_terminal_colorscheme();

        let style = if light {
            RenderStyle {
                h1: "#1a1a1a".to_string(),
                h2: "#8b6914".to_string(),
                h3: "#2e7d32".to_string(),
                h4: "#00695c".to_string(),
                h5: "#555555".to_string(),
                h6: "#777777".to_string(),
                code_bg: String::new(),
                code_fg: "#b31d28".to_string(),
                code_dim: false,
                code_pad: false,
                code_label: "#00695c".to_string(),
                bullet: "#00695c".to_string(),
                table_header_bg: String::new(),
                table_body_bg: String::new(),
                table_border: "#888888".to_string(),
                blockquote_border: "#888888".to_string(),
                think_border: "#888888".to_string(),
                hr: "#aaaaaa".to_string(),
                link_url: "#888888".to_string(),
                image_marker: "#00695c".to_string(),
                footnote: "#00695c".to_string(),
                heading_centered: false,
            }
        } else {
            RenderStyle {
                heading_centered: false,
                code_bg: String::new(),
                code_fg: "#f47067".to_string(),
                table_header_bg: String::new(),
                table_body_bg: String::new(),
                ..Default::default()
            }
        };

        let theme = if light { "InspiredGitHub" } else { "base16-eighties.dark" };

        let mut renderer = Renderer::with_style(buf, width as usize, style);
        renderer.set_features(RenderFeatures {
            pretty_pad: false,
            pretty_broken: false,
            clipboard: false,
            savebrace: false,
            margin: 2,
            fixed_width: Some((width as usize).saturating_sub(2)),
            ..Default::default()
        });
        renderer.set_theme(theme);
        renderer
    }
}
