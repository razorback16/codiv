use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, schemars::JsonSchema)]
pub struct GrepInput {
    /// The regex pattern to search for
    pub pattern: String,
    /// File or directory to search in (defaults to cwd)
    pub path: Option<String>,
    /// Glob pattern to filter files (e.g. "*.rs")
    pub include: Option<String>,
}

/// Maximum number of matching lines to return from grep.
const GREP_MAX_MATCHES: usize = 500;

pub fn execute(value: Value, cwd: &str) -> Result<String, String> {
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
        let walker = WalkBuilder::new(root)
            .hidden(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .build();

        'outer: for entry in walker.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

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
        result.push_str(&format!(
            "\n[truncated: showing first {} matches]",
            GREP_MAX_MATCHES
        ));
    }

    Ok(result)
}
