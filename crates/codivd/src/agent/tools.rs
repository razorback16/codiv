use std::sync::Arc;

use aisdk::core::tools::{Tool, ToolExecute};
use serde_json::Value;

use codiv_tools::tools::bash::BashInput;
use codiv_tools::tools::{edit, glob, grep, read, write};

use super::permissions::PermissionContext;
use super::relay_manager::RelayManager;

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
        execute: ToolExecute::from_sync(move |_ctx, input| (final_execute)(input)),
    }
}

fn make_async_tool_with_permissions<T: schemars::JsonSchema, F, Fut>(
    name: &str,
    description: &str,
    execute: F,
    permission_ctx: Option<&Arc<PermissionContext>>,
) -> Tool
where
    F: Fn(Value) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<String, String>> + Send + 'static,
{
    let final_execute: Arc<
        dyn Fn(Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
            + Send
            + Sync,
    > = match permission_ctx {
        Some(ctx) => {
            let wrapped = super::permissions::wrap_with_permissions_async(
                name.to_string(),
                execute,
                Arc::clone(ctx),
            );
            Arc::new(wrapped)
        }
        None => {
            let f = Arc::new(execute);
            Arc::new(move |v: Value| -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>> {
                let f = Arc::clone(&f);
                Box::pin(async move { f(v).await })
            })
        }
    };

    Tool {
        name: name.to_string(),
        description: description.to_string(),
        input_schema: schemars::schema_for!(T),
        execute: ToolExecute::from_async(move |_ctx, input| {
            let exec = Arc::clone(&final_execute);
            async move { exec(input).await }
        }),
    }
}

pub fn build_tools(
    relay: Arc<RelayManager>,
    cwd: String,
    permission_ctx: Option<Arc<PermissionContext>>,
) -> Vec<Tool> {
    let pctx = &permission_ctx;

    vec![
        {
            let relay = Arc::clone(&relay);
            make_async_tool_with_permissions::<BashInput, _, _>(
                "bash",
                "Execute a bash command and return its output. Pagers are disabled and stdin is /dev/null — do not run interactive/TUI programs (vim, top, less, htop, nano, etc.) as they will hang. Use for running shell commands, installing packages, running tests, etc.",
                move |v| {
                    let relay = Arc::clone(&relay);
                    async move {
                        let input: BashInput = serde_json::from_value(v)
                            .map_err(|e| format!("invalid bash input: {}", e))?;
                        relay
                            .execute_bash(
                                String::new(),
                                input.command,
                                input.timeout_ms,
                            )
                            .await
                    }
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
            let cwd = cwd.clone();
            make_tool_with_permissions::<glob::GlobInput>(
                "glob",
                "Find files matching a glob pattern. Returns sorted list of file paths.",
                move |v| {
                    glob::execute(v, &cwd)
                },
                pctx.as_ref(),
            )
        },
        {
            let cwd = cwd;
            make_tool_with_permissions::<grep::GrepInput>(
                "grep",
                "Search file contents using regex. Returns matching lines with file paths and line numbers.",
                move |v| {
                    grep::execute(v, &cwd)
                },
                pctx.as_ref(),
            )
        },
    ]
}
