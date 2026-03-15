---
name: streamdown-rs
description: >
  Guide for building with the streamdown-rs library (razorback16/streamdown-rs) and integrating
  it into the codiv TUI client. Covers the streaming markdown parser, ANSI renderer, syntax
  highlighting, RenderStyle/RenderFeatures configuration, and wiring ANSI output into ratatui's
  VT100 canvas. Use this skill PROACTIVELY whenever:
  - Implementing or fixing the markdown output rendering area in codiv
  - Handling IpcMessage::StreamChunk or StreamChunk::Text/Reasoning in the codiv TUI
  - Debugging incomplete code blocks, broken formatting, or mid-stream display glitches
  - Wiring streamdown-rs ANSI output through tui-term/vt100 into ratatui Lines/Spans
  - Implementing resize/reflow logic for the scrollable output zone (Event::Resize)
  - Developing the streamdown-rs library itself: new block types, syntax themes, streaming internals
  - Working on streamdown-parser, streamdown-render, streamdown-syntax, or streamdown-config crates
  Even if the user doesn't say "streamdown-rs" explicitly — if they mention streaming markdown,
  code fence rendering, the output canvas, VT100 reflow, syntax highlighting in codiv, or
  incomplete block handling mid-stream, use this skill.
---

# streamdown-rs Integration Guide

This skill covers the **streamdown-rs** library (a multi-crate streaming markdown renderer) and
how it integrates with the **codiv** TUI client for real-time AI response rendering.

## Crate Structure

```
streamdown-rs/                         # Library root (workspace)
├── crates/
│   ├── streamdown-core/               # ParseState enum, core types
│   ├── streamdown-parser/             # Parser, ParseEvent, inline tokenizer
│   ├── streamdown-render/             # Renderer<W>, RenderStyle, ANSI output
│   ├── streamdown-syntax/             # Highlighter (syntect), HighlightState
│   ├── streamdown-config/             # Config/StyleConfig/ComputedStyle TOML loading
│   └── streamdown-plugin/             # LaTeX → Unicode plugin
└── src/                               # Top-level re-export crate (streamdown)
```

**Important:** streamdown-rs outputs raw **ANSI escape strings** to any `impl Write` — it does
NOT produce `ratatui::Line`/`Span` types directly. Integration with ratatui goes through the
VT100 canvas pattern (see Integration section below).

---

## Core Streaming Pattern

The fundamental loop — use this whenever consuming streamdown-rs:

```rust
use streamdown_parser::Parser;
use streamdown_render::{Renderer, RenderStyle};

let mut parser = Parser::new();
let mut buf = Vec::<u8>::new();
let mut renderer = Renderer::with_style(&mut buf, terminal_width, RenderStyle::default());

// For each incoming token/line from the AI stream:
for line in token_lines {
    let events = parser.parse_line(&line);   // returns Vec<ParseEvent>
    renderer.render(&events).unwrap();       // writes ANSI bytes to buf
}

// At end of stream — flush any incomplete block state:
let final_events = parser.finalize();
renderer.render(&final_events).unwrap();
```

For event-by-event rendering (lower latency per token):
```rust
for event in parser.parse_line(&line) {
    renderer.render_event(&event).unwrap();
}
```

---

## ParseEvent — What the Parser Emits

```rust
pub enum ParseEvent {
    // Inline content
    Text(String),
    InlineCode(String),
    Bold(String), Italic(String), BoldItalic(String),
    Underline(String), Strikeout(String),
    Link { text: String, url: String },
    Image { alt: String, url: String },
    Footnote(String),
    InlineElements(Vec<InlineElement>),

    // Block boundaries
    CodeBlockStart { language: Option<String>, indent: usize },
    CodeBlockLine(String),
    CodeBlockEnd,

    Heading { level: u8, content: String },

    ListItem { indent: usize, bullet: ListBullet, content: String },
    ListEnd,

    TableHeader(Vec<String>),
    TableRow(Vec<String>),
    TableSeparator,
    TableEnd,

    BlockquoteStart { depth: usize },
    BlockquoteLine(String),
    BlockquoteEnd,

    ThinkBlockStart,       // <think> blocks (AI reasoning)
    ThinkBlockLine(String),
    ThinkBlockEnd,

    HorizontalRule,
    EmptyLine,
    Newline,
    Prompt(String),
}
```

