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
    /// Maximum number of matches to return. Defaults to 500. Use -1 for unlimited.
    pub max_matches: Option<i64>,
}

/// Default maximum number of matching lines to return from grep.
const DEFAULT_MAX_MATCHES: usize = 500;

pub fn execute(value: Value, cwd: &str) -> Result<String, String> {
    use grep_regex::RegexMatcher;
    use grep_searcher::sinks::UTF8;
    use grep_searcher::Searcher;
    use ignore::{WalkBuilder, WalkState};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    let input: GrepInput =
        serde_json::from_value(value).map_err(|e| format!("invalid grep input: {e}"))?;

    let search_path = input.path.as_deref().unwrap_or(cwd);

    let max_matches: Option<usize> = match input.max_matches {
        Some(n) if n < 0 => None,
        Some(n) => Some(n as usize),
        None => Some(DEFAULT_MAX_MATCHES),
    };

    tracing::debug!(pattern = %input.pattern, path = %search_path, ?max_matches, "grep");

    let matcher =
        RegexMatcher::new(&input.pattern).map_err(|e| format!("invalid regex pattern: {e}"))?;

    let root = std::path::Path::new(search_path);
    if root.is_file() {
        return search_single_file(&matcher, root, max_matches);
    }

    let glob_pattern = input
        .include
        .as_deref()
        .map(glob::Pattern::new)
        .transpose()
        .map_err(|e| format!("invalid glob pattern: {e}"))?;

    let match_count = Arc::new(AtomicUsize::new(0));
    let hit_limit = Arc::new(AtomicBool::new(false));
    let (tx, rx) = crossbeam_channel::unbounded::<(String, u64, String)>();

    WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build_parallel()
        .run(|| {
            let matcher = matcher.clone();
            let glob_pattern = glob_pattern.clone();
            let match_count = Arc::clone(&match_count);
            let hit_limit = Arc::clone(&hit_limit);
            let tx = tx.clone();

            Box::new(move |entry| {
                if hit_limit.load(Ordering::Relaxed) {
                    return WalkState::Quit;
                }

                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => return WalkState::Continue,
                };

                let path = entry.path();
                if !path.is_file() {
                    return WalkState::Continue;
                }

                if let Some(ref pat) = glob_pattern {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if !pat.matches(name) {
                        return WalkState::Continue;
                    }
                }

                let display_path = path.display().to_string();
                let mut searcher = Searcher::new();
                let _ = searcher.search_path(
                    &matcher,
                    path,
                    UTF8(|line_num, line| {
                        if let Some(m) = max_matches {
                            let prev = match_count.fetch_add(1, Ordering::Relaxed);
                            if prev >= m {
                                hit_limit.store(true, Ordering::Relaxed);
                                return Ok(false);
                            }
                        } else {
                            match_count.fetch_add(1, Ordering::Relaxed);
                        }
                        let _ = tx.send((
                            display_path.clone(),
                            line_num,
                            line.to_string(),
                        ));
                        Ok(true)
                    }),
                );

                if hit_limit.load(Ordering::Relaxed) {
                    WalkState::Quit
                } else {
                    WalkState::Continue
                }
            })
        });

    drop(tx);

    let mut matches: Vec<(String, u64, String)> = rx.iter().collect();
    matches.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    let mut result = String::new();
    for (path, line_num, line) in &matches {
        result.push_str(&format!("{}:{}:{}", path, line_num, line));
        if !line.ends_with('\n') {
            result.push('\n');
        }
    }

    if hit_limit.load(std::sync::atomic::Ordering::Relaxed) {
        if let Some(m) = max_matches {
            result.push_str(&format!("\n[truncated: showing first {} matches]", m));
        }
    }

    Ok(result)
}

fn search_single_file(
    matcher: &grep_regex::RegexMatcher,
    path: &std::path::Path,
    max_matches: Option<usize>,
) -> Result<String, String> {
    use grep_searcher::sinks::UTF8;
    use grep_searcher::Searcher;

    let mut result = String::new();
    let mut searcher = Searcher::new();
    let mut match_count: usize = 0;

    searcher
        .search_path(
            matcher,
            path,
            UTF8(|line_num, line| {
                if max_matches.is_some_and(|m| match_count >= m) {
                    return Ok(false);
                }
                match_count += 1;
                result.push_str(&format!("{}:{}:{}", path.display(), line_num, line));
                if !line.ends_with('\n') {
                    result.push('\n');
                }
                Ok(true)
            }),
        )
        .map_err(|e| format!("grep error: {e}"))?;

    if let Some(m) = max_matches {
        if match_count >= m {
            result.push_str(&format!("\n[truncated: showing first {} matches]", m));
        }
    }

    Ok(result)
}
