use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::Arc;

use aisdk::core::tools::{Tool, ToolExecute};
use serde::Deserialize;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Parameter structs
// ---------------------------------------------------------------------------

#[derive(Deserialize, schemars::JsonSchema)]
struct BashInput {
    /// The bash command to execute
    command: String,
    /// Timeout in milliseconds (default: 30000)
    #[serde(default = "default_timeout")]
    timeout_ms: u64,
}

fn default_timeout() -> u64 {
    30000
}

#[derive(Deserialize, schemars::JsonSchema)]
struct ReadInput {
    /// Absolute path to the file to read
    file_path: String,
    /// Line number to start reading from (1-based)
    offset: Option<usize>,
    /// Number of lines to read
    limit: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct WriteInput {
    /// Absolute path to the file to write
    file_path: String,
    /// Content to write to the file
    content: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct EditInput {
    /// Absolute path to the file to edit
    file_path: String,
    /// The exact string to find and replace (must be unique in the file)
    old_string: String,
    /// The string to replace it with
    new_string: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct GlobInput {
    /// The glob pattern to match files against (e.g. "**/*.rs")
    pattern: String,
    /// Directory to search in (defaults to cwd)
    path: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct GrepInput {
    /// The regex pattern to search for
    pattern: String,
    /// File or directory to search in (defaults to cwd)
    path: Option<String>,
    /// Glob pattern to filter files (e.g. "*.rs")
    include: Option<String>,
}

// ---------------------------------------------------------------------------
// Helper: construct a Tool with schema derived from T
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

fn exec_bash(
    value: Value,
    cwd: &str,
    env_vars: &[(String, String)],
) -> Result<String, String> {
    let input: BashInput =
        serde_json::from_value(value).map_err(|e| format!("invalid bash input: {e}"))?;

    tracing::debug!(command = %input.command, timeout_ms = input.timeout_ms, "bash");

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "bash".into());
    let mut child = Command::new(&shell)
        .arg("-c")
        .arg(&input.command)
        .current_dir(cwd)
        .envs(env_vars.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn bash: {e}"))?;

    let mut result = String::new();

    // Stream stdout line-by-line
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) => {
                    result.push_str(&line);
                    result.push('\n');
                }
                Err(e) => {
                    let err_line = format!("[read error: {e}]");
                    result.push_str(&err_line);
                    result.push('\n');
                }
            }
        }
    }

    // Read stderr after stdout drains
    if let Some(stderr) = child.stderr.take() {
        let stderr_output: String = BufReader::new(stderr)
            .lines()
            .filter_map(|l| l.ok())
            .collect::<Vec<_>>()
            .join("\n");
        if !stderr_output.is_empty() {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(&stderr_output);
        }
    }

    let status = child.wait().map_err(|e| format!("wait error: {e}"))?;

    if !status.success() {
        let code = status.code().unwrap_or(-1);
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(&format!("exit code: {code}"));
    }

    Ok(result)
}

fn exec_read(value: Value) -> Result<String, String> {
    let input: ReadInput =
        serde_json::from_value(value).map_err(|e| format!("invalid read input: {e}"))?;

    tracing::debug!(file_path = %input.file_path, "read");

    let content = std::fs::read_to_string(&input.file_path)
        .map_err(|e| format!("failed to read {}: {e}", input.file_path))?;

    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();

    let start = input.offset.unwrap_or(1).saturating_sub(1).min(total);
    let count = input.limit.unwrap_or(total - start);
    let end = (start + count).min(total);

    let mut result = String::new();
    for (i, line) in lines[start..end].iter().enumerate() {
        let line_num = start + i + 1;
        result.push_str(&format!("{line_num:>6}\t{line}\n"));
    }

    Ok(result)
}

fn exec_write(value: Value) -> Result<String, String> {
    let input: WriteInput =
        serde_json::from_value(value).map_err(|e| format!("invalid write input: {e}"))?;

    tracing::debug!(file_path = %input.file_path, "write");

    if let Some(parent) = std::path::Path::new(&input.file_path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create directories for {}: {e}", input.file_path))?;
    }

    std::fs::write(&input.file_path, &input.content)
        .map_err(|e| format!("failed to write {}: {e}", input.file_path))?;

    Ok(format!(
        "Wrote {} bytes to {}",
        input.content.len(),
        input.file_path
    ))
}

fn exec_edit(value: Value) -> Result<String, String> {
    let input: EditInput =
        serde_json::from_value(value).map_err(|e| format!("invalid edit input: {e}"))?;

    tracing::debug!(file_path = %input.file_path, "edit");

    let content = std::fs::read_to_string(&input.file_path)
        .map_err(|e| format!("failed to read {}: {e}", input.file_path))?;

    let count = content.matches(&input.old_string).count();
    match count {
        0 => return Err("old_string not found in file".to_string()),
        1 => {}
        n => {
            return Err(format!(
                "old_string is not unique in file (found {n} occurrences)"
            ))
        }
    }

    let new_content = content.replacen(&input.old_string, &input.new_string, 1);
    std::fs::write(&input.file_path, &new_content)
        .map_err(|e| format!("failed to write {}: {e}", input.file_path))?;

    Ok(format!("Edited {}", input.file_path))
}

