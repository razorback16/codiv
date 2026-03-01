use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, schemars::JsonSchema)]
pub struct BashInput {
    /// The bash command to execute
    pub command: String,
    /// Timeout in milliseconds (default: 30000)
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

fn default_timeout() -> u64 {
    30000
}

pub fn execute(value: Value, cwd: &str, env_vars: &[(String, String)]) -> Result<String, String> {
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

    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) => {
                    result.push_str(&line);
                    result.push('\n');
                }
                Err(e) => {
                    result.push_str(&format!("[read error: {e}]"));
                    result.push('\n');
                }
            }
        }
    }

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
