use streamdown_parser::Parser;
use streamdown_render::{Renderer, RenderFeatures, RenderStyle};

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
        let style = RenderStyle {
            h1: "0;255;128".to_string(),
            h2: "0;220;128".to_string(),
            h3: "0;200;128".to_string(),
            h4: "0;180;128".to_string(),
            h5: "0;160;128".to_string(),
            h6: "0;140;128".to_string(),
            code_bg: "20;20;60".to_string(),
            code_label: "0;255;255".to_string(),
            bullet: "255;255;0".to_string(),
            table_header_bg: "80;60;120".to_string(),
            table_border: "180;160;220".to_string(),
            blockquote_border: "0;255;255".to_string(),
            think_border: "128;128;128".to_string(),
            hr: "128;128;128".to_string(),
            link_url: "0;255;255".to_string(),
            image_marker: "255;255;0".to_string(),
            footnote: "180;160;220".to_string(),
            heading_centered: false,
        };
        let mut renderer = Renderer::with_style(buf, self.terminal_width as usize, style);
        renderer.set_features(RenderFeatures {
            pretty_pad: false,
            pretty_broken: false,
            clipboard: false,
            savebrace: false,
            margin: 0,
            fixed_width: Some(self.terminal_width as usize),
            ..Default::default()
        });
        renderer.set_theme("base16-eighties.dark");
        renderer
    }
}
