use streamdown_parser::Parser;
use streamdown_render::{Renderer, RenderFeatures};

pub struct MarkdownStream {
    parser: Parser,
    line_buffer: String,
    terminal_width: u16,
}

impl MarkdownStream {
    pub fn new(terminal_width: u16) -> Self {
        Self {
            parser: Parser::new(),
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

        let mut buf = Vec::new();
        {
            let mut renderer = self.make_renderer(&mut buf);
            for line in complete.lines() {
                let events = self.parser.parse_line(line);
                renderer.render(&events).ok();
            }
        }

        if buf.is_empty() {
            None
        } else {
            let output = String::from_utf8_lossy(&buf).replace('\n', "\r\n");
            Some(output.into_bytes())
        }
    }

    /// Flush remaining line buffer and close open blocks.
    pub fn finish(&mut self) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut renderer = self.make_renderer(&mut buf);
            if !self.line_buffer.is_empty() {
                let line = std::mem::take(&mut self.line_buffer);
                let events = self.parser.parse_line(&line);
                renderer.render(&events).ok();
            }
            let final_events = self.parser.finalize();
            renderer.render(&final_events).ok();
        }

        let output = String::from_utf8_lossy(&buf).replace('\n', "\r\n");
        output.into_bytes()
    }

    /// Reset for a new streaming session.
    pub fn reset(&mut self) {
        self.parser.reset();
        self.line_buffer.clear();
    }

    pub fn set_width(&mut self, width: u16) {
        self.terminal_width = width;
    }

    fn make_renderer<'a>(&self, buf: &'a mut Vec<u8>) -> Renderer<&'a mut Vec<u8>> {
        let features = RenderFeatures {
            pretty_pad: false,
            pretty_broken: false,
            clipboard: false,
            savebrace: false,
            margin: 0,
            fixed_width: Some(self.terminal_width as usize),
            ..Default::default()
        };
        let mut renderer = Renderer::with_features(buf, self.terminal_width as usize, features);
        renderer.set_theme("base16-eighties.dark");
        renderer
    }
}
