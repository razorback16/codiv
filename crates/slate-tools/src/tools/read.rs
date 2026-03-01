use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ReadInput {
    /// Absolute path to the file to read
    pub file_path: String,
    /// Line number to start reading from (1-based)
    pub offset: Option<usize>,
    /// Number of lines to read
    pub limit: Option<usize>,
}

pub fn execute(value: Value) -> Result<String, String> {
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
