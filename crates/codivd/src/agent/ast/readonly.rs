use brush_parser::ast::{
    AndOr, Command, CommandPrefixOrSuffixItem, CompoundCommand, Pipeline, Program, SimpleCommand,
};

use super::classify::parse_command;
use super::risk_tables::{READONLY_COMMANDS, READONLY_SUBCOMMANDS};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Check whether a bash command string consists entirely of read-only operations.
pub(crate) fn is_readonly(command: &str) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return true;
    }

    match parse_command(trimmed) {
        Some(program) => program_is_readonly(&program),
        None => false,
    }
}

// ---------------------------------------------------------------------------
// AST walking -- readonly detection
// ---------------------------------------------------------------------------

fn program_is_readonly(program: &Program) -> bool {
    for cc in &program.complete_commands {
        for item in &cc.0 {
            if !and_or_list_is_readonly(&item.0) {
                return false;
            }
        }
    }
    true
}

fn and_or_list_is_readonly(and_or_list: &brush_parser::ast::AndOrList) -> bool {
    if !pipeline_is_readonly(&and_or_list.first) {
        return false;
    }
    for additional in &and_or_list.additional {
        let pipeline = match additional {
            AndOr::And(p) | AndOr::Or(p) => p,
        };
        if !pipeline_is_readonly(pipeline) {
            return false;
        }
    }
    true
}

fn pipeline_is_readonly(pipeline: &Pipeline) -> bool {
    pipeline.seq.iter().all(ast_command_is_readonly)
}

fn ast_command_is_readonly(cmd: &Command) -> bool {
    match cmd {
        Command::Simple(simple) => simple_command_is_readonly(simple),
        Command::Compound(compound, _) => compound_command_is_readonly(compound),
        Command::Function(_) => false,
        Command::ExtendedTest(_) => true,
    }
}

fn compound_command_is_readonly(compound: &CompoundCommand) -> bool {
    match compound {
        CompoundCommand::Subshell(sub) => compound_list_is_readonly(&sub.list),
        CompoundCommand::BraceGroup(bg) => compound_list_is_readonly(&bg.list),
        _ => false,
    }
}

fn compound_list_is_readonly(list: &brush_parser::ast::CompoundList) -> bool {
    list.0.iter().all(|item| and_or_list_is_readonly(&item.0))
}

fn simple_command_is_readonly(simple: &SimpleCommand) -> bool {
    let cmd_name = match &simple.word_or_name {
        Some(w) => w.value.as_str(),
        None => return true, // pure assignment
    };

    let base_cmd = cmd_name.rsplit('/').next().unwrap_or(cmd_name);

    if READONLY_COMMANDS.contains(base_cmd) {
        return true;
    }

    // `env` with no inner command (bare env or env VAR=val) is readonly;
    // `env inner_cmd` is readonly only if inner_cmd is readonly.
    if base_cmd == "env" {
        let args: Vec<String> = simple
            .suffix
            .as_ref()
            .map(|s| {
                s.0.iter()
                    .filter_map(|item| {
                        if let CommandPrefixOrSuffixItem::Word(w) = item {
                            Some(w.value.clone())
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Skip flags and VAR=val to find inner command
        for arg in &args {
            if arg.starts_with('-') || arg.contains('=') {
                continue;
            }
            // Found inner command -- check if it's readonly
            let inner_base = arg.rsplit('/').next().unwrap_or(arg);
            return READONLY_COMMANDS.contains(inner_base);
        }
        return true; // bare env or env VAR=val
    }

    // Check readonly subcommands (subset of safe subcommands that have no side effects)
    if let Some(suffix) = &simple.suffix {
        for item in &suffix.0 {
            if let CommandPrefixOrSuffixItem::Word(w) = item {
                if let Some(ro_set) = READONLY_SUBCOMMANDS.get(base_cmd) {
                    if ro_set.contains(w.value.as_str()) {
                        return true;
                    }
                }
                break; // only check first word arg
            }
        }
    }

    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(is_readonly("which rustc"));
        assert!(is_readonly("ps aux"));
        assert!(is_readonly("ping -c 1 host"));
        assert!(is_readonly("docker ps"));
        assert!(is_readonly("kubectl get pods"));

        assert!(!is_readonly("rm file.txt"));
        assert!(!is_readonly("cargo build"));
        assert!(!is_readonly("npm install"));
        assert!(!is_readonly("git push origin main"));
        assert!(!is_readonly("python script.py"));
        assert!(!is_readonly("env rm file.txt"));
        assert!(!is_readonly("eval echo hi"));
        assert!(!is_readonly("xargs echo"));
        assert!(!is_readonly("yes"));
    }
}
