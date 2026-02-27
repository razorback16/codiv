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
    fn single_line_head_tail() {
        let lines: Vec<String> = (1..=10).map(|i| format!("line {i}")).collect();
        let input = lines.join("\n");
        let result = truncate_output(&input, 1, 1);
        assert!(result.starts_with("line 1\n"));
        assert!(result.ends_with("line 10"));
        assert!(result.contains("8 lines omitted"));
    }
}
