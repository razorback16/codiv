//! Input classification for the shell dispatcher.

// ---------------------------------------------------------------------------
// Input classification
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub enum InputAction {
    /// Route according to current InputMode.
    Submit,
    /// Clear screen and scrollback.
    Clear,
    /// Full session reset.
    Reset,
    /// User wants to exit.
    Exit,
    /// Empty input line.
    Empty,
    /// List and select saved sessions.
    Sessions,
    /// Manually compact the conversation history.
    Compact,
    /// Unknown slash command (e.g. "/foo").
    UnknownCommand(String),
}

/// Classify a raw input line into an [`InputAction`].
pub fn classify_input(input: &str) -> InputAction {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return InputAction::Empty;
    }
    // Slash commands
    if let Some(cmd) = trimmed.strip_prefix('/') {
        return match cmd {
            "exit" | "quit" => InputAction::Exit,
            "clear" => InputAction::Clear,
            "compact" => InputAction::Compact,
            "reset" => InputAction::Reset,
            "sessions" => InputAction::Sessions,
            _ => InputAction::UnknownCommand(trimmed.to_string()),
        };
    }
    // Legacy bare commands (only exit/quit kept for convenience)
    if trimmed == "exit" || trimmed == "quit" {
        return InputAction::Exit;
    }
    InputAction::Submit
}

/// Returns slash commands matching the given prefix (e.g. "/cl" → ["/clear"]).
pub fn complete_slash_command(prefix: &str) -> Vec<String> {
    const COMMANDS: &[&str] = &["/clear", "/compact", "/exit", "/quit", "/reset", "/sessions"];
    COMMANDS
        .iter()
        .filter(|cmd| cmd.starts_with(prefix))
        .map(|cmd| cmd.to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_empty() {
        assert_eq!(classify_input(""), InputAction::Empty);
        assert_eq!(classify_input("   "), InputAction::Empty);
    }

    #[test]
    fn classify_exit() {
        assert_eq!(classify_input("exit"), InputAction::Exit);
        assert_eq!(classify_input("quit"), InputAction::Exit);
        assert_eq!(classify_input("/exit"), InputAction::Exit);
        assert_eq!(classify_input("/quit"), InputAction::Exit);
    }

    #[test]
    fn classify_slash_commands() {
        assert_eq!(classify_input("/clear"), InputAction::Clear);
        assert_eq!(classify_input("/compact"), InputAction::Compact);
        assert_eq!(classify_input("/reset"), InputAction::Reset);
        assert_eq!(classify_input("/sessions"), InputAction::Sessions);
    }

    #[test]
    fn classify_unknown_command() {
        assert_eq!(
            classify_input("/foo"),
            InputAction::UnknownCommand("/foo".to_string())
        );
    }

    #[test]
    fn classify_bare_words_are_submit() {
        // "clear" and "reset" without slash are now Submit
        assert_eq!(classify_input("clear"), InputAction::Submit);
        assert_eq!(classify_input("reset"), InputAction::Submit);
    }

    #[test]
    fn classify_submit() {
        assert_eq!(classify_input("ls -la"), InputAction::Submit);
        assert_eq!(classify_input("hello world"), InputAction::Submit);
        assert_eq!(classify_input("?what is rust"), InputAction::Submit);
        assert_eq!(classify_input("cd /tmp"), InputAction::Submit);
        assert_eq!(classify_input("FOO=bar"), InputAction::Submit);
    }

    #[test]
    fn complete_slash_prefix() {
        let results = complete_slash_command("/cl");
        assert_eq!(results, vec!["/clear"]);
    }

    #[test]
    fn complete_slash_multiple() {
        let results = complete_slash_command("/");
        assert_eq!(results.len(), 6);
    }

    #[test]
    fn complete_slash_no_match() {
        let results = complete_slash_command("/zzz");
        assert!(results.is_empty());
    }
}
