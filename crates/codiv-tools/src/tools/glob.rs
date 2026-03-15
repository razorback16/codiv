use serde::Deserialize;
use serde_json::Value;

/// Default maximum number of matching files to return from glob.
const DEFAULT_MAX_RESULTS: usize = 1000;

#[derive(Deserialize, schemars::JsonSchema)]
pub struct GlobInput {
    /// The glob pattern to match files against (e.g. "**/*.rs")
    pub pattern: String,
    /// Directory to search in (defaults to cwd)
    pub path: Option<String>,
    /// Maximum files to return. Defaults to 1000. Use -1 for unlimited.
    pub max_results: Option<i64>,
}

pub fn execute(value: Value, cwd: &str) -> Result<String, String> {
    let input: GlobInput =
        serde_json::from_value(value).map_err(|e| format!("invalid glob input: {e}"))?;

    let base = input.path.as_deref().unwrap_or(cwd);
    let full_pattern = format!("{base}/{}", input.pattern);

    let max_results: Option<usize> = match input.max_results {
        Some(n) if n < 0 => None,
        Some(n) => Some(n as usize),
        None => Some(DEFAULT_MAX_RESULTS),
    };

    tracing::debug!(pattern = %full_pattern, ?max_results, "glob");

    let mut paths: Vec<String> = glob::glob(&full_pattern)
        .map_err(|e| format!("invalid glob pattern: {e}"))?
        .filter_map(|entry| entry.ok())
        .map(|p| p.display().to_string())
        .collect();

    paths.sort();

    if let Some(limit) = max_results {
        let total = paths.len();
        if total > limit {
            paths.truncate(limit);
            let mut result = paths.join("\n");
            result.push_str(&format!(
                "\n[truncated: showing first {limit} of {total} matching files]"
            ));
            return Ok(result);
        }
    }

    Ok(paths.join("\n"))
}
