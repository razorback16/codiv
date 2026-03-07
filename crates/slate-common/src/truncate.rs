/// Maximum bytes for a single tool output before truncation.
pub const MAX_TOOL_OUTPUT_BYTES: usize = 256 * 1024; // 256KB

/// Truncate tool output if it exceeds `max_bytes`.
/// First tries line-based truncation (200 head / 100 tail lines).
/// If still over limit, does hard byte-level truncation at UTF-8 char boundaries.
pub fn truncate_tool_output(output: &str, max_bytes: usize) -> String {
    if output.len() <= max_bytes {
        return output.to_string();
    }

    // First try line-based truncation
    let truncated = truncate_output(output, 200, 100);
    if truncated.len() <= max_bytes {
        return truncated;
    }

    // Hard byte-level truncation as last resort
    let head_bytes = max_bytes * 2 / 3;
    let tail_bytes = max_bytes / 4;

    // Find safe UTF-8 char boundaries
    let head_end = floor_char_boundary(output, head_bytes);
    let tail_start = ceil_char_boundary(output, output.len().saturating_sub(tail_bytes));

    let omitted = output.len() - head_end - (output.len() - tail_start);
    format!(
        "{}\n... ({} bytes omitted) ...\n{}",
        &output[..head_end],
        omitted,
        &output[tail_start..]
    )
}

// Helper: find largest index <= pos that is a char boundary
fn floor_char_boundary(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    let mut i = pos;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

// Helper: find smallest index >= pos that is a char boundary
fn ceil_char_boundary(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    let mut i = pos;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Truncate long output keeping the first `head_lines` and last `tail_lines`.
/// If the output fits within head + tail lines, return it unchanged.
/// Otherwise, insert an omission marker in the middle.
pub fn truncate_output(output: &str, head_lines: usize, tail_lines: usize) -> String {
    if output.is_empty() {
        return String::new();
    }

    let lines: Vec<&str> = output.lines().collect();
    let total = lines.len();

    if total <= head_lines + tail_lines {
        return output.to_string();
    }

    let head: Vec<&str> = lines[..head_lines].to_vec();
    let tail: Vec<&str> = lines[total - tail_lines..].to_vec();
    let omitted = total - head_lines - tail_lines;

    format!(
        "{}\n... ({} lines omitted) ...\n{}",
        head.join("\n"),
        omitted,
        tail.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_output_unchanged() {
        let input = "line1\nline2\nline3";
        assert_eq!(truncate_output(input, 20, 20), input);
    }

    #[test]
    fn empty_output_unchanged() {
        assert_eq!(truncate_output("", 20, 20), "");
    }

    #[test]
    fn exact_boundary_unchanged() {
        let lines: Vec<String> = (1..=40).map(|i| format!("line {i}")).collect();
        let input = lines.join("\n");
        assert_eq!(truncate_output(&input, 20, 20), input);
    }

    #[test]
    fn truncates_long_output() {
        let lines: Vec<String> = (1..=100).map(|i| format!("line {i}")).collect();
        let input = lines.join("\n");
        let result = truncate_output(&input, 3, 3);
        assert!(result.contains("line 1"));
        assert!(result.contains("line 3"));
        assert!(result.contains("line 98"));
        assert!(result.contains("line 100"));
        assert!(result.contains("94 lines omitted"));
        assert!(!result.contains("line 50"));
    }

    #[test]
    fn tool_output_under_limit_unchanged() {
        let input = "short output";
        assert_eq!(truncate_tool_output(input, 1024), input);
    }

    #[test]
    fn tool_output_empty_string() {
        assert_eq!(truncate_tool_output("", 1024), "");
    }

    #[test]
    fn tool_output_large_gets_line_truncated() {
        // Create output with 500 lines, each ~20 bytes => ~10KB total
        let lines: Vec<String> = (1..=500).map(|i| format!("line number {i:>04}")).collect();
        let input = lines.join("\n");
        let result = truncate_tool_output(&input, 256); // very small limit to force truncation
        // Should contain omission marker
        assert!(
            result.contains("omitted"),
            "expected truncation marker in: {result}"
        );
    }

    #[test]
    fn tool_output_multibyte_utf8_no_panic() {
        // 4-byte UTF-8 chars (emoji)
        let input = "🎉".repeat(1000); // 4000 bytes
        let result = truncate_tool_output(&input, 100);
        // Should not panic and should be valid UTF-8
        assert!(result.len() <= 200); // some overhead from marker
        // Verify it's valid UTF-8 by just using it as &str
        let _ = result.as_str();
    }

    #[test]
    fn single_line_head_tail() {
        let lines: Vec<String> = (1..=10).map(|i| format!("line {i}")).collect();
        let input = lines.join("\n");
        let result = truncate_output(&input, 1, 1);
        assert!(result.starts_with("line 1\n"));
        assert!(result.ends_with("line 10"));
        assert!(result.contains("8 lines omitted"));
    }
}
