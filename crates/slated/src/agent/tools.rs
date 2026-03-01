use std::sync::Arc;

use aisdk::core::tools::{Tool, ToolExecute};
use serde_json::Value;

use slate_tools::tools::{bash, edit, glob, grep, read, write};

fn make_tool<T: schemars::JsonSchema>(
    name: &str,
    description: &str,
    execute: impl Fn(Value) -> Result<String, String> + Send + Sync + 'static,
) -> Tool {
    Tool {
        name: name.to_string(),
        description: description.to_string(),
        input_schema: schemars::schema_for!(T),
        execute: ToolExecute::new(Box::new(execute)),
    }
}

pub fn build_tools(
    cwd: String,
    env_vars: Vec<(String, String)>,
) -> Vec<Tool> {
    let cwd = Arc::new(cwd);
    let env_vars = Arc::new(env_vars);

    vec![
        {
            let cwd = Arc::clone(&cwd);
            let env_vars = Arc::clone(&env_vars);
            make_tool::<bash::BashInput>(
                "bash",
                "Execute a bash command and return its output. Use for running shell commands, installing packages, running tests, etc.",
                move |v| bash::execute(v, &cwd, &env_vars),
            )
        },
        make_tool::<read::ReadInput>(
            "read",
            "Read the contents of a file. Returns numbered lines. Use offset and limit for large files.",
            |v| read::execute(v),
        ),
        make_tool::<write::WriteInput>(
            "write",
            "Write content to a file. Creates parent directories if needed. Overwrites existing content.",
            |v| write::execute(v),
        ),
        make_tool::<edit::EditInput>(
            "edit",
            "Replace a unique string in a file. The old_string must appear exactly once in the file.",
            |v| edit::execute(v),
        ),
        {
            let cwd = Arc::clone(&cwd);
            make_tool::<glob::GlobInput>(
                "glob",
                "Find files matching a glob pattern. Returns sorted list of file paths.",
                move |v| glob::execute(v, &cwd),
            )
        },
        {
            let cwd = Arc::clone(&cwd);
            make_tool::<grep::GrepInput>(
                "grep",
                "Search file contents using regex. Returns matching lines with file paths and line numbers.",
                move |v| grep::execute(v, &cwd),
            )
        },
    ]
}
