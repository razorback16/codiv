# AST-Based Shell Command Risk Classifier

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the regex-based bash command risk classifier with a proper AST parser that handles pipes, chains, redirections, subshells, and sensitive paths.

**Architecture:** Use `brush-parser` to parse shell commands into an AST, then walk the tree to classify each command. The riskiest command in a pipeline/chain determines the overall risk. Sensitive target paths escalate risk. Unknown commands default to Medium.

**Tech Stack:** `brush-parser` 0.3 (POSIX/bash AST parser), existing `RiskLevel` enum from `slate-common`.

---

### Task 1: Add brush-parser dependency

**Files:**
- Modify: `crates/slated/Cargo.toml`

**Step 1: Add the dependency**

Add `brush-parser = "0.3"` to `[dependencies]` in `crates/slated/Cargo.toml`.

**Step 2: Verify it compiles**

Run: `cargo check -p slated`
Expected: compiles with no errors

**Step 3: Commit**

```bash
git add crates/slated/Cargo.toml Cargo.lock
git commit -m "chore: add brush-parser dependency for AST-based command classification"
```

---

### Task 2: Create the AST classifier module with command tier lookup

**Files:**
- Create: `crates/slated/src/agent/ast_classifier.rs`
- Test: inline `#[cfg(test)]` module

This is the core of the new classifier. It parses a shell command string into an AST using `brush-parser`, walks the tree, and returns the max `RiskLevel` across all commands in the pipeline/chain.

**Step 1: Write failing tests**

