use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, schemars::JsonSchema)]
pub struct WriteInput {
    /// Absolute path to the file to write
    pub file_path: String,
    /// Content to write to the file
    pub content: String,
}

pub fn execute(value: Value) -> Result<String, String> {
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
