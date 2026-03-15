---
name: codiv-aisdk
description: >
  Guide for building with the aisdk library (razorback16/aisdk) in the Codiv agent system.
  Covers multi-provider streaming, the agent tool loop, provider configuration, tool registration,
  and aisdk library internals. Use this skill PROACTIVELY whenever:
  - Working on codivd daemon code (crates/codivd/) especially agent/, config, or streaming logic
  - Adding or modifying AI providers (Anthropic, OpenAI, Google, or OpenAI-compatible)
  - Implementing or debugging streaming text generation
  - Building or modifying the agent tool loop
  - Defining tools with the #[tool] macro or manual Tool construction
  - Working on aisdk library internals (providers/, core/, macros/)
  - Configuring model catalogs, provider settings, or API keys
  - Handling LanguageModelStreamChunkType variants or StreamChunk IPC mapping
  Even if the user doesn't say "aisdk" explicitly — if they mention providers, streaming, tool calls,
  agent loop, or LLM integration in the context of Codiv, use this skill.
---

# Codiv aisdk Integration Guide

This skill covers the **aisdk** library (a Rust Vercel AI SDK-inspired multi-provider LLM abstraction)
and how it integrates with the **codivd** daemon in the Codiv terminal agent project.

## Project Layout

```
codiv/                          # Main Codiv project
├── crates/
│   ├── codiv/                        # Rust TUI client (ratatui + crossterm)
│   ├── codivd/                       # Async daemon — where aisdk is consumed
│   │   └── src/
│   │       ├── main.rs               # Daemonization (fork before tokio)
│   │       ├── daemon.rs             # Event loop: accept, dispatch, cleanup
│   │       ├── session.rs            # ClientSession with lazy Agent ownership
│   │       ├── ipc/server.rs         # Unix socket + framed bincode messages
│   │       └── agent/
│   │           ├── agent.rs          # Agent struct, history, message building
│   │           ├── config.rs         # Provider dispatch + run_stream<M>()
│   │           ├── tools.rs          # Tool registration (bash/read/write/edit/glob/grep)
│   │           └── roles.rs          # System prompts per AgentRole
│   ├── codiv-common/                 # Shared types: ClientMessage, DaemonMessage, StreamChunk
│   └── codiv-tools/                  # Tool implementations
│       └── src/tools/                # bash.rs, read.rs, write.rs, edit.rs, glob.rs, grep.rs

codiv-skills/aisdk/             # The aisdk library itself
├── src/
│   ├── lib.rs                        # Re-exports: Error, Result, tool macro
│   ├── core/
│   │   ├── mod.rs                    # Core re-exports
│   │   ├── language_model/
│   │   │   ├── mod.rs                # LanguageModel trait, options, types
│   │   │   ├── generate_text.rs      # Synchronous agent loop
│   │   │   ├── stream_text.rs        # Streaming agent loop (channel-based)
│   │   │   └── request.rs            # Type-state builder pattern
│   │   ├── messages.rs               # Message, Messages, Role, builders
│   │   ├── tools.rs                  # Tool, ToolExecute, ToolList, ToolCallInfo
│   │   ├── capabilities.rs           # Marker traits + model_capabilities! macro
│   │   ├── client.rs                 # HTTP retry logic (429, 502-504)
│   │   └── embedding_model/          # EmbeddingModel trait + request
│   ├── providers/
│   │   ├── openai/                   # Native OpenAI provider
│   │   ├── anthropic/                # Native Anthropic provider
│   │   ├── google/                   # Native Google provider
│   │   └── openaichatcompletions/    # Base for 50+ OpenAI-compatible providers
│   ├── error.rs                      # Error enum (ApiError, ToolCallError, etc.)
│   └── extensions.rs                 # Type-erased metadata container
└── macros/                           # #[tool] proc-macro crate
```

## Core Patterns

### 1. Provider Construction

Providers follow a consistent pattern: generic over a model type `M: ModelName`.

**Compile-time model selection** (preferred when the model is known):
```rust
use aisdk::providers::anthropic::Anthropic;
use aisdk::providers::openai::OpenAI;
use aisdk::providers::google::Google;

let model = Anthropic::claude_sonnet_4_0();  // Provider<ClaudeSonnet40>
let model = OpenAI::gpt_4o();               // Provider<Gpt4O>
let model = Google::gemini_2_0_flash();      // Provider<Gemini20Flash>
```

**Runtime model selection** (when model name comes from config):
```rust
use aisdk::core::DynamicModel;

let model = Anthropic::model_name("claude-sonnet-4-0");  // Provider<DynamicModel>
let model = OpenAI::model_name("gpt-4o");
```

**Builder for custom config** (API key, base URL overrides):
```rust
let model = OpenAI::<DynamicModel>::builder()
    .model_name("gpt-4o")
    .api_key("sk-...")
    .base_url("https://custom-endpoint.com")
    .build()?;
```