```rust
// crates/slated/src/agent/ast_classifier.rs

#[cfg(test)]
mod tests {
    use super::*;
    use slate_common::messages::RiskLevel;

    // --- Simple commands ---
    #[test]
    fn readonly_commands_are_low() {
        assert_eq!(classify_command("ls -la"), RiskLevel::Low);
        assert_eq!(classify_command("cat README.md"), RiskLevel::Low);
        assert_eq!(classify_command("git status"), RiskLevel::Low);
        assert_eq!(classify_command("git log --oneline"), RiskLevel::Low);
        assert_eq!(classify_command("git diff HEAD"), RiskLevel::Low);
        assert_eq!(classify_command("pwd"), RiskLevel::Low);
        assert_eq!(classify_command("echo hello"), RiskLevel::Low);
        assert_eq!(classify_command("head -5 file.txt"), RiskLevel::Low);
        assert_eq!(classify_command("tail -f log"), RiskLevel::Low);
        assert_eq!(classify_command("find . -name '*.rs'"), RiskLevel::Low);
        assert_eq!(classify_command("wc -l file.txt"), RiskLevel::Low);
        assert_eq!(classify_command("env"), RiskLevel::Low);
        assert_eq!(classify_command("which rustc"), RiskLevel::Low);
        assert_eq!(classify_command("tree src/"), RiskLevel::Low);
        assert_eq!(classify_command("file README.md"), RiskLevel::Low);
        assert_eq!(classify_command("stat file.txt"), RiskLevel::Low);
    }

    #[test]
    fn build_commands_are_low() {
        assert_eq!(classify_command("cargo build"), RiskLevel::Low);
        assert_eq!(classify_command("cargo test"), RiskLevel::Low);
        assert_eq!(classify_command("cargo check"), RiskLevel::Low);
        assert_eq!(classify_command("cargo clippy"), RiskLevel::Low);
        assert_eq!(classify_command("cargo fmt"), RiskLevel::Low);
        assert_eq!(classify_command("npm test"), RiskLevel::Low);
        assert_eq!(classify_command("npm run build"), RiskLevel::Low);
        assert_eq!(classify_command("make"), RiskLevel::Low);
        assert_eq!(classify_command("make test"), RiskLevel::Low);
    }

    #[test]
    fn unknown_commands_are_medium() {
        assert_eq!(classify_command("python script.py"), RiskLevel::Medium);
        assert_eq!(classify_command("docker ps"), RiskLevel::Medium);
        assert_eq!(classify_command("node server.js"), RiskLevel::Medium);
        assert_eq!(classify_command("./custom_script.sh"), RiskLevel::Medium);
    }

    #[test]
    fn write_commands_are_medium() {
        assert_eq!(classify_command("touch newfile.txt"), RiskLevel::Medium);
        assert_eq!(classify_command("mkdir -p src/new"), RiskLevel::Medium);
        assert_eq!(classify_command("cp file.txt backup.txt"), RiskLevel::Medium);
        assert_eq!(classify_command("tee output.txt"), RiskLevel::Medium);
    }

    #[test]
    fn dangerous_commands_are_high() {
        assert_eq!(classify_command("rm file.txt"), RiskLevel::High);
        assert_eq!(classify_command("git push origin feature"), RiskLevel::High);
        assert_eq!(classify_command("mv src/old.rs src/new.rs"), RiskLevel::High);
        assert_eq!(classify_command("npm install express"), RiskLevel::High);
        assert_eq!(classify_command("pip install requests"), RiskLevel::High);
        assert_eq!(classify_command("cargo install ripgrep"), RiskLevel::High);
        assert_eq!(classify_command("brew install jq"), RiskLevel::High);
        assert_eq!(classify_command("git reset HEAD~1"), RiskLevel::High);
        assert_eq!(classify_command("git clean -fd"), RiskLevel::High);
        assert_eq!(classify_command("wget https://example.com/file"), RiskLevel::High);
    }

    #[test]
    fn critical_commands() {
        assert_eq!(classify_command("rm -rf ./build/"), RiskLevel::Critical);
        assert_eq!(classify_command("rm -fr /tmp/test"), RiskLevel::Critical);
        assert_eq!(classify_command("sudo apt update"), RiskLevel::Critical);
        assert_eq!(classify_command("git push --force origin main"), RiskLevel::Critical);
        assert_eq!(classify_command("git push -f origin main"), RiskLevel::Critical);
        assert_eq!(classify_command("git reset --hard HEAD~3"), RiskLevel::Critical);
        assert_eq!(classify_command("dd if=/dev/zero of=/dev/sda"), RiskLevel::Critical);
        assert_eq!(classify_command("mkfs.ext4 /dev/sda1"), RiskLevel::Critical);
        assert_eq!(classify_command("chmod -R 777 /"), RiskLevel::Critical);
        assert_eq!(classify_command("chown -R root:root /"), RiskLevel::Critical);
    }

    // --- Pipelines: max risk wins ---
    #[test]
    fn pipeline_takes_max_risk() {
        // ls (Low) | grep (Low) → Low
        assert_eq!(classify_command("ls | grep foo"), RiskLevel::Low);
        // cat (Low) | rm (High) → High
        assert_eq!(classify_command("echo y | rm -i file.txt"), RiskLevel::High);
        // ls (Low) | sudo tee (Critical) → Critical
        assert_eq!(classify_command("echo hello | sudo tee /etc/config"), RiskLevel::Critical);
    }

    // --- Chains (&&, ||, ;): max risk wins ---
    #[test]
    fn chain_takes_max_risk() {
        // ls (Low) && rm (High) → High
        assert_eq!(classify_command("ls && rm file.txt"), RiskLevel::High);
        // echo (Low) ; sudo (Critical) → Critical
        assert_eq!(classify_command("echo done; sudo reboot"), RiskLevel::Critical);
        // cargo build (Low) || rm -rf (Critical) → Critical
        assert_eq!(classify_command("cargo build || rm -rf ./target"), RiskLevel::Critical);
    }

    // --- Subshells ---
    #[test]
    fn subshell_takes_max_risk() {
        assert_eq!(classify_command("(rm -rf /tmp/test)"), RiskLevel::Critical);
        assert_eq!(classify_command("(ls && echo done)"), RiskLevel::Low);
    }

    // --- Command substitution in args ---
    #[test]
    fn command_substitution_analyzed() {
        // The outer command is rm (High), inner is ls (Low) → High
        assert_eq!(classify_command("rm $(ls /tmp)"), RiskLevel::High);
    }

    // --- Sensitive paths escalate risk ---
    #[test]
    fn sensitive_path_escalation() {
        // rm targeting home dir → Critical
        assert_eq!(classify_command("rm -rf ~/"), RiskLevel::Critical);
        assert_eq!(classify_command("rm -rf $HOME"), RiskLevel::Critical);
        // rm targeting system paths → Critical
        assert_eq!(classify_command("rm -rf /etc"), RiskLevel::Critical);
        assert_eq!(classify_command("rm -rf /usr"), RiskLevel::Critical);
    }

    // --- Redirections ---
    #[test]
    fn output_redirect_to_sensitive_path() {
        // Writing to /etc is Critical
        assert_eq!(classify_command("echo foo > /etc/passwd"), RiskLevel::Critical);
        // Normal redirect doesn't escalate
        assert_eq!(classify_command("echo foo > output.txt"), RiskLevel::Low);
    }

    // --- Parse failures fall back to Medium ---
    #[test]
    fn unparseable_defaults_to_medium() {
        assert_eq!(classify_command(";;;"), RiskLevel::Medium);
        assert_eq!(classify_command(""), RiskLevel::Low);
    }

    // --- Readonly detection (used by Manual mode) ---
    #[test]
    fn readonly_detection() {
        assert!(is_readonly("ls -la"));
        assert!(is_readonly("cat file.txt"));
        assert!(is_readonly("git status"));
        assert!(is_readonly("git log --oneline"));
        assert!(is_readonly("git diff HEAD"));
        assert!(is_readonly("pwd"));
        assert!(is_readonly("echo hello"));
        assert!(is_readonly("find . -name '*.rs'"));
        assert!(is_readonly("head -5 file"));
        assert!(is_readonly("tail -f log"));
        assert!(is_readonly("env"));
        assert!(is_readonly("which rustc"));

        assert!(!is_readonly("rm file.txt"));
        assert!(!is_readonly("cargo build"));
        assert!(!is_readonly("npm install"));
        assert!(!is_readonly("git push origin main"));
        assert!(!is_readonly("python script.py"));
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p slated ast_classifier`
Expected: compilation error (module doesn't exist yet)

**Step 3: Implement the classifier**

The classifier has these layers:

1. **Parse** the command string into `brush_parser::ast::Program`
2. **Walk** the AST tree, extracting every `SimpleCommand`
3. **Classify** each simple command by looking up the binary name in tier tables
4. **Check flags** on known commands (e.g., `rm` with `-rf` → Critical, `rm` without → High)
5. **Check target paths** for sensitive locations (`/`, `~`, `/etc`, `/usr`, etc.)
6. **Check redirections** for writes to sensitive paths
7. **Return** the maximum risk across all commands in the tree

```rust
// crates/slated/src/agent/ast_classifier.rs

use std::collections::HashSet;
use std::io::Cursor;
use std::sync::LazyLock;

use slate_common::messages::RiskLevel;

/// Tier tables for command classification.
/// Commands not in any tier default to Medium.

/// Low-risk: purely read-only, no side effects.
static READONLY_COMMANDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        // filesystem reads
        "ls", "cat", "head", "tail", "less", "more", "file", "stat",
        "wc", "find", "tree", "du", "df", "realpath", "basename",
        "dirname", "readlink", "md5sum", "sha256sum", "shasum",
        "cksum", "strings", "od", "xxd", "hexdump",
        // text processing (read-only)
        "grep", "rg", "ag", "awk", "sed", "sort", "uniq", "cut",
        "tr", "diff", "comm", "join", "paste", "column", "fold",
        "fmt", "expand", "unexpand", "nl", "pr", "rev",
        // system info
        "pwd", "echo", "printf", "which", "where", "env", "printenv",
        "date", "uname", "hostname", "whoami", "id", "groups",
        "free", "uptime", "lsof", "ps", "top", "htop",
        // misc safe
        "true", "false", "test", "[", "seq", "yes", "cal", "bc",
        "expr", "factor", "numfmt", "base64", "jq", "yq",
        "bat", "fd", "fzf", "exa", "eza",
    ].into_iter().collect()
});

/// Low-risk: build/dev commands that compile or test but don't install globally.
static BUILD_COMMANDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    ["make", "cmake", "ninja", "gradle", "mvn", "ant"].into_iter().collect()
});

/// Subcommands that make a parent command low-risk.
/// Format: "parent" → set of safe subcommands.
static SAFE_SUBCOMMANDS: LazyLock<std::collections::HashMap<&'static str, HashSet<&'static str>>> = LazyLock::new(|| {
    let mut m = std::collections::HashMap::new();
    m.insert("git", ["status", "log", "diff", "branch", "show", "stash", "tag", "remote", "fetch", "blame", "shortlog", "describe", "ls-files", "ls-tree", "rev-parse", "config"].into_iter().collect());
    m.insert("cargo", ["build", "test", "check", "clippy", "fmt", "doc", "bench", "metadata", "tree", "verify-project"].into_iter().collect());
    m.insert("npm", ["test", "run", "start", "list", "ls", "outdated", "audit", "info", "view", "explain", "query"].into_iter().collect());
    m.insert("yarn", ["test", "run", "start", "list", "info", "why"].into_iter().collect());
    m.insert("pnpm", ["test", "run", "start", "list", "ls", "outdated", "audit", "why"].into_iter().collect());
    m.insert("bun", ["test", "run", "start"].into_iter().collect());
    m.insert("docker", ["ps", "images", "logs", "inspect", "stats", "top", "port", "version", "info", "history", "diff"].into_iter().collect());
    m.insert("kubectl", ["get", "describe", "logs", "top", "version", "config", "explain", "api-resources", "api-versions", "cluster-info"].into_iter().collect());
    m
});

/// High-risk commands (potentially dangerous but not catastrophic).
static HIGH_COMMANDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "rm", "mv", "wget", "curl",
    ].into_iter().collect()
});

/// High-risk subcommands for specific parent commands.
static HIGH_SUBCOMMANDS: LazyLock<std::collections::HashMap<&'static str, HashSet<&'static str>>> = LazyLock::new(|| {
    let mut m = std::collections::HashMap::new();
    m.insert("git", ["push", "reset", "clean", "checkout", "restore", "rebase", "merge", "cherry-pick", "revert", "stash"].into_iter().collect());
    m.insert("npm", ["install", "uninstall", "update", "link", "publish"].into_iter().collect());
    m.insert("yarn", ["add", "remove", "install", "upgrade", "publish"].into_iter().collect());
    m.insert("pip", ["install", "uninstall"].into_iter().collect());
    m.insert("cargo", ["install", "uninstall", "publish"].into_iter().collect());
    m.insert("brew", ["install", "uninstall", "upgrade", "remove", "cleanup"].into_iter().collect());
    m.insert("apt", ["install", "remove", "purge", "upgrade", "autoremove"].into_iter().collect());
    m.insert("docker", ["run", "exec", "rm", "rmi", "pull", "push", "build", "stop", "kill", "restart", "create", "network", "volume", "compose"].into_iter().collect());
    m.insert("kubectl", ["apply", "create", "delete", "edit", "patch", "replace", "rollout", "scale", "exec", "run", "expose", "set"].into_iter().collect());
    m.insert("cp", HashSet::new()); // cp by itself is high
    m
});

/// Critical: always-critical commands regardless of args.
static CRITICAL_COMMANDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "sudo", "su", "doas",
        "mkfs", "mkfs.ext4", "mkfs.xfs", "mkfs.btrfs",
        "dd", "fdisk", "parted", "gdisk",
        "shutdown", "reboot", "poweroff", "halt", "init",
        "iptables", "ip6tables", "nft",
        "systemctl", "service",
    ].into_iter().collect()
});

/// Sensitive path prefixes. Operations targeting these escalate to Critical.
static SENSITIVE_PATHS: &[&str] = &[
    "/", "/etc", "/usr", "/bin", "/sbin", "/boot", "/dev",
    "/sys", "/proc", "/var", "/lib", "/lib64", "/opt",
    "~/", "$HOME", "~",
    "/root",
    ".ssh", ".gnupg", ".aws", ".kube",
];

/// Classify a bash command string by parsing its AST.
pub fn classify_command(command: &str) -> RiskLevel {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return RiskLevel::Low;
    }

    let input = Cursor::new(trimmed);
    let source = brush_parser::SourceInfo::default();
    let opts = brush_parser::ParserOptions::default();
    let mut parser = brush_parser::Parser::new(input, &opts, &source);

    match parser.parse_program() {
        Ok(program) => classify_program(&program),
        Err(_) => RiskLevel::Medium, // unparseable → cautious default
    }
}

/// Check if a command is purely read-only (for Manual mode).
/// Returns true only if ALL commands in the pipeline/chain are readonly.
pub fn is_readonly(command: &str) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return true;
    }

    let input = Cursor::new(trimmed);
    let source = brush_parser::SourceInfo::default();
    let opts = brush_parser::ParserOptions::default();
    let mut parser = brush_parser::Parser::new(input, &opts, &source);

    match parser.parse_program() {
        Ok(program) => check_program_readonly(&program),
        Err(_) => false, // unparseable → not readonly
    }
}

// ---------- AST walkers ----------

fn classify_program(program: &brush_parser::ast::Program) -> RiskLevel {
    let mut max = RiskLevel::Low;
    for item in &program.0 {
        max = max_risk(max, classify_complete_command(item));
    }
    max
}

fn classify_complete_command(cc: &brush_parser::ast::CompleteCommand) -> RiskLevel {
    let mut max = RiskLevel::Low;
    for item in &cc.0 {
        max = max_risk(max, classify_compound_list_item(item));
    }
    max
}

fn classify_compound_list_item(item: &brush_parser::ast::CompoundListItem) -> RiskLevel {
    classify_and_or_list(&item.0)
}

fn classify_and_or_list(aol: &brush_parser::ast::AndOrList) -> RiskLevel {
    let mut max = classify_pipeline(&aol.first);
    for and_or in &aol.additional {
        max = max_risk(max, classify_pipeline(and_or.pipeline()));
    }
    max
}

fn classify_pipeline(pipeline: &brush_parser::ast::Pipeline) -> RiskLevel {
    let mut max = RiskLevel::Low;
    for cmd in &pipeline.seq {
        max = max_risk(max, classify_command_node(cmd));
    }
    max
}

fn classify_command_node(cmd: &brush_parser::ast::Command) -> RiskLevel {
    match cmd {
        brush_parser::ast::Command::Simple(simple) => classify_simple_command(simple),
        brush_parser::ast::Command::Compound(compound, _redirects) => {
            classify_compound_command(compound)
        }
        brush_parser::ast::Command::Function(func) => {
            // Function definitions: classify the body
            classify_compound_command(&func.body)
        }
        _ => RiskLevel::Medium,
    }
}

fn classify_compound_command(cmd: &brush_parser::ast::CompoundCommand) -> RiskLevel {
    match cmd {
        brush_parser::ast::CompoundCommand::BraceGroup(list)
        | brush_parser::ast::CompoundCommand::Subshell(list) => {
            let mut max = RiskLevel::Low;
            for item in &list.0 {
                max = max_risk(max, classify_compound_list_item(item));
            }
            max
        }
        brush_parser::ast::CompoundCommand::If(if_cmd) => {
            let mut max = RiskLevel::Low;
            for clause in &if_cmd.if_clauses {
                for item in &clause.condition.0 {
                    max = max_risk(max, classify_compound_list_item(item));
                }
                for item in &clause.body.0 {
                    max = max_risk(max, classify_compound_list_item(item));
                }
            }
            if let Some(ref else_body) = if_cmd.else_part {
                for item in &else_body.0 {
                    max = max_risk(max, classify_compound_list_item(item));
                }
            }
            max
        }
        brush_parser::ast::CompoundCommand::While(while_cmd) => {
            let mut max = RiskLevel::Low;
            for item in &while_cmd.condition.0 {
                max = max_risk(max, classify_compound_list_item(item));
            }
            for item in &while_cmd.body.0 {
                max = max_risk(max, classify_compound_list_item(item));
            }
            max
        }
        brush_parser::ast::CompoundCommand::Until(until_cmd) => {
            let mut max = RiskLevel::Low;
            for item in &until_cmd.condition.0 {
                max = max_risk(max, classify_compound_list_item(item));
            }
            for item in &until_cmd.body.0 {
                max = max_risk(max, classify_compound_list_item(item));
            }
            max
        }
        brush_parser::ast::CompoundCommand::For(for_cmd) => {
            let mut max = RiskLevel::Low;
            for item in &for_cmd.body.0 {
                max = max_risk(max, classify_compound_list_item(item));
            }
            max
        }
        brush_parser::ast::CompoundCommand::Case(case_cmd) => {
            let mut max = RiskLevel::Low;
            for item_ref in &case_cmd.cases {
                if let Some(ref body) = item_ref.body {
                    for item in &body.0 {
                        max = max_risk(max, classify_compound_list_item(item));
                    }
                }
            }
            max
        }
        _ => RiskLevel::Medium,
    }
}

fn classify_simple_command(cmd: &brush_parser::ast::SimpleCommand) -> RiskLevel {
    let words = extract_words(cmd);
    if words.is_empty() {
        // Assignment-only command (e.g., FOO=bar) — Low
        return RiskLevel::Low;
    }

    let binary = words[0].as_str();
    let args: Vec<&str> = words[1..].iter().map(|s| s.as_str()).collect();

    // 1. Critical commands
    if CRITICAL_COMMANDS.contains(binary) {
        return RiskLevel::Critical;
    }

    // 2. Check subcommand-based classification
    let subcommand = args.first().copied().unwrap_or("");

    // Critical flag patterns on specific commands
    let critical = match binary {
        "rm" => has_any_flag(&args, &["rf", "fr"]) || (has_flag(&args, 'r') && has_flag(&args, 'f')),
        "git" => matches!(subcommand,
            "push" if args.iter().any(|a| *a == "--force" || *a == "-f") => true,
            "reset" if args.iter().any(|a| *a == "--hard") => true,
            _ => false,
        ),
        "chmod" => has_flag(&args, 'R') && args.iter().any(|a| *a == "777" || a.starts_with("777")),
        "chown" => has_flag(&args, 'R'),
        _ => false,
    };
    if critical {
        return max_risk(RiskLevel::Critical, check_path_sensitivity(&args, binary));
    }

    // 3. High commands
    if HIGH_COMMANDS.contains(binary) {
        let base = match binary {
            "curl" => {
                // curl GET is medium, curl POST/PUT/DELETE/PATCH is high
                if args.iter().any(|a| ["-X", "--request"].contains(a)) {
                    RiskLevel::High
                } else if args.iter().any(|a| ["-d", "--data", "-F", "--form", "-T", "--upload-file"].contains(a)) {
                    RiskLevel::High
                } else {
                    RiskLevel::Medium
                }
            }
            _ => RiskLevel::High,
        };
        return max_risk(base, check_path_sensitivity(&args, binary));
    }

    // 4. Check subcommand tables
    if let Some(safe_subs) = SAFE_SUBCOMMANDS.get(binary) {
        if safe_subs.contains(subcommand) {
            return RiskLevel::Low;
        }
    }
    if let Some(high_subs) = HIGH_SUBCOMMANDS.get(binary) {
        if high_subs.is_empty() || high_subs.contains(subcommand) {
            return max_risk(RiskLevel::High, check_path_sensitivity(&args, binary));
        }
    }

    // 5. Readonly commands
    if READONLY_COMMANDS.contains(binary) {
        return RiskLevel::Low;
    }

    // 6. Build commands
    if BUILD_COMMANDS.contains(binary) {
        return RiskLevel::Low;
    }

    // 7. Check for SQL keywords in the command text
    let full_cmd = words.join(" ");
    if is_dangerous_sql(&full_cmd) {
        return RiskLevel::Critical;
    }

    // 8. Check redirections for sensitive writes
    let redir_risk = check_redirect_risk(cmd);

    // 9. Default: unknown → Medium
    max_risk(RiskLevel::Medium, redir_risk)
}

/// Extract word strings from a SimpleCommand (word_or_name + suffix words).
fn extract_words(cmd: &brush_parser::ast::SimpleCommand) -> Vec<String> {
    let mut words = Vec::new();

    if let Some(ref w) = cmd.word_or_name {
        words.push(w.value.clone());
    }

    if let Some(ref suffix) = cmd.suffix {
        for entry in &suffix.0 {
            match entry {
                brush_parser::ast::CommandSuffixItem::Word(w) => {
                    words.push(w.value.clone());
                }
                _ => {} // skip redirections here (handled separately)
            }
        }
    }

    words
}

/// Check redirections on a simple command for writes to sensitive paths.
fn check_redirect_risk(cmd: &brush_parser::ast::SimpleCommand) -> RiskLevel {
    let mut max = RiskLevel::Low;

    // Check prefix redirections
    if let Some(ref prefix) = cmd.prefix {
        for entry in &prefix.0 {
            if let brush_parser::ast::CommandPrefixItem::IoRedirect(redir) = entry {
                max = max_risk(max, classify_redirect(redir));
            }
        }
    }

    // Check suffix redirections
    if let Some(ref suffix) = cmd.suffix {
        for entry in &suffix.0 {
            if let brush_parser::ast::CommandSuffixItem::IoRedirect(redir) = entry {
                max = max_risk(max, classify_redirect(redir));
            }
        }
    }

    max
}

fn classify_redirect(redir: &brush_parser::ast::IoRedirect) -> RiskLevel {
    // Only output redirections (>, >>) to sensitive paths are risky
    match redir {
        brush_parser::ast::IoRedirect::File(_, op, target) => {
            let is_output = matches!(op,
                brush_parser::ast::IoFileRedirectKind::Write
                | brush_parser::ast::IoFileRedirectKind::Append
                | brush_parser::ast::IoFileRedirectKind::Clobber
            );
            if is_output {
                let path = &target.value;
                if is_sensitive_path(path) {
                    return RiskLevel::Critical;
                }
            }
            RiskLevel::Low
        }
        _ => RiskLevel::Low,
    }
}

fn is_sensitive_path(path: &str) -> bool {
    for sensitive in SENSITIVE_PATHS {
        if path == *sensitive || path.starts_with(&format!("{}/", sensitive)) {
            return true;
        }
    }
    // Also check for writing to /dev/
    if path.starts_with("/dev/") {
        return true;
    }
    false
}

fn check_path_sensitivity(args: &[&str], binary: &str) -> RiskLevel {
    // Only escalate for destructive commands
    let is_destructive = matches!(binary, "rm" | "chmod" | "chown" | "mv");
    if !is_destructive {
        return RiskLevel::Low;
    }
    for arg in args {
        if arg.starts_with('-') { continue; } // skip flags
        if is_sensitive_path(arg) {
            return RiskLevel::Critical;
        }
    }
    RiskLevel::Low
}

fn has_flag(args: &[&str], flag: char) -> bool {
    args.iter().any(|a| {
        a.starts_with('-') && !a.starts_with("--") && a.contains(flag)
    })
}

fn has_any_flag(args: &[&str], combos: &[&str]) -> bool {
    args.iter().any(|a| {
        if !a.starts_with('-') || a.starts_with("--") { return false; }
        let flags: String = a.chars().skip(1).collect();
        combos.iter().any(|combo| {
            combo.chars().all(|c| flags.contains(c))
        })
    })
}

fn is_dangerous_sql(text: &str) -> bool {
    let upper = text.to_uppercase();
    upper.contains("DROP TABLE") || upper.contains("DROP DATABASE")
        || upper.contains("TRUNCATE ")
}

fn max_risk(a: RiskLevel, b: RiskLevel) -> RiskLevel {
    let rank = |r: RiskLevel| match r {
        RiskLevel::Low => 0,
        RiskLevel::Medium => 1,
        RiskLevel::High => 2,
        RiskLevel::Critical => 3,
    };
    if rank(a) >= rank(b) { a } else { b }
}

// --- Readonly check (walks AST, returns true only if all commands are readonly) ---

fn check_program_readonly(program: &brush_parser::ast::Program) -> bool {
    program.0.iter().all(|cc| check_complete_command_readonly(cc))
}

fn check_complete_command_readonly(cc: &brush_parser::ast::CompleteCommand) -> bool {
    cc.0.iter().all(|item| check_and_or_list_readonly(&item.0))
}

fn check_and_or_list_readonly(aol: &brush_parser::ast::AndOrList) -> bool {
    check_pipeline_readonly(&aol.first)
        && aol.additional.iter().all(|ao| check_pipeline_readonly(ao.pipeline()))
}

fn check_pipeline_readonly(pipeline: &brush_parser::ast::Pipeline) -> bool {
    pipeline.seq.iter().all(|cmd| check_command_readonly(cmd))
}

fn check_command_readonly(cmd: &brush_parser::ast::Command) -> bool {
    match cmd {
        brush_parser::ast::Command::Simple(simple) => {
            let words = extract_words(simple);
            if words.is_empty() { return true; }
            READONLY_COMMANDS.contains(words[0].as_str())
        }
        brush_parser::ast::Command::Compound(compound, _) => {
            check_compound_readonly(compound)
        }
        _ => false,
    }
}

fn check_compound_readonly(cmd: &brush_parser::ast::CompoundCommand) -> bool {
    match cmd {
        brush_parser::ast::CompoundCommand::BraceGroup(list)
        | brush_parser::ast::CompoundCommand::Subshell(list) => {
            list.0.iter().all(|item| check_and_or_list_readonly(&item.0))
        }
        _ => false,
    }
}
```

**Important implementation notes:**
- The `extract_words` function pulls word values from the `SimpleCommand`'s `word_or_name` and `suffix` fields. These are `brush_parser::ast::Word` structs with a `.value: String` field.
- `brush_parser::ast::CommandSuffixItem` has `Word(Word)` and `IoRedirect(IoRedirect)` variants.
- `brush_parser::ast::CommandPrefixItem` has `AssignmentWord(...)` and `IoRedirect(IoRedirect)` variants.
- `brush_parser::ast::IoRedirect` has `File(Option<u32>, IoFileRedirectKind, Word)` and `HereDocument(...)` variants.
- `brush_parser::ast::IoFileRedirectKind` has `Write`, `Append`, `Clobber`, `Read`, `ReadAndWrite`, etc.
- The `AndOr` enum's `.pipeline()` method returns a `&Pipeline`.
- The AST types may differ slightly from what's documented — the implementer MUST check the actual `brush_parser` source types at build time. Use `cargo doc -p brush-parser --open` if the code doesn't compile as-is, and adjust enum variant names and struct fields accordingly.

**Step 4: Run tests**

Run: `cargo test -p slated ast_classifier -- --nocapture`
Expected: all tests pass

**Step 5: Commit**

```bash
git add crates/slated/src/agent/ast_classifier.rs
git commit -m "feat: add AST-based shell command risk classifier using brush-parser"
```

---

### Task 3: Register the module and wire it into the existing classifier

**Files:**
- Modify: `crates/slated/src/agent/mod.rs` — add `pub mod ast_classifier;`
- Modify: `crates/slated/src/agent/risk_classifier.rs` — delegate to `ast_classifier`
- Modify: `crates/slated/src/agent/permission_evaluator.rs` — use `ast_classifier::is_readonly` instead of `risk_classifier::is_readonly_bash`

**Step 1: Add module declaration**

In `crates/slated/src/agent/mod.rs`, add:
```rust
pub mod ast_classifier;
```

**Step 2: Update risk_classifier.rs to delegate to AST classifier**

Replace the `classify_bash_risk` function body and `is_readonly_bash` to delegate:

```rust
fn classify_bash_risk(args: &serde_json::Value) -> RiskLevel {
    let command = args.get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    super::ast_classifier::classify_command(command)
}

pub fn is_readonly_bash(command: &str) -> bool {
    super::ast_classifier::is_readonly(command)
}
```

Remove the old `CRITICAL_PATTERNS`, `HIGH_PATTERNS`, `LOW_PATTERNS`, and `READONLY_PATTERN` statics. Keep the `classify_risk` function as the public entry point — it still dispatches by tool name, just `classify_bash_risk` now uses the AST.

**Step 3: Run all existing tests**

Run: `cargo test -p slated`
Expected: all tests pass (existing `risk_classifier` tests + new `ast_classifier` tests)

**Step 4: Commit**

```bash
git add crates/slated/src/agent/mod.rs crates/slated/src/agent/risk_classifier.rs crates/slated/src/agent/permission_evaluator.rs
git commit -m "refactor: wire AST classifier into risk classification pipeline"
```

---

### Task 4: Clean up old regex patterns and verify full build

**Files:**
- Modify: `crates/slated/src/agent/risk_classifier.rs` — remove unused regex import and old statics
- Verify: `crates/slated/Cargo.toml` — regex dep can be removed if no longer used elsewhere

**Step 1: Remove regex import from risk_classifier.rs if no longer needed**

Check if `regex` is used elsewhere in slated. If only in `risk_classifier.rs` (old patterns), remove the `regex = "1"` dep from `Cargo.toml`.

**Step 2: Run full build and clippy**

Run: `cargo build -p slated && cargo clippy -p slated -- -W clippy::all`
Expected: clean build, no warnings

**Step 3: Run full test suite**

Run: `cargo test`
Expected: all tests pass across all crates

**Step 4: Commit**

```bash
git add -A
git commit -m "chore: clean up old regex patterns, replaced by AST classifier"
```

---

## Notes for implementer

- The `brush_parser` AST types are complex. If a variant name doesn't match (e.g., `IoFileRedirectKind::Write` vs `OutputRedirect`), use `cargo doc -p brush-parser --open` and check the actual enum. The code in Task 2 is based on the v0.3.0 API but minor adjustments may be needed.
- The `classify_redirect` function needs to match the actual redirect enum variants in brush-parser. The key distinction is output redirections (`>`, `>>`, `>|`) vs input (`<`).
- The `AndOr` type in brush-parser may use `.pipeline()` or have a direct field — check the actual struct definition.
- Performance: `brush-parser` creates a full AST on every tool call. This is fine for our use case (one parse per tool invocation, typically sub-millisecond for single commands). No caching needed.
