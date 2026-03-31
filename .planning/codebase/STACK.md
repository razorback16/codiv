# Technology Stack

**Analysis Date:** 2026-03-31

## Languages

**Primary:**
- Rust (edition 2021) - All application code across all four crates

**Secondary:**
- TOML - Configuration files (`~/.codiv/config.toml`, `crates/codivd/config.default.toml`)
- Bash - Install script (`install.sh`), Makefile build targets

## Runtime

**Environment:**
- Unix-only (macOS / Linux) — `nix` crate usage, Unix domain sockets, PTY management
- No browser, no WASM, no cross-platform Windows target

**Package Manager:**
- Cargo 1.94.0
- Lockfile: `Cargo.lock` present (committed)

## Workspace Layout

Four-crate Cargo workspace at `Cargo.toml`:

| Crate | Path | Role |
|---|---|---|
| `codiv` | `crates/codiv/` | TUI client binary |
| `codivd` | `crates/codivd/` | Daemon binary |
| `codiv-tools` | `crates/codiv-tools/` | Tool implementations (shared lib) |
| `codiv-common` | `crates/codiv-common/` | Shared types, IPC, config (shared lib) |

## Frameworks

**Terminal UI:**
- `ratatui` 0.30 — TUI rendering framework (`crates/codiv/`)
- `crossterm` 0.28 — Cross-platform terminal control (`crates/codiv/`)
- `tui-term` 0.3 — Terminal emulator widget for ratatui (`crates/codiv/`)
- `vt100` 0.16 — VT100 terminal emulator state (`crates/codiv/`)

**Async Runtime:**
- `tokio` 1 (features: `full`) — Async runtime in daemon (`crates/codivd/`)
- `tokio` 1 (features: `rt-multi-thread`, `macros`) — Used in codiv dev-dependencies

**AI/LLM:**
- `aisdk` 0.5 (forked: `github.com/razorback16/aisdk`, branch `main`) — Unified LLM interface, features: `anthropic`, `openai`, `google`, `openaicompatible`, `vllm` (`crates/codivd/`)

**Markdown:**
- `streamdown-parser` (forked: `github.com/razorback16/streamdown-rs`) — Streaming markdown parser (`crates/codiv/`)
- `streamdown-render` (forked: `github.com/razorback16/streamdown-rs`) — Markdown renderer (`crates/codiv/`)

**Bash Parsing:**
- `brush-parser` 0.3 — Bash AST parsing for risk classification (`crates/codivd/`)

**Testing:**
- `termwright` 0.2 — Terminal UI testing framework (`crates/codiv/` dev-dependencies)
- Standard Rust `#[test]` and `#[tokio::test]` for unit/integration tests

## Key Dependencies

**IPC / Serialization:**
- `serde` 1 + `serde_json` 1 — JSON and struct serialization throughout all crates
- `bincode` 1 — Binary serialization for IPC framing (all crates)
- `crossbeam-channel` 0.5 — Multi-producer multi-consumer channels (`codiv`, `codiv-common`, `codiv-tools`)

**PTY / Process:**
- `portable-pty` 0.9 — PTY (pseudo-terminal) management (`codiv`, `codiv-common`)
- `nix` 0.29 — Unix syscalls: signals, process, terminal, filesystem (`codiv`, `codivd`)
- `libc` 0.2 — Low-level C bindings (`codiv`, `codiv-common`)

**Storage:**
- `rusqlite` 0.31 (feature: `bundled`) — Embedded SQLite for session storage (`crates/codivd/`)

**Configuration:**
- `toml` 0.8 — Config file parsing (`crates/codivd/`)
- `toml_edit` 0.22 — Format-preserving config editing (`crates/codivd/`)
- `notify` 7 — Filesystem watcher for hot-reload of config (`crates/codivd/`)

**HTTP:**
- `reqwest` 0.12 (feature: `json`) — HTTP client used within `aisdk` for LLM API calls (`crates/codivd/`)

**Other Utilities:**
- `clap` 4 (feature: `derive`) — CLI argument parsing (`crates/codiv/`)
- `uuid` 1 (feature: `v4`) — Session ID generation (`crates/codivd/`)
- `schemars` 1 — JSON Schema generation for tool definitions (`crates/codivd/`, `crates/codiv-tools/`)
- `futures` 0.3 — Stream combinators (`crates/codivd/`)
- `tracing` 0.1 + `tracing-subscriber` 0.3 — Structured logging in daemon (`crates/codivd/`)
- `log` 0.4 + `env_logger` 0.11 — Logging in TUI client (`crates/codiv/`)
- `dirs` 6 — Platform home directory resolution (`crates/codiv/`)
- `rand` 0.8 — Random number generation (`codiv`, `codiv-common`)
- `similar` 2 — Diff computation for edit display (`crates/codiv/`)
- `human_format` 1 — Human-readable number formatting (`crates/codiv/`)
- `glob` 0.3 — File glob matching (`crates/codiv-tools/`)
- `grep-regex` 0.1 + `grep-searcher` 0.1 — Ripgrep-based file search (`crates/codiv-tools/`)
- `ignore` 0.4 — `.gitignore`-aware directory traversal (`crates/codiv-tools/`)

## Configuration

**Environment:**
- Config file: `~/.codiv/config.toml` (auto-created from embedded default on first run)
- Environment variable overrides: `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GOOGLE_API_KEY`
- Per-provider and per-role API keys configurable in TOML
- Config hot-reloads at runtime via `notify` filesystem watcher (no daemon restart needed)

**Build:**
- `Makefile` with targets: `build`, `test`, `release`, `debug`, `clean`
- `make build` → `cargo build --workspace`
- `make release` → `cargo build --workspace --release`
- `make test` → `cargo test --workspace`

**Runtime Files:**
- `~/.codiv/config.toml` — User configuration
- `~/.codiv/sessions.db` — SQLite session database
- `~/.codiv/codivd.pid` — Daemon PID file
- `~/.codiv/codivd.log` — Daemon log file
- `/tmp/codiv-debug.log` — Debug log (only with `--debug` flag)
- `/tmp/codivd-{uid}.sock` — Unix domain socket for IPC

## Platform Requirements

**Development:**
- Rust 1.94.0+ (edition 2021)
- macOS or Linux (Unix-only)
- `cargo` / `make`

**Production:**
- macOS (tested; LaunchAgent plist in `install.sh`)
- Linux (supported; systemd unit in `install.sh`)
- Self-contained binary distribution via GitHub Releases (`razorback16/codiv`)

---

*Stack analysis: 2026-03-31*