`ListBullet` variants: `Dash`, `Asterisk`, `Plus`, `PlusExpand`, `Ordered(usize)`.

---

## How Incomplete Blocks Are Handled

This is the core value of streamdown-rs for streaming AI output:

- **`parse_line()`** maintains `ParseState` across calls. Lines inside a code block, table,
  blockquote, or think block are buffered — they emit nothing (empty `Vec`) until their
  block boundary is properly detected.
- **`finalize()`** flushes any dangling open block state (e.g., an unclosed ``` at EOF or
  mid-stream interruption). Always call this when the AI stream ends.
- **Code fences** are detected by regex `^\s*(```+|~~~+|<pre>)\s*([^\s]*)\s*$` — supports
  both backtick and tilde fences, and `<pre>` tags.
- **Loose lists** (list items separated by blank lines) preserve ordered list numbering via
  a counter stack `ordered_list_numbers`.
- **Think blocks** (`<think>...</think>`) have their own state machine separate from code blocks.

When streaming AI tokens arrive mid-fence, the UI should **not** call `finalize()` — only call
it when the full response is done or on connection drop.

---

## Parser API

```rust
pub struct Parser { /* streaming state */ }

impl Parser {
    pub fn new() -> Self
    pub fn with_state(state: ParseState) -> Self

    // Core method — call once per incoming line
    pub fn parse_line(&mut self, line: &str) -> Vec<ParseEvent>

    // Parse a complete document at once
    pub fn parse_document(&mut self, content: &str) -> Vec<ParseEvent>

    // Flush any open block state at stream end
    pub fn finalize(&mut self) -> Vec<ParseEvent>

    // Configuration
    pub fn set_process_links(&mut self, enabled: bool)
    pub fn set_process_images(&mut self, enabled: bool)
    pub fn set_code_spaces(&mut self, enabled: bool)  // 4-space indent as code block

    // State inspection
    pub fn state(&self) -> &ParseState
    pub fn state_mut(&mut self) -> &mut ParseState

    pub fn reset(&mut self)
}
```

---

## Renderer API

```rust
pub struct Renderer<W: Write> { /* writer, width, highlighter, style, features */ }

impl<W: Write> Renderer<W> {
    pub fn new(writer: W, width: usize) -> Self
    pub fn with_style(writer: W, width: usize, style: RenderStyle) -> Self
    pub fn with_features(writer: W, width: usize, features: RenderFeatures) -> Self

    // Core rendering
    pub fn render_event(&mut self, event: &ParseEvent) -> std::io::Result<()>
    pub fn render(&mut self, events: &[ParseEvent]) -> std::io::Result<()>

    // Runtime reconfiguration (use for resize)
    pub fn set_style(&mut self, style: RenderStyle)
    pub fn set_features(&mut self, features: RenderFeatures)
    pub fn set_theme(&mut self, theme: &str)      // syntax highlight theme
    pub fn set_pretty_pad(&mut self, enabled: bool)
    pub fn set_clipboard(&mut self, enabled: bool)
    pub fn set_savebrace(&mut self, enabled: bool)

    pub fn width(&self) -> usize
    pub fn style(&self) -> &RenderStyle
    pub fn features(&self) -> &RenderFeatures
}
```

---

## RenderStyle — Color Configuration

All colors are strings in one of: `"#rrggbb"`, `"r;g;b"` (RGB components), or a Colodore
palette name. Use `RenderStyle::default()` for the built-in Colodore color scheme.

```rust
pub struct RenderStyle {
    // Headings
    pub h1: String, pub h2: String, pub h3: String,
    pub h4: String, pub h5: String, pub h6: String,

    // Code blocks
    pub code_bg: String,      // code block background color
    pub code_fg: String,      // code block foreground (empty = use syntax highlight colors)
    pub code_dim: bool,       // dim inline code
    pub code_pad: bool,       // add padding spaces around inline code text
    pub code_label: String,   // language label color

    // Lists
    pub bullet: String,

    // Tables
    pub table_header_bg: String,
    pub table_body_bg: String,
    pub table_border: String,

    // Borders
    pub blockquote_border: String,
    pub think_border: String,
    pub hr: String,

    // Links and special
    pub link_url: String,
    pub image_marker: String,
    pub footnote: String,

    // Layout
    pub heading_centered: bool,
}

impl RenderStyle {
    pub fn default() -> Self                           // Colodore palette defaults
    pub fn from_computed(computed: &ComputedStyle) -> Self  // from HSV config
}
```

To derive a `RenderStyle` from the TOML config system:
```rust
use streamdown_config::Config;
let config = Config::load()?;
let style = RenderStyle::from_computed(&config.computed_style());
```

---

## RenderFeatures — Layout and Behavior Flags

```rust
pub struct RenderFeatures {
    pub pretty_pad: bool,           // ▄▄▄/▀▀▀ unicode borders for code blocks (default: true)
    pub pretty_broken: bool,        // wrap long code lines visually (default: false)
    pub clipboard: bool,            // OSC 52 clipboard copy support (default: false)
    pub savebrace: bool,            // save code blocks to temp files (default: false)
    pub width_wrap: bool,           // allow terminal to wrap long lines (default: true)
    pub fixed_width: Option<usize>, // override terminal width detection
    pub margin: usize,              // left margin padding (default: 1)
}

impl RenderFeatures {
    pub fn pretty() -> Self     // pretty_pad + pretty_broken enabled
    pub fn copyable() -> Self   // both disabled (for piped/non-interactive use)
    pub fn effective_width(&self) -> usize
    pub fn full_width(&self) -> usize
}
```

Code block borders use Unicode block chars: `CODEPAD_TOP = '▄'`, `CODEPAD_BOTTOM = '▀'`.
Bullet symbols: `BULLETS = ["•", "◦", "▪", "‣"]` (nested list levels).

---

## Syntax Highlighting

Backed by **syntect**. The `Highlighter` maintains per-code-block streaming state:

```rust
use streamdown_syntax::Highlighter;

let mut hl = Highlighter::new();                      // default: "base16-ocean.dark"
let mut hl = Highlighter::with_theme("Solarized (dark)");
hl.set_theme("base16-eighties.dark");

// For streaming line-by-line (what Renderer uses internally):
let mut state = hl.new_highlight_state("rust");
let ansi_line = hl.highlight_line_with_state(&code_line, &mut state);

// For complete blocks:
let ansi_block = hl.highlight_block(&code, "python");

// Available themes:
hl.themes()  // → ["base16-ocean.dark", "base16-ocean.light", "base16-eighties.dark",
             //     "base16-mocha.dark", "InspiredGitHub", "Solarized (dark)", "Solarized (light)"]

// Language aliases (set in Renderer via set_theme):
// "py" → Python, "js" → JavaScript, "ts" → TypeScript, "rs" → Rust,
// "sh"/"bash" → Bash, "cpp", "go", "rb", "java", "cs", "toml", "yaml", "json", etc.
```

When `code_fg` in `RenderStyle` is empty, the renderer uses full syntect colors (fg + bg per
token). When `code_bg` is set and `code_fg` is empty, the renderer strips syntect background
colors so only foreground token colors show over your custom background.

---

## Integration with codiv TUI (VT100 Canvas Pattern)

streamdown-rs outputs ANSI strings. The codiv TUI uses `vt100` + `tui-term` to paint these
into a ratatui widget. The pattern:

```rust
use vt100::Parser as VtParser;

// State held in the output area widget:
struct OutputArea {
    vt: VtParser,
    scroll_offset: u16,
    width: u16,
    height: u16,
}

// On each StreamChunk::Text arriving from codivd over IPC:
fn handle_stream_chunk(&mut self, text: &str) {
    // 1. Feed the text through streamdown to get ANSI bytes
    let mut buf = Vec::<u8>::new();
    let events = self.sd_parser.parse_line(text);
    self.sd_renderer.render(&events).unwrap();
    // buf now contains ANSI escape sequences

    // 2. Feed ANSI bytes into the VT100 parser
    self.output_area.vt.process(&buf);
}

// On stream end:
fn handle_stream_end(&mut self) {
    let mut buf = Vec::<u8>::new();
    let events = self.sd_parser.finalize();
    self.sd_renderer.render(&events).unwrap();
    self.output_area.vt.process(&buf);
}

// In ratatui render():
fn render_output(&self, frame: &mut Frame, area: Rect) {
    use tui_term::widget::PseudoTerminal;
    let screen = self.output_area.vt.screen();
    let widget = PseudoTerminal::new(screen).scroll(self.output_area.scroll_offset);
    frame.render_widget(widget, area);
}
```

**Renderer writer lifetime:** The `Renderer` holds a mutable reference to its writer. For the
pattern above, create the `buf`, pass `&mut buf` to `Renderer::new()`, use the renderer, then
let the renderer drop before reading `buf`. Or use a shared buffer with `Rc<RefCell<Vec<u8>>>`.

---

## Handling Terminal Resize / Reflow

When `Event::Resize(new_cols, new_rows)` fires in the codiv event loop:

```rust
fn handle_resize(&mut self, new_cols: u16, new_rows: u16) {
    // 1. Resize the VT100 parser (this handles reflow internally for tui-term)
    self.output_area.vt = vt100::Parser::new(new_rows, new_cols, 10_000);

    // 2. Update renderer width so future lines wrap correctly
    // Renderer doesn't expose set_width directly — recreate or re-render
    // Option A: Store all raw events and re-render from scratch (correct but expensive)
    // Option B: Accept that already-rendered content won't reflow (simpler)

    // 3. Resize the ratatui backend
    // crossterm handles this automatically via Event::Resize
}
```

**Reflow trade-off:** The VT100 screen state already rendered won't automatically reflow when
the terminal width changes — ANSI escape sequences are positional. For full reflow you need to
store the original `ParseEvent` stream and re-render it with the new width. For a simpler
approach, clear the VT screen and re-render from stored raw text on resize.

For new content after resize: update the `Renderer` width by recreating it:
```rust
// Keep track of width; recreate renderer when it changes
self.sd_renderer = Renderer::with_style(&mut self.render_buf, new_cols as usize, style);
```

---

## Config System

```rust
use streamdown_config::{Config, StyleConfig, ComputedStyle, FeaturesConfig};

// Load from default path (~/.config/streamdown/config.toml or XDG)
let config = Config::load()?;

// Load from explicit path
let config = Config::load_from(Path::new("/path/to/config.toml"))?;

// Access computed (ANSI-ready) style values
let computed: ComputedStyle = config.computed_style();
// computed.dark_fg, computed.mid_fg, computed.codebg, computed.codepad, etc.

// Build a RenderStyle from computed values
let style = RenderStyle::from_computed(&computed);
```

`StyleConfig` uses **HSV-based color generation**: a base `hsv: [f64; 3]` value plus
`HsvMultiplier { h, s, v }` for each role (`dark`, `mid`, `symbol`, `head`, `grey`, `bright`).
This generates a cohesive palette from a single hue/saturation/value base.

`FeaturesConfig` fields (all `bool`): `code_spaces`, `clipboard`, `logging`, `savebrace`,
`images`, `links`. Plus `timeout: f64`.

---

## Adding a New Block Type (streamdown-rs library dev)

To add a new block type (e.g., `AdmonitionBlock`), touch these files in order:

**Step 1 — Add enum variants** in `streamdown-core/src/enums.rs`.
This is where block-type enums live alongside `BlockType`, `ListType`, `TableState`, etc.
If your block has subtypes (e.g., NOTE/WARNING/TIP), add a new enum here:
```rust
#[derive(Debug, Clone, PartialEq)]
pub enum AdmonitionKind {
    Note, Warning, Tip, Important, Caution,
}

impl AdmonitionKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "NOTE" => Some(Self::Note),
            "WARNING" => Some(Self::Warning),
            "TIP" => Some(Self::Tip),
            "IMPORTANT" => Some(Self::Important),
            "CAUTION" => Some(Self::Caution),
            _ => None,
        }
    }
}
```
Re-export it from `streamdown-core/src/lib.rs` alongside the other enums.

**Step 2 — Add state** in `streamdown-core/src/state.rs`:
```rust
pub struct ParseState {
    // ... existing fields ...
    pub admonition_kind: Option<AdmonitionKind>,
}
```
Also update `reset()` and `Default` to clear the new field.

**Step 3 — Add `ParseEvent` variants** in `streamdown-parser/src/lib.rs`:
```rust
pub enum ParseEvent {
    // ... existing ...
    AdmonitionStart { kind: AdmonitionKind },
    AdmonitionLine(String),
    AdmonitionEnd,
}
```

**Step 4 — Add detection and parsing logic** in `Parser::parse_line()` in `streamdown-parser/src/lib.rs`:
- Add a `static ADMONITION_RE` regex (e.g., `(?i)^\[!(NOTE|WARNING|TIP|IMPORTANT|CAUTION)\]`)
- In `try_parse_block()`, check for the pattern **before** the existing blockquote regex so `> [!NOTE]` is intercepted first
- Add a fast-path branch in `parse_line()` when `state.admonition_kind.is_some()` to emit `AdmonitionLine` for continuation lines
- Update `handle_empty_line()` to emit `AdmonitionEnd` and clear `admonition_kind`

**Step 5 — Add render handler** in `streamdown-render/src/lib.rs` inside `render_event()`:
```rust
ParseEvent::AdmonitionStart { kind } => {
    // render a ┌─ NOTE ─ header line in kind-appropriate color
}
ParseEvent::AdmonitionLine(line) => {
    // render │-prefixed content lines
}
ParseEvent::AdmonitionEnd => {
    // render └ footer line
}
```
Add any new color fields to `RenderStyle` for the admonition kinds.

**Step 6 — Update `finalize()`** in `Parser` to emit `AdmonitionEnd` if the stream ends
mid-block (same pattern as the existing `CodeBlockEnd` flush in finalize).

---

## Adding a New Syntax Theme (streamdown-rs library dev)

syntect loads themes from `.tmTheme` XML files. To bundle a new theme:

1. Place the `.tmTheme` file in `streamdown-syntax/themes/` (or wherever your `SyntaxSet`
   is loaded from).
2. In `streamdown-syntax/src/lib.rs`, the `Highlighter::new()` loads themes from the embedded
   `ThemeSet`. Add your theme file to the embedded assets if using `SyntaxSet::load_defaults()`,
   or use `ThemeSet::load_from_folder()` for external themes.
3. Expose via `hl.themes()` list and `hl.has_theme(name)`.
4. Users set it via `Renderer::set_theme("My Theme Name")` or `RenderStyle`-level config.

---

## Wiring StreamChunk in codiv TUI

In the codiv client, AI output arrives as `DaemonMessage::AgentStreamChunk { request_id, chunk }`
over the Unix socket. The `StreamChunk` variants that feed streamdown-rs:

| `StreamChunk` variant | Action |
|---|---|
| `StreamChunk::Text(t)` | `parser.parse_line(&t)` → `renderer.render(&events)` → VT feed |
| `StreamChunk::Reasoning(t)` | Same as Text, or render differently (e.g., dimmed style) |
| `StreamChunk::ToolCall { name, arguments }` | Display as formatted tool call (not markdown) |
| `StreamChunk::ToolResult { name, result }` | Display as formatted result block |

StreamChunk::Text is the primary feed for streamdown-rs. Reasoning and tool output are typically
rendered with separate formatting, not through the markdown parser.

**Key point:** codiv is the **only consumer** of streamdown-rs in this project. codivd never
touches streamdown-rs — it handles raw text and sends it over the socket. All markdown rendering
happens client-side in codiv's output area widget.

---

## Common Pitfalls

1. **Forgetting `finalize()`:** If you don't call `parser.finalize()` after the stream ends,
   unclosed code blocks will never emit `CodeBlockEnd`, leaving the renderer in an inconsistent
   state for the next response.

2. **Parser state across responses:** Create a **new `Parser`** for each AI response (or call
   `parser.reset()`). ParseState carries over between calls — leftover state from a previous
   response will corrupt the next one.

3. **Renderer width mismatch:** The renderer wraps text at the width set at construction. If
   the terminal resizes, text wrapped for the old width looks odd. Update width on resize.

4. **ANSI vs ratatui Spans:** streamdown-rs writes raw ANSI escape bytes. Don't try to parse
   these into `ratatui::Span` — use the VT100 canvas (`vt100` crate + `tui-term`) to paint
   them into ratatui's rendering pipeline.

5. **`code_fg` vs syntax highlighting:** If `code_fg` is non-empty in `RenderStyle`, it
   overrides syntect foreground colors for all tokens. Leave it empty to use full syntax
   highlighting, or set it if you want uniform monochrome code blocks.

6. **Think blocks vs code blocks:** `<think>...</think>` is parsed as `ThinkBlock*` events,
   NOT as a code block. If you see raw `<think>` text in the output, the parser needs the
   ThinkBlock state machine active (it is on by default in `Parser::new()`).

7. **Renderer holds writer mutably:** The `Renderer<W>` owns (or mutably borrows) the writer.
   To get the output bytes after rendering, use `Vec<u8>` as writer and access it after the
   renderer is dropped (or via interior mutability).