fn exec_glob(value: Value, cwd: &str) -> Result<String, String> {
    let input: GlobInput =
        serde_json::from_value(value).map_err(|e| format!("invalid glob input: {e}"))?;

    let base = input.path.as_deref().unwrap_or(cwd);
    let full_pattern = format!("{base}/{}", input.pattern);

    tracing::debug!(pattern = %full_pattern, "glob");

    let mut paths: Vec<String> = glob::glob(&full_pattern)
        .map_err(|e| format!("invalid glob pattern: {e}"))?
        .filter_map(|entry| entry.ok())
        .map(|p| p.display().to_string())
        .collect();

    paths.sort();
    Ok(paths.join("\n"))
}

/// Maximum number of matching lines to return from grep.
const GREP_MAX_MATCHES: usize = 500;

fn exec_grep(value: Value, cwd: &str) -> Result<String, String> {
    use grep_regex::RegexMatcher;
    use grep_searcher::sinks::UTF8;
    use grep_searcher::Searcher;
    use ignore::WalkBuilder;

    let input: GrepInput =
        serde_json::from_value(value).map_err(|e| format!("invalid grep input: {e}"))?;

    let search_path = input.path.as_deref().unwrap_or(cwd);

    tracing::debug!(pattern = %input.pattern, path = %search_path, "grep");

    let matcher =
        RegexMatcher::new(&input.pattern).map_err(|e| format!("invalid regex pattern: {e}"))?;

    let mut result = String::new();
    let mut searcher = Searcher::new();
    let mut match_count: usize = 0;

    let root = std::path::Path::new(search_path);
    if root.is_file() {
        searcher
            .search_path(
                &matcher,
                root,
                UTF8(|line_num, line| {
                    if match_count >= GREP_MAX_MATCHES {
                        return Ok(false);
                    }
                    match_count += 1;
                    result.push_str(&format!("{}:{}:{}", root.display(), line_num, line));
                    if !line.ends_with('\n') {
                        result.push('\n');
                    }
                    Ok(true)
                }),
            )
            .map_err(|e| format!("grep error: {e}"))?;
    } else {
        // Use ignore crate for .gitignore-aware recursive walking
        let walker = WalkBuilder::new(root)
            .hidden(true) // skip hidden files
            .git_ignore(true) // respect .gitignore
            .git_global(true)
            .git_exclude(true)
            .build();

        'outer: for entry in walker.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            // Apply include filter if provided
            if let Some(ref pattern) = input.include {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !glob::Pattern::new(pattern)
                    .map(|p| p.matches(name))
                    .unwrap_or(false)
                {
                    continue;
                }
            }

            let display_path = path.display().to_string();
            let _ = searcher.search_path(
                &matcher,
                path,
                UTF8(|line_num, line| {
                    if match_count >= GREP_MAX_MATCHES {
                        return Ok(false);
                    }
                    match_count += 1;
                    result.push_str(&format!("{}:{}:{}", display_path, line_num, line));
                    if !line.ends_with('\n') {
                        result.push('\n');
                    }
                    Ok(true)
                }),
            );

            if match_count >= GREP_MAX_MATCHES {
                break 'outer;
            }
        }
    }

    if match_count >= GREP_MAX_MATCHES {
        result.push_str(&format!("\n[truncated: showing first {} matches]", GREP_MAX_MATCHES));
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Public factory
// ---------------------------------------------------------------------------

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
            make_tool::<BashInput>(
                "bash",
                "Execute a bash command and return its output. Use for running shell commands, installing packages, running tests, etc.",
                move |v| exec_bash(v, &cwd, &env_vars),
            )
        },
        make_tool::<ReadInput>(
            "read",
            "Read the contents of a file. Returns numbered lines. Use offset and limit for large files.",
            |v| exec_read(v),
        ),
        make_tool::<WriteInput>(
            "write",
            "Write content to a file. Creates parent directories if needed. Overwrites existing content.",
            |v| exec_write(v),
        ),
        make_tool::<EditInput>(
            "edit",
            "Replace a unique string in a file. The old_string must appear exactly once in the file.",
            |v| exec_edit(v),
        ),
        {
            let cwd = Arc::clone(&cwd);
            make_tool::<GlobInput>(
                "glob",
                "Find files matching a glob pattern. Returns sorted list of file paths.",
                move |v| exec_glob(v, &cwd),
            )
        },
        {
            let cwd = Arc::clone(&cwd);
            make_tool::<GrepInput>(
                "grep",
                "Search file contents using regex. Returns matching lines with file paths and line numbers.",
                move |v| exec_grep(v, &cwd),
            )
        },
    ]
}
