//! Unified diff generation using the `similar` crate.

use similar::TextDiff;

/// Generate a unified diff between `old` and `new` text for a given file path.
///
/// Returns plain text (no ANSI codes) — coloring is applied by the modal
/// renderer. Returns an empty string if old and new are identical.
pub fn generate_unified_diff(file_path: &str, old: &str, new: &str) -> String {
    if old == new {
        return String::new();
    }

    let diff = TextDiff::from_lines(old, new);
    diff.unified_diff()
        .context_radius(3)
        .header(&format!("a/{}", file_path), &format!("b/{}", file_path))
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_returns_empty() {
        assert_eq!(generate_unified_diff("f.rs", "hello\n", "hello\n"), "");
    }

    #[test]
    fn basic_diff() {
        let old = "line1\nline2\nline3\n";
        let new = "line1\nchanged\nline3\n";
        let diff = generate_unified_diff("f.rs", old, new);
        assert!(diff.contains("-line2"));
        assert!(diff.contains("+changed"));
        assert!(diff.contains("--- a/f.rs"));
        assert!(diff.contains("+++ b/f.rs"));
    }
}