**Default API keys** are read from environment variables:
- `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GOOGLE_GENERATIVE_AI_API_KEY`
- Compatible providers have their own env vars (e.g., `GROQ_API_KEY`)

### 2. Building Messages

Use the type-state `MessageBuilder`:

```rust
use aisdk::core::messages::Message;

let messages = Message::builder()
    .system("You are a coding assistant.")
    .user("Fix this bug")
    .build();
```

For multi-turn conversations (what codivd does):
```rust
let messages = Message::conversation_builder()
    .user("First question")
    .assistant("First answer")
    .user("Follow-up question")
    .build();
```

The system message is set separately via `.system()` on the request builder, not in the messages vector. In codivd's `Agent::build_messages()`, the system prompt is kept separate and passed via the request builder.

### 3. The Agent Tool Loop

Both `generate_text` and `stream_text` implement an automatic multi-step agent loop.

**generate_text** (blocking, simpler):
```rust
use aisdk::core::LanguageModelRequest;

let response = LanguageModelRequest::builder()
    .model(model)
    .system("System prompt")
    .messages(messages)
    .with_tool(my_tool)
    .build()
    .generate_text()
    .await?;

let text = response.text();       // Final text output
let usage = response.usage();     // Token counts
let steps = response.steps();     // All agent loop steps
```

**stream_text** (what codivd uses):
```rust
let response = LanguageModelRequest::builder()
    .model(model)
    .system("System prompt")
    .messages(messages)
    .with_tool(bash_tool)
    .with_tool(read_tool)
    .with_tool(write_tool)
    .build()
    .stream_text()
    .await?;

// Consume the stream
use futures::StreamExt;
while let Some(chunk) = response.stream.next().await {
    match chunk {
        LanguageModelStreamChunkType::Text(t) => { /* streaming text */ }
        LanguageModelStreamChunkType::Reasoning(t) => { /* thinking/reasoning */ }
        LanguageModelStreamChunkType::ToolCallDelta { tool_call_id, tool_name, delta } => {
            /* partial tool call arguments (JSON fragments) */
        }
        LanguageModelStreamChunkType::ToolCallStart(info) => {
            /* tool execution begins — info.tool.name, info.input */
        }
        LanguageModelStreamChunkType::ToolResult(info) => {
            /* tool finished — info.output is Result<serde_json::Value> */
        }
        LanguageModelStreamChunkType::Failed(err) => { /* stream error */ }
        LanguageModelStreamChunkType::Incomplete(_) => { /* partial/interrupted */ }
        _ => {}
    }
}

// After stream ends, get final metadata
let usage = response.usage().await;
let stop = response.stop_reason().await;
```

The agent loop automatically:
1. Sends messages to the model
2. If the model returns tool calls, executes them via `ToolList::execute()`
3. Appends tool results as `Message::Tool(ToolResultInfo)` to the conversation
4. Loops back to step 1 until the model returns text (no more tool calls)

**Default tool step limit:** `DEFAULT_TOOL_STEP_COUNT = 3`. Override with hooks.

### 4. Defining Tools

**Using the #[tool] proc-macro** (recommended for standalone tools):
```rust
use aisdk::tool;

/// Read the contents of a file at the given path.
#[tool]
fn read_file(path: String) -> String {
    std::fs::read_to_string(&path).unwrap_or_else(|e| format!("Error: {e}"))
}

// Use it: creates a Tool value
let tool = read_file();
```

The macro generates:
- A struct from the function args for JSON Schema derivation
- A factory function returning `Tool` with name from fn name, description from doc comment
- The function body becomes the `ToolExecute` closure

**Manual Tool construction** (what codivd does for tools needing captured state):
```rust
use aisdk::core::tools::{Tool, ToolExecute};
use schemars::schema_for;
use serde::Deserialize;

#[derive(Deserialize, schemars::JsonSchema)]
struct BashInput {
    /// The command to execute
    command: String,
    /// Optional timeout in milliseconds
    timeout_ms: Option<u64>,
}

let cwd = Arc::new(Mutex::new("/home/user".to_string()));
let env = Arc::new(env_vars.clone());

let bash_tool = Tool {
    name: "bash".into(),
    description: "Execute a bash command".into(),
    input_schema: schema_for!(BashInput),
    execute: ToolExecute::new(Box::new(move |input| {
        let args: BashInput = serde_json::from_value(input).map_err(|e| e.to_string())?;
        // execute command with captured cwd/env
        Ok(output)
    })),
};
```

### Adding a New Tool to Codivd (the full codiv-specific flow)

Codivd's tools span three layers. Follow all three when adding a new tool:

**Step 1: Create the tool module in `crates/codiv-tools/src/tools/`**

Each tool is a module exporting an input struct and an `execute` function:

