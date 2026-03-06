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
}

/// Classify a raw input line into an [`InputAction`].
pub fn classify_input(input: &str) -> InputAction {
    let trimmed = input.trim();
    if trimmed.is_empty() { return InputAction::Empty; }
    if trimmed == "exit" || trimmed == "quit" { return InputAction::Exit; }
    if trimmed == "clear" { return InputAction::Clear; }
    if trimmed == "reset" { return InputAction::Reset; }
    InputAction::Submit
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
    }

    #[test]
    fn classify_submit() {
        assert_eq!(classify_input("ls -la"), InputAction::Submit);
        assert_eq!(classify_input("hello world"), InputAction::Submit);
        assert_eq!(classify_input("?what is rust"), InputAction::Submit);
        assert_eq!(classify_input("cd /tmp"), InputAction::Submit);
        assert_eq!(classify_input("FOO=bar"), InputAction::Submit);
    }
}
