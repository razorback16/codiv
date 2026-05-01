use crate::ui::theme::Theme;

/// Split ANSI bytes by `\r\n` and append complete lines to the accumulator.
/// Partial lines (no trailing `\r\n`) are appended to the last entry or added as a new entry.
pub(super) fn accumulate_ai_lines(acc: &mut Vec<String>, ansi: &[u8]) {
    let text = String::from_utf8_lossy(ansi);
    let mut parts: Vec<&str> = text.split("\r\n").collect();
    // If text ends with \r\n, split produces a trailing empty string — pop it.
    if text.ends_with("\r\n") && parts.last() == Some(&"") {
        parts.pop();
    }
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            // Append to the last incomplete line, or start a new one.
            if let Some(last) = acc.last_mut() {
                last.push_str(part);
            } else {
                acc.push(part.to_string());
            }
            // If there are more parts, this line is now complete — next parts start new lines.
        } else {
            acc.push(part.to_string());
        }
    }
    // If the original text ended with \r\n, the current last line is complete.
    // Push an empty string to start a new line for future appends.
    if text.ends_with("\r\n") {
        acc.push(String::new());
    }
}

/// Build the ANSI-formatted rendered lines for a tool block result.
/// Each line does NOT include trailing `\r\n` — the caller adds that.
pub(super) fn build_tool_rendered_lines(
    tool_name: &str,
    header: &str,
    summary: &str,
    preview_lines: &[String],
    perm: Option<&(String, bool, String)>,
    theme: &Theme,
) -> Vec<String> {
    let mut lines = Vec::new();

    if let Some((_perm_tool, granted, reason)) = perm {
        if *granted {
            lines.push(format!("{}{}\x1b[0m", theme.ansi_tool_done, header));
            lines.push(format!("{}  \u{2514} {}\x1b[0m", theme.ansi_tool_done_suffix, reason));
            let is_tool_error = (tool_name.eq_ignore_ascii_case("bash")
                && !summary.contains("exit 0"))
                || summary.contains(" failed:");
            let color = if is_tool_error {
                theme.ansi_exit_failure
            } else {
                theme.ansi_tool_done_suffix
            };
            lines.push(format!("{}{}\x1b[0m", color, summary));
            for pline in preview_lines {
                lines.push(format!("{}{}\x1b[0m", theme.ansi_thinking, pline));
            }
        } else {
            lines.push(format!("{}{}\x1b[0m", theme.ansi_tool_denied, header));
            lines.push(format!("{}  \u{2514} {}\x1b[0m", theme.ansi_tool_denied_suffix, reason));
        }
    } else {
        lines.push(format!("{}{}\x1b[0m", theme.ansi_tool_done, header));
        let is_tool_error = (tool_name.eq_ignore_ascii_case("bash")
            && !summary.contains("exit 0"))
            || summary.contains(" failed:");
        let color = if is_tool_error {
            theme.ansi_exit_failure
        } else {
            theme.ansi_tool_done_suffix
        };
        lines.push(format!("{}{}\x1b[0m", color, summary));
        for pline in preview_lines {
            lines.push(format!("{}{}\x1b[0m", theme.ansi_thinking, pline));
        }
    }

    lines
}