```rust
// crates/codiv-tools/src/tools/search.rs
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SearchInput {
    /// The search query string
    pub query: String,
    /// Maximum number of results to return
    pub max_results: Option<usize>,
}

pub fn execute(value: Value) -> Result<String, String> {
    let input: SearchInput =
        serde_json::from_value(value).map_err(|e| format!("invalid input: {e}"))?;
    // ... search logic ...
    Ok(results_string)
}
```

Doc comments on struct fields become the parameter descriptions in the JSON Schema
that the LLM reads to understand what arguments to pass.

**Step 2: Register the module in `crates/codiv-tools/src/tools/mod.rs`**

```rust
pub mod search;  // add alongside bash, edit, glob, grep, read, write
```

**Step 3: Add to `build_tools()` in `crates/codivd/src/agent/tools.rs`**

Codivd uses a `make_tool` helper that combines the name, description, schemars-derived
schema, and execute function into an aisdk `Tool`:

```rust
use codiv_tools::tools::search;  // add to imports

// Inside build_tools() vec:
make_tool::<search::SearchInput>(
    "search",
    "Search for information using a query string.",
    search::execute,  // fn(Value) -> Result<String, String>
),
```

If the tool needs captured state (like `cwd` or `env_vars`), clone the `Arc` and
use a closure, same as `bash` and `glob` do:

```rust
{
    let cwd = Arc::clone(&cwd);
    make_tool::<search::SearchInput>(
        "search",
        "Search for information using a query string.",
        move |v| search::execute(v, &cwd),
    )
}
```

The `build_tools()` result flows through `Agent::run_streaming()` -> `config::stream_from_config()`
where each tool is added to the aisdk request via `.with_tool(tool)`.

**OpenAI schema sanitization:** When using OpenAI or OpenAI-compatible providers (Groq, etc.),
tool schemas need sanitization (remove `$schema`, `title`, `format`, `minimum`, `maximum`,
`default`; ensure all properties are in `required`). This happens automatically in
`stream_from_config()` via `sanitize_tool_schema_for_openai()` — you don't need to handle
it in the tool definition itself, but be aware it exists if you're debugging schema issues.

### 5. Hooks for Controlling the Agent Loop

```rust
use std::sync::Arc;

let request = LanguageModelRequest::builder()
    .model(model)
    .messages(messages)
    // Stop after a condition is met
    .stop_when(Arc::new(|opts| {
        opts.steps().len() >= 10  // max 10 tool-call rounds
    }))
    // Called before each step
    .on_step_start(Arc::new(|opts| {
        log::info!("Starting step {}", opts.current_step_id());
    }))
    // Called after each step
    .on_step_finish(Arc::new(|opts| {
        log::info!("Finished step, usage so far: {:?}", opts.usage());
    }))
    .build()
    .stream_text()
    .await?;
```

### 6. How codivd Maps Stream Chunks to IPC

In `crates/codivd/src/agent/config.rs`, the `run_stream<M>()` function maps aisdk chunks to
`StreamChunk` variants (defined in `crates/codiv-common/src/messages.rs`):

| aisdk `LanguageModelStreamChunkType` | codivd `StreamChunk` |
|--------------------------------------|----------------------|
| `Text(t)` | `StreamChunk::Text(t)` |
| `Reasoning(t)` | `StreamChunk::Reasoning(t)` |
| `ToolCallDelta { id, name, delta }` | `StreamChunk::ToolCallDelta { tool_call_id, tool_name, delta }` |
| `ToolCallStart(info)` | `StreamChunk::ToolCall { name, arguments }` |
| `ToolResult(info)` | `StreamChunk::ToolResult { name, result }` |
| `Failed(err)` | Error return (retry or propagate) |
| `Incomplete(_)` | Warning log, continue |

These are wrapped in `DaemonMessage::AgentStreamChunk { request_id, chunk }` and sent
over the Unix socket as framed bincode to the codiv client.

## aisdk Library Internals

### Adding a New Native Provider

Native providers (OpenAI, Anthropic, Google) implement the full `LanguageModel` trait directly
with custom HTTP/SSE logic. To add one:

1. Create `src/providers/myprovider/` with:
   - `mod.rs` — provider struct, builder, `model_capabilities!` invocation
   - `language_model.rs` — `impl LanguageModel for Provider<M>` (generate_text, stream_text)
   - `settings.rs` — API key env var, base URL, headers

2. The provider struct pattern:
```rust
pub struct Provider<M: ModelName> {
    settings: ProviderSettings,
    options: ProviderOptions,  // headers, etc.
    _phantom: PhantomData<M>,
}
```

3. Define models via `model_capabilities!`:
```rust
model_capabilities!(Provider, MyModel, "my-model-id", [
    TextInputSupport, TextOutputSupport, ToolCallSupport
]);
```

