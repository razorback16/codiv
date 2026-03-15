use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, schemars::JsonSchema)]
pub struct EditInput {
    /// Absolute path to the file to edit
    pub file_path: String,
    /// The exact string to find and replace (must be unique in the file)
    pub old_string: String,
    /// The string to replace it with
    pub new_string: String,
}

pub fn execute(value: Value) -> Result<String, String> {
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
