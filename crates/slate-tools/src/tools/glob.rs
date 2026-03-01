use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, schemars::JsonSchema)]
pub struct GlobInput {
    /// The glob pattern to match files against (e.g. "**/*.rs")
    pub pattern: String,
    /// Directory to search in (defaults to cwd)
    pub path: Option<String>,
}

pub fn execute(value: Value, cwd: &str) -> Result<String, String> {
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