4. Implement `LanguageModel`:
   - `generate_text()` → HTTP POST, parse JSON response into `LanguageModelResponse`
   - `stream_text()` → SSE stream via `reqwest-eventsource`, parse into `ProviderStream`

5. Add a feature flag in `Cargo.toml` and conditional compilation in `src/providers/mod.rs`.

### Adding an OpenAI-Compatible Provider

Much simpler — most providers use the OpenAI Chat Completions format:

1. Create `src/providers/myprovider/mod.rs`:
```rust
use crate::providers::openaichatcompletions::OpenAIChatCompletions;

pub type Provider<M> = OpenAIChatCompletions<M>;

// Default settings
impl<M: ModelName> Default for ProviderSettings<M> {
    fn default() -> Self {
        Self {
            api_key: env::var("MYPROVIDER_API_KEY").unwrap_or_default(),
            base_url: "https://api.myprovider.com".into(),
            ..
        }
    }
}
```

2. Add feature flag: `myprovider = ["openaichatcompletions"]`

### Streaming Architecture (aisdk internals)

The streaming pipeline:

```
Provider::stream_text()
  → reqwest-eventsource (SSE)
  → parse_stream_sse() (provider-specific SSE event parsing)
  → ProviderStream (Pin<Box<dyn Stream<Item = Result<Vec<LanguageModelStreamChunk>>>>>)

stream_text.rs agent loop (tokio::spawn):
  → reads ProviderStream
  → for Delta chunks: forward to UnboundedSender<LanguageModelStreamChunkType>
  → for Done(ToolCall): execute tool, emit ToolCallStart + ToolResult, continue loop
  → when done: drop sender (closes stream)

Consumer (codivd's run_stream):
  → LanguageModelStream implements Stream<Item = LanguageModelStreamChunkType>
  → reads via StreamExt::next()
```

Key types in the chain:
- `ProviderStream` = `Pin<Box<dyn Stream<Item = Result<Vec<LanguageModelStreamChunk>>> + Send>>`
- `LanguageModelStreamChunk` = `Delta(ChunkType) | Done(AssistantMessage)`
- `LanguageModelStreamChunkType` = the individual deltas consumers see
- `LanguageModelStream` = wraps `UnboundedReceiver`, implements `futures::Stream`

### Modifying the Agent Loop

The agent loop lives in two places:
- `src/core/language_model/generate_text.rs` — synchronous loop
- `src/core/language_model/stream_text.rs` — async loop in spawned task

Both follow the same pattern:
1. Increment `current_step_id`
2. Call `on_step_start` hook
3. Call model's `generate_text()`/`stream_text()`
4. Process response contents (text, reasoning, tool calls)
5. For tool calls: execute via `ToolList::execute()`, push result as `Message::Tool`
6. Call `on_step_finish` hook
7. Check `stop_when` hook → break if true
8. If last content was not a tool call → break (model is done)
9. Otherwise loop (model wants more tool results)

### Error Handling and Retry

The HTTP client (`src/core/client.rs`) handles retries automatically:
- Retryable status codes: 429, 502, 503, 504
- Exponential backoff: `initial_wait * 2^retry`, max 30s, ±10% jitter
- Respects `Retry-After` header
- Default: 5 retries, 1s initial wait

Codivd adds its own retry layer in `config.rs` (3 retries with `classify_error()`).

Stream errors surface as `LanguageModelStreamChunkType::Failed(String)` — codivd's
`run_stream()` checks these and decides whether to retry or propagate.

### Message Merging

aisdk automatically merges consecutive assistant messages to prevent API 400 errors
(some providers reject conversations with back-to-back same-role messages). This happens
in the request builder before sending to the provider.

## Common Pitfalls

1. **System prompt location:** Pass via `.system()` on the request builder, NOT as a
   `Message::System` in the messages vector. The builder handles system message placement
   per-provider (Anthropic uses a separate `system` field, OpenAI uses a system role message).

2. **Temperature scaling:** aisdk uses integer 0-100 (not 0.0-1.0). It's scaled internally.

3. **Tool schema compatibility:** OpenAI's strict mode rejects schemas with extra fields.
   Use `sanitize_tool_schema_for_openai()` or be careful with `schemars` output.

4. **Agent ownership in codivd:** The `Agent` is moved out of `ClientSession` during
   streaming and returned via a oneshot channel. Don't try to access it concurrently.

5. **Fork before tokio:** codivd forks for daemonization BEFORE creating the tokio runtime.
   This avoids inheriting kqueue/epoll file descriptors across the fork.

6. **DynamicModel bypasses compile-time checks:** When using `Provider::model_name()`,
   all capability traits are implemented automatically. The compiler won't catch unsupported
   operations — you'll get runtime errors instead.

7. **Tool result truncation:** codivd truncates tool results to 256KB before sending back
   to the model to avoid context window overflow.
