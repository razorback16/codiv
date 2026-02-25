//! Completion engine — delegates to bash's compgen and programmable completion.

use std::time::Instant;

use crate::shell::bash_coprocess::BashCoprocess;

/// Result of a completion request.
pub struct CompletionResult {
    /// The completion candidates (full replacement strings).
    pub candidates: Vec<String>,
    /// Byte offset in the input where the word being completed starts.
    pub replace_start: usize,
    /// Byte offset in the input where the word being completed ends (cursor pos).
    pub replace_end: usize,
}

/// Tracks the background initialization state of the completion engine.
enum InitState {
    NotStarted,
    /// Sourcing bash-completion libraries (step 1 of 2).
    Sourcing {
        sentinel: String,
        accumulated: String,
    },
    /// Defining the __slate_complete helper function (step 2 of 2).
    DefiningHelper {
        sentinel: String,
        accumulated: String,
    },
    /// Fully initialized and ready for completion requests.
    Ready,
}

/// Drives tab completion by running a bash helper function in the coprocess.
pub struct CompletionEngine {
    state: InitState,
}

/// The bash helper function that handles all three completion tiers:
/// 1. Programmable completions (COMP_WORDS/COMPREPLY)
/// 2. Command completions (compgen -c) for the first word
/// 3. File completions (compgen -f) for subsequent words
const BASH_HELPER: &str = r#"
__slate_complete() {
    local line="$1"
    local point="$2"

    local before_cursor="${line:0:$point}"

    # Split the portion before cursor into words
    local -a words
    read -ra words <<< "$before_cursor"

    # Handle empty input
    if [[ ${#words[@]} -eq 0 ]]; then
        words=("")
    fi

    local cmd="${words[0]}"
    local cword=$(( ${#words[@]} - 1 ))

    # If cursor is right after a space, we're starting a new empty word
    if [[ $point -gt 0 && "${line:$((point-1)):1}" == " " ]]; then
        words+=("")
        cword=$(( ${#words[@]} - 1 ))
    fi

    # Try programmable completion first
    local comp_spec func
    comp_spec=$(complete -p "$cmd" 2>/dev/null)

    # bash-completion v2 uses a default handler (complete -D) with
    # _completion_loader for lazy loading. If no command-specific spec
    # exists, invoke the default handler to load it, then re-fetch.
    if [[ -z "$comp_spec" ]]; then
        local default_spec
        default_spec=$(complete -p -D 2>/dev/null)
        if [[ -n "$default_spec" ]]; then
            func=$(echo "$default_spec" | sed -n 's/.*-F \([^ ]*\).*/\1/p')
            if [[ -n "$func" ]]; then
                COMP_LINE="$line"
                COMP_POINT=$point
                COMP_WORDS=("${words[@]}")
                COMP_CWORD=$cword
                "$func" "$cmd" 2>/dev/null
                comp_spec=$(complete -p "$cmd" 2>/dev/null)
            fi
        fi
    fi

    if [[ -n "$comp_spec" ]]; then
        func=$(echo "$comp_spec" | sed -n 's/.*-F \([^ ]*\).*/\1/p')

        if [[ -n "$func" ]]; then
            COMP_LINE="$line"
            COMP_POINT=$point
            COMP_WORDS=("${words[@]}")
            COMP_CWORD=$cword
            COMPREPLY=()
            "$func" 2>/dev/null
            if [[ ${#COMPREPLY[@]} -gt 0 ]]; then
                printf '%s\n' "${COMPREPLY[@]}" | head -100
                return
            fi
        fi
    fi

    # Fallback
    local prefix="${words[$cword]}"
    if [[ $cword -eq 0 ]]; then
        compgen -c -- "$prefix" 2>/dev/null | sort -u | head -100
    else
        compgen -f -- "$prefix" 2>/dev/null | sort -u | head -100
    fi
}
"#;

const SOURCE_CMD: &str = "source /usr/share/bash-completion/bash_completion 2>/dev/null || \
             source /opt/homebrew/etc/bash_completion 2>/dev/null || \
             true";

/// Wrap BASH_HELPER in `eval $'...'` so it's sent as a single line to bash.
///
/// Multi-line function definitions sent to an interactive bash PTY get processed
/// line-by-line with PS2 continuation prompts. The sentinel appended by
/// `start_command()` ends up inside the continuation context, causing
/// `bash: syntax error near unexpected token ';'`. Wrapping in `eval $'...'`
/// makes the entire function definition a single logical line — bash processes
/// the `$'...'` quoting atomically, then `eval` defines the function.
fn helper_eval_cmd() -> String {
    let escaped = BASH_HELPER
        .trim()
        .replace('\\', "\\\\") // escape backslashes first
        .replace('\'', "\\'")  // escape single quotes
        .replace('\n', "\\n"); // encode newlines as \n (interpreted by $'...')
    format!("eval $'{}'", escaped)
}

impl CompletionEngine {
    pub fn new() -> Self {
        Self {
            state: InitState::NotStarted,
        }
    }

    /// Kick off background initialization (non-blocking).
    /// Sends the first command (sourcing bash-completion) to the coprocess.
    pub fn start_init(&mut self, bash: &mut BashCoprocess) {
        if !matches!(self.state, InitState::NotStarted) {
            return;
        }
        if let Some(sentinel) = bash.start_command(SOURCE_CMD) {
            self.state = InitState::Sourcing {
                sentinel,
                accumulated: String::new(),
            };
        }
    }

    /// Poll initialization progress (non-blocking). Call each event loop tick
    /// when no user command is pending (they share the coprocess).
    pub fn poll_init(&mut self, bash: &mut BashCoprocess) {
        loop {
            match &mut self.state {
                InitState::Sourcing {
                    sentinel,
                    accumulated,
                } => {
                    let bytes = bash.try_read();
                    if !bytes.is_empty() {
                        accumulated.push_str(&String::from_utf8_lossy(&bytes));
                    }
                    if BashCoprocess::check_complete(accumulated, SOURCE_CMD, sentinel).is_some()
                    {
                        // Step 1 done — start step 2.
                        let helper_cmd = helper_eval_cmd();
                        if let Some(new_sentinel) = bash.start_command(&helper_cmd) {
                            self.state = InitState::DefiningHelper {
                                sentinel: new_sentinel,
                                accumulated: String::new(),
                            };
                            continue;
                        } else {
                            self.state = InitState::NotStarted;
                        }
                    }
                }
                InitState::DefiningHelper {
                    sentinel,
                    accumulated,
                } => {
                    let bytes = bash.try_read();
                    if !bytes.is_empty() {
                        accumulated.push_str(&String::from_utf8_lossy(&bytes));
                    }
                    let helper_cmd = helper_eval_cmd();
                    if BashCoprocess::check_complete(accumulated, &helper_cmd, sentinel).is_some()
                    {
                        self.state = InitState::Ready;
                    }
                }
                _ => {}
            }
            break;
        }
    }

    /// Whether the engine is fully initialized and ready for completions.
    pub fn is_ready(&self) -> bool {
        matches!(self.state, InitState::Ready)
    }

    /// Block until initialization completes. Only called if the user presses
    /// Tab before background init finishes.
    fn finish_init_blocking(&mut self, bash: &mut BashCoprocess) {
        match &self.state {
            InitState::Ready => return,
            InitState::NotStarted => {
                bash.execute(SOURCE_CMD, 5000);
                bash.execute(&helper_eval_cmd(), 2000);
                self.state = InitState::Ready;
                return;
            }
            _ => {}
        }

        // Poll in a tight loop until ready (with a timeout).
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        while !self.is_ready() && Instant::now() < deadline {
            self.poll_init(bash);
            if !self.is_ready() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }

        // If still not ready after timeout, force blocking init.
        if !self.is_ready() {
            bash.execute(SOURCE_CMD, 5000);
            bash.execute(&helper_eval_cmd(), 2000);
            self.state = InitState::Ready;
        }
    }

    /// Generate completions for the input line at the given cursor position.
    pub fn complete(
        &mut self,
        bash: &mut BashCoprocess,
        line: &str,
        cursor: usize,
    ) -> Option<CompletionResult> {
        if !self.is_ready() {
            self.finish_init_blocking(bash);
        }

        let before_cursor = &line[..cursor];

        // Find word start: scan backwards from cursor to whitespace.
        let word_start = before_cursor
            .rfind(char::is_whitespace)
            .map(|pos| pos + 1)
            .unwrap_or(0);

        // Call the bash helper. Shell-quote the line to protect special chars.
        let quoted_line = shell_quote(line);
        let cmd = format!("__slate_complete {} {}", quoted_line, cursor);
        let result = bash.execute(&cmd, 2000);

        if result.exit_code == -1 {
            return None;
        }

        let candidates: Vec<String> = result
            .output
            .lines()
            .map(|l| l.trim_end().to_string())
            .filter(|l| !l.is_empty())
            .collect();

        Some(CompletionResult {
            candidates,
            replace_start: word_start,
            replace_end: cursor,
        })
    }
}

impl Default for CompletionEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Shell-quote a string by wrapping in single quotes and escaping embedded quotes.
fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::bash_coprocess::BashCoprocess;
    use std::sync::Mutex;

    static PTY_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_shell_quote() {
        assert_eq!(shell_quote("hello"), "'hello'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn test_command_completion() {
        let _lock = PTY_LOCK.lock().unwrap();
        let mut bash = BashCoprocess::spawn(500, 24).expect("spawn");
        let mut engine = CompletionEngine::new();
        let result = engine.complete(&mut bash, "ech", 3).unwrap();
        assert!(
            result.candidates.iter().any(|c| c == "echo"),
            "should complete 'ech' to 'echo', got: {:?}",
            result.candidates
        );
        assert_eq!(result.replace_start, 0);
        assert_eq!(result.replace_end, 3);
    }

    #[test]
    fn test_file_completion() {
        let _lock = PTY_LOCK.lock().unwrap();
        let mut bash = BashCoprocess::spawn(500, 24).expect("spawn");
        let mut engine = CompletionEngine::new();
        // /tmp should exist and have files
        let result = engine.complete(&mut bash, "ls /tmp/", 8).unwrap();
        // We just check it returns something (the exact files vary)
        assert_eq!(result.replace_start, 3);
        assert_eq!(result.replace_end, 8);
    }

    #[test]
    fn test_no_matches() {
        let _lock = PTY_LOCK.lock().unwrap();
        let mut bash = BashCoprocess::spawn(500, 24).expect("spawn");
        let mut engine = CompletionEngine::new();
        let result = engine.complete(&mut bash, "xyzzy_no_such_cmd_999", 21).unwrap();
        assert!(
            result.candidates.is_empty(),
            "should have no matches, got: {:?}",
            result.candidates
        );
    }

    #[test]
    fn test_git_subcommand_completion() {
        let _lock = PTY_LOCK.lock().unwrap();
        let mut bash = BashCoprocess::spawn(500, 24).expect("spawn");
        let mut engine = CompletionEngine::new();
        // Complete "git sta" — should include status/stash via lazy-loaded completion
        let result = engine.complete(&mut bash, "git sta", 7).unwrap();
        assert!(
            result
                .candidates
                .iter()
                .any(|c| c.trim() == "status" || c.trim() == "stash"),
            "should complete 'git sta' to include 'status' or 'stash', got: {:?}",
            result.candidates
        );
        assert_eq!(result.replace_start, 4);
        assert_eq!(result.replace_end, 7);
    }

    #[test]
    fn test_completion_with_argument_context() {
        let _lock = PTY_LOCK.lock().unwrap();
        let mut bash = BashCoprocess::spawn(500, 24).expect("spawn");
        let mut engine = CompletionEngine::new();
        // Complete "ls /e" — should include /etc
        let result = engine.complete(&mut bash, "ls /e", 5).unwrap();
        assert!(
            result.candidates.iter().any(|c| c.contains("etc")),
            "should complete '/e' to include 'etc', got: {:?}",
            result.candidates
        );
    }
}
