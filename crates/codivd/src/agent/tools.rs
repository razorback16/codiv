use std::sync::{Arc, RwLock};

use aisdk::core::tools::{Tool, ToolExecute};
use serde_json::Value;

use codiv_tools::tools::bash::BashInput;
use codiv_tools::tools::{edit, glob, grep, read, write};

use super::permissions::PermissionContext;
use super::shell_backend::ShellBackend;

fn make_tool_with_permissions<T: schemars::JsonSchema>(
    name: &str,
    description: &str,
    execute: impl Fn(Value) -> Result<String, String> + Send + Sync + 'static,
    permission_ctx: Option<&Arc<PermissionContext>>,
) -> Tool {
    let boxed: Box<dyn Fn(Value) -> Result<String, String> + Send + Sync> = Box::new(execute);
    let final_execute = match permission_ctx {
        Some(ctx) => super::permissions::wrap_with_permissions(
            name.to_string(),
            boxed,
            Arc::clone(ctx),
        ),
        None => boxed,
    };
    Tool {
        name: name.to_string(),
        description: description.to_string(),
        input_schema: schemars::schema_for!(T),
        execute: ToolExecute::new(final_execute),
    }
}

pub fn build_tools(
    backend: ShellBackend,
    cwd_ref: Arc<RwLock<String>>,
    permission_ctx: Option<Arc<PermissionContext>>,
) -> Vec<Tool> {
    let pctx = &permission_ctx;

    vec![
        {
            let backend = backend.clone();
            let cwd_ref = Arc::clone(&cwd_ref);
            make_tool_with_permissions::<BashInput>(
                "bash",
                "Execute a bash command and return its output. Pagers are disabled and stdin is /dev/null — do not run interactive/TUI programs (vim, top, less, htop, nano, etc.) as they will hang. Use for running shell commands, installing packages, running tests, etc.",
                move |v| {
                    let input: BashInput = serde_json::from_value(v)
                        .map_err(|e| format!("invalid bash input: {}", e))?;
                    backend.execute(&input.command, input.timeout_ms, &cwd_ref)
                },
                pctx.as_ref(),
            )
        },
        make_tool_with_permissions::<read::ReadInput>(
            "read",
            "Read the contents of a file. Returns numbered lines. Use offset and limit for large files.",
            read::execute,
            pctx.as_ref(),
        ),
        make_tool_with_permissions::<write::WriteInput>(
            "write",
            "Write content to a file. Creates parent directories if needed. Overwrites existing content.",
            write::execute,
            pctx.as_ref(),
        ),
        make_tool_with_permissions::<edit::EditInput>(
            "edit",
            "Replace a string in a file. The old_string must be unique unless replace_all is true.",
            edit::execute,
            pctx.as_ref(),
        ),
        {
            let cwd_ref = Arc::clone(&cwd_ref);
            make_tool_with_permissions::<glob::GlobInput>(
                "glob",
                "Find files matching a glob pattern. Returns sorted list of file paths.",
                move |v| {
                    let cwd = cwd_ref.read().unwrap().clone();
                    glob::execute(v, &cwd)
                },
                pctx.as_ref(),
            )
        },
        {
            let cwd_ref = Arc::clone(&cwd_ref);
            make_tool_with_permissions::<grep::GrepInput>(
                "grep",
                "Search file contents using regex. Returns matching lines with file paths and line numbers.",
                move |v| {
                    let cwd = cwd_ref.read().unwrap().clone();
                    grep::execute(v, &cwd)
                },
                pctx.as_ref(),
            )
        },
    ]
}
