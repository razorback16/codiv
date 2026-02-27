//! Test that reproduces the daemon's EXACT message building flow.
//!
//! Run with: cargo run -p slated --example tool_loop_test

use aisdk::core::language_model::LanguageModelStreamChunkType;
use aisdk::core::messages::Message;
use aisdk::core::tools::{Tool, ToolExecute};
use aisdk::core::LanguageModelRequest;
use aisdk::providers::Anthropic;
use futures::StreamExt;
use serde::Deserialize;

#[derive(Deserialize, schemars::JsonSchema)]
struct BashInput {
    /// The bash command to execute
    command: String,
}

fn make_bash_tool() -> Tool {
    Tool {
        name: "bash".to_string(),
        description: "Execute a bash command and return its output.".to_string(),
        input_schema: schemars::schema_for!(BashInput),
        execute: ToolExecute::new(Box::new(|v| {
            let input: BashInput =
                serde_json::from_value(v).map_err(|e| format!("bad input: {e}"))?;
            eprintln!("[TOOL] executing: {}", input.command);
            let output = std::process::Command::new("bash")
                .arg("-c")
                .arg(&input.command)
                .output()
                .map_err(|e| format!("spawn error: {e}"))?;
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            eprintln!("[TOOL] stdout: {}", stdout.trim());
            Ok(stdout)
        })),
    }
}

/// Replicate exactly what the daemon's Agent::build_messages() does.
fn build_messages_like_daemon(system: &str, history: &[(bool, String)]) -> aisdk::core::messages::Messages {
    let mut builder = Message::builder().system(system);

    for (is_user, text) in history {
        if *is_user {
            builder = builder.user(text.clone());
        } else {
            builder = builder.assistant(text.clone());
        }
    }

    builder.build()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    eprintln!("=== daemon message-building test ===\n");

    let system_prompt = "You are a helpful coding assistant embedded in a terminal. \
        You can see the user's recent terminal commands and their output in the conversation history. \
        Use this context to give relevant, concise answers. \
        When referencing files or directories, use paths relative to the user's current working directory when possible.";

    // Simulate the daemon's session timeline:
    // 1. User ran `ls` before asking (ShellCommand -> assistant + user pair)
    // 2. User asked `? read the cli-tool.md` (UserQuery)
    let history: Vec<(bool, String)> = vec![
        // ShellCommand: ls
        (false, "Running command: ls (in /Users/subhagato/Development/apps/slate-agent)".to_string()),
        (true, "[Terminal output for `ls`]:\nCargo.lock\nCargo.toml\nCLAUDE.md\ncrates\nREADME.md\nslated\ntarget".to_string()),
        // UserQuery
        (true, "read the `cli-tool.md`".to_string()),
    ];

    let messages = build_messages_like_daemon(system_prompt, &history);

    eprintln!("[MAIN] Built {} messages", messages.len());
    for (i, msg) in messages.iter().enumerate() {
        let preview = format!("{:?}", msg);
        eprintln!("  msg[{}]: {:.100}", i, preview);
    }

    let model = Anthropic::model_name("claude-sonnet-4-5");
    let tools = vec![make_bash_tool()];

    let mut builder = LanguageModelRequest::builder()
        .model(model)
        .messages(messages);

    for tool in tools {
        builder = builder.with_tool(tool);
    }

    let mut response = builder.build().stream_text().await?;
    let mut full_text = String::new();
    let mut chunk_count = 0u32;

    eprintln!("\n[MAIN] streaming response...\n");

    while let Some(chunk) = response.stream.next().await {
        chunk_count += 1;
        match &chunk {
            LanguageModelStreamChunkType::Text(text) => {
                eprint!("{}", text);
                full_text.push_str(text);
            }
            LanguageModelStreamChunkType::Start => {
                eprintln!("[START]");
            }
            LanguageModelStreamChunkType::Failed(e) => {
                eprintln!("[FAILED] {}", e);
            }
            LanguageModelStreamChunkType::Incomplete(reason) => {
                eprintln!("[INCOMPLETE] {}", reason);
            }
            _ => {}
        }
    }

    eprintln!("\n\n=== stream ended ===");
    eprintln!("chunks: {}, text_len: {}", chunk_count, full_text.len());
    eprintln!("full_text: {:?}", &full_text[..full_text.len().min(200)]);
    eprintln!("stop_reason: {:?}", response.stop_reason().await);

    if let Some(calls) = response.tool_calls().await {
        eprintln!("tool_calls: {}", calls.len());
        for tc in &calls {
            eprintln!("  - {}: {:?}", tc.tool.name, tc.input);
        }
    }

    if full_text.is_empty() {
        eprintln!("\n*** BUG REPRODUCED: full_text is EMPTY ***");
        std::process::exit(1);
    } else {
        eprintln!("\n=== TEST PASSED ===");
    }

    Ok(())
}
