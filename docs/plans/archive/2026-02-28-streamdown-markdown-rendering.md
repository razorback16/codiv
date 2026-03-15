# Streamdown Markdown Rendering Integration

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Render AI response text as formatted markdown (headings, code blocks with syntax highlighting, lists, bold/italic, etc.) using the `streamdown` crate ecosystem instead of plain text.

**Architecture:** A `MarkdownStream` wrapper accumulates streaming tokens in a line buffer, parses complete lines via `streamdown_parser::Parser`, and renders them to ANSI via `streamdown_render::Renderer`. The ANSI output is fed into the existing vt100 parser for display. Reasoning tokens remain plain dim text.

**Tech Stack:** `streamdown-parser = "0.1"`, `streamdown-render = "0.1"` (Rust crates from the streamdown ecosystem)

---

### Task 1: Add streamdown dependencies to Cargo.toml

**Files:**
- Modify: `crates/codiv/Cargo.toml`

**Step 1: Add dependencies**

Add two lines to the end of `[dependencies]` in `crates/codiv/Cargo.toml`:

```toml
streamdown-parser = "0.1"
streamdown-render = "0.1"
```

**Step 2: Verify it compiles**

Run: `cargo check -p codiv`
Expected: compiles with no errors (new deps downloaded)

**Step 3: Commit**

```bash
git add crates/codiv/Cargo.toml Cargo.lock
git commit -m "feat: add streamdown-parser and streamdown-render dependencies"
```

---

### Task 2: Create the MarkdownStream module

**Files:**
- Create: `crates/codiv/src/markdown.rs`
- Modify: `crates/codiv/src/main.rs` (add `mod markdown;`)

**Step 1: Create `crates/codiv/src/markdown.rs`**

```rust
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
```

**Step 2: Register the module in `crates/codiv/src/main.rs`**

Add `mod markdown;` after the existing module declarations (line 4). The file starts with:

```rust
mod app;
mod shell;
mod ui;
mod ipc;
mod markdown;
```

**Step 3: Verify it compiles**

Run: `cargo check -p codiv`
Expected: compiles (module registered, no users yet, may warn about dead code — that's fine)

**Step 4: Commit**

```bash
git add crates/codiv/src/markdown.rs crates/codiv/src/main.rs
git commit -m "feat: add MarkdownStream module for streaming markdown rendering"
```

---

### Task 3: Wire MarkdownStream into the terminal event loop

**Files:**
- Modify: `crates/codiv/src/ui/terminal.rs`

This task has 4 changes to this file:

**Step 1: Add the import**

At the top of `terminal.rs`, after the existing `use crate::` imports (around line 24-30), add:

```rust
use crate::markdown::MarkdownStream;
```

**Step 2: Initialize MarkdownStream in the event loop**

In the `run_event_loop` function, after `let mut agent_streaming = false;` (line 204), add:

```rust
    let mut md_stream = MarkdownStream::new(term_size.width);
```

`term_size` is already computed on line 144 as `let term_size = term.size()?;`.

**Step 3: Update StreamChunk::Text handling**

Replace the current `Text` handler at lines 853-856:

```rust
                            ipc_messages::StreamChunk::Text(t) => {
                                let t = t.replace('\n', "\r\n");
                                parser.process(t.as_bytes());
                            }
```

With:

```rust
                            ipc_messages::StreamChunk::Text(t) => {
                                if let Some(ansi) = md_stream.push(&t) {
                                    parser.process(&ansi);
                                }
                            }
```

**Step 4: Update AgentComplete handling**

Replace the current handler at lines 893-901:

```rust
                    ipc_messages::DaemonMessage::AgentComplete {
                        request_id: _,
                        summary: _,
                    } => {
                        if agent_streaming {
                            parser.process(b"\r\n");
                            agent_streaming = false;
                        }
                    }
```

With:

```rust
                    ipc_messages::DaemonMessage::AgentComplete {
                        request_id: _,
                        summary: _,
                    } => {
                        if agent_streaming {
                            let final_bytes = md_stream.finish();
                            if !final_bytes.is_empty() {
                                parser.process(&final_bytes);
                            }
                            parser.process(b"\r\n");
                            md_stream.reset();
                            agent_streaming = false;
                        }
                    }
```

**Step 5: Update md_stream width on terminal resize**

In the `Event::Resize` handler (lines 456-461), add `md_stream.set_width(*cols);` after the existing resize calls:

```rust
                Event::Resize(cols, rows) => {
                    let parser_rows = rows.saturating_sub(1).max(1);
                    let parser_cols = (*cols).max(1);
                    parser.screen_mut().set_size(parser_rows, parser_cols);
                    bash.resize(parser_rows);
                    md_stream.set_width(*cols);
                    continue;
                }
```

**Step 6: Also reset md_stream on Error (agent_streaming = false)**

In the Error handler (around line 917 where `agent_streaming = false`), add a reset:

```rust
                    ipc_messages::DaemonMessage::Error {
                        request_id,
                        message,
                    } => {
                        agent_streaming = false;
                        md_stream.reset();
                        parser_push_styled(
                            parser,
                            &format!("[daemon] error ({}): {}", request_id, message),
                            "\x1b[31m",
                        );
                    }
```

**Step 7: Build and verify**

Run: `cargo build -p codiv`
Expected: compiles with no errors

**Step 8: Commit**

```bash
git add crates/codiv/src/ui/terminal.rs
git commit -m "feat: integrate streamdown for streaming markdown rendering of AI responses"
```
