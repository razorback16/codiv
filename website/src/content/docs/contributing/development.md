---
title: Development Setup
description: Prerequisites, building, testing, and debugging Codiv locally.
section: Contributing
order: 1
---

## Prerequisites

To work on Codiv, you need:

- **Rust toolchain** (stable) — install via [rustup](https://rustup.rs/)
- **Git** for version control
- A Unix-like OS (Linux or macOS) — Codiv uses Unix domain sockets and PTY

Optional:

- An LLM API key if you want to test AI features
- `cargo-watch` for auto-rebuilding during development

## Building

Build the entire workspace:

```bash
cargo build --workspace
```

For a release build with optimizations:

```bash
cargo build --workspace --release
```

Or use the Makefile shortcuts:

```bash
make build    # cargo build --workspace
make release  # cargo build --workspace --release
```

## Testing

Run all tests:

```bash
cargo test --workspace
```

Or via Make:

```bash
make test
```

## Linting

Check code style and common issues:

```bash
# Clippy lints
cargo clippy --workspace

# Format check
cargo fmt --check
```

Fix formatting automatically:

```bash
cargo fmt --all
```

## Debug Mode

Enable debug logging to trace execution:

```bash
# Default debug level
codiv --debug

# Trace-level (very verbose)
codiv --debug=trace
```

Logs are written to `/tmp/codiv-debug.log`. This is useful for debugging IPC issues, command classification, and agent behavior.

## Project Structure

```
codiv/
├── codiv/          # TUI client crate
│   └── src/
├── codivd/         # Daemon crate
│   └── src/
├── codiv-tools/    # Shared tools library
│   └── src/
├── codiv-common/   # Shared types
│   └── src/
├── Cargo.toml      # Workspace manifest
├── Makefile
└── website/        # This documentation site
```

Each crate has its own `Cargo.toml` and `src/` directory. The workspace root `Cargo.toml` defines the member crates.
