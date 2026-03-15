# AST Tool Design

**Date:** 2026-03-09
**Status:** Approved

## Overview

Add a native `ast` tool to Codiv that provides structural code search, inspection, linting, and rewriting using the `ast-grep-core` Rust library. Exposed as a single tool with four modes.

## Modes

| Mode | Risk Level | Description |
|------|-----------|-------------|
| `search` | Low | Find code matching a structural AST pattern |
| `inspect` | Low | Dump the AST for a file |
| `lint` | Low | Run YAML rules to detect anti-patterns |
| `rewrite` | Medium | Apply structural replacements to files |

## Input Schema

```rust
struct AstInput {
    /// The operation mode: "search", "inspect", "lint", "rewrite"
    mode: AstMode,

    /// Structural pattern (required for search/rewrite, e.g. "console.log($$$)")
    pattern: Option<String>,

    /// Replacement pattern (required for rewrite, e.g. "logger.info($$$)")
    replacement: Option<String>,

    /// Language (auto-detected from file extension if omitted)
    language: Option<String>,

    /// File or directory to operate on (defaults to cwd)
    path: Option<String>,

    /// Glob filter for files (e.g. "*.ts")
    include: Option<String>,

    /// YAML rule string (for lint mode)
    rule: Option<String>,

    /// Maximum matches to return (default 500)
    max_matches: Option<i64>,
}
```

### Validation Per Mode

- `search` — requires `pattern`
- `inspect` — requires `path` (to a file)
- `lint` — requires `rule`
- `rewrite` — requires `pattern` + `replacement`

### Output Format

- `search` → `file:line:matched_code` (consistent with grep tool)
- `inspect` → AST dump (S-expression or structured representation)
- `lint` → diagnostic messages with file, line, rule ID, message
- `rewrite` → list of files modified + diff summary

## Permission Integration

Mode-aware risk classification in `risk_classifier.rs`:

```rust
"ast" => classify_ast_risk(args),

fn classify_ast_risk(args: &serde_json::Value) -> RiskLevel {
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("");
    match mode {
        "search" | "inspect" | "lint" => RiskLevel::Low,
        "rewrite" => RiskLevel::Medium,
        _ => RiskLevel::Medium,
    }
}
```

## Dependencies

Added to `crates/codiv-tools/Cargo.toml`:

- `ast-grep-core` — pattern matching, rewriting, AST inspection
- `ast-grep-language` — language registry with all supported tree-sitter grammars

## Agent Guide

```
Structural code search and transformation using AST patterns. Use mode=search
for language-aware code search (e.g. pattern='console.log($$$)' finds all
console.log calls regardless of arguments). Use mode=inspect to view the AST
of a file. Use mode=lint with a YAML rule for anti-pattern detection. Use
mode=rewrite with pattern + replacement for structural refactoring. Language
is auto-detected from file extension. Prefer ast search over grep when you
need language-aware matching.
```

## Files Changed

| File | Change |
|------|--------|
| `crates/codiv-tools/Cargo.toml` | Add `ast-grep-core`, `ast-grep-language` |
| `crates/codiv-tools/src/tools/mod.rs` | Add `pub mod ast;` |
| `crates/codiv-tools/src/tools/ast.rs` | New — `AstInput` schema + `execute()` |
| `crates/codiv-tools/src/agent_guide.rs` | Add `"ast"` entry, update `TOOL_NAMES` |
| `crates/codivd/src/agent/tools.rs` | Register `ast` tool with permissions |
| `crates/codivd/src/agent/risk_classifier.rs` | Add `"ast"` classification |
| `crates/codiv/src/cli/tools.rs` | Add `Ast` CLI subcommand + dispatch |
