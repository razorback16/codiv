use std::sync::Arc;

use aisdk::core::tools::{Tool, ToolExecute};
use serde_json::Value;

use slate_tools::tools::{bash, edit, glob, grep, read, write};

use super::permissions::PermissionContext;

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
    cwd: String,
    env_vars: Vec<(String, String)>,
    permission_ctx: Option<Arc<PermissionContext>>,
) -> Vec<Tool> {
    let cwd = Arc::new(cwd);
    let env_vars = Arc::new(env_vars);
    let pctx = &permission_ctx;

    vec![
        {
            let cwd = Arc::clone(&cwd);
            let env_vars = Arc::clone(&env_vars);
            make_tool_with_permissions::<bash::BashInput>(
                "bash",
                "Execute a bash command and return its output. Use for running shell commands, installing packages, running tests, etc.",
                move |v| bash::execute(v, &cwd, &env_vars),
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
            "Replace a unique string in a file. The old_string must appear exactly once in the file.",
            edit::execute,
            pctx.as_ref(),
        ),
        {
            let cwd = Arc::clone(&cwd);
            make_tool_with_permissions::<glob::GlobInput>(
                "glob",
                "Find files matching a glob pattern. Returns sorted list of file paths.",
                move |v| glob::execute(v, &cwd),
                pctx.as_ref(),
            )
        },
        {
            let cwd = Arc::clone(&cwd);
            make_tool_with_permissions::<grep::GrepInput>(
                "grep",
                "Search file contents using regex. Returns matching lines with file paths and line numbers.",
                move |v| grep::execute(v, &cwd),
                pctx.as_ref(),
            )
        },
    ]
}
