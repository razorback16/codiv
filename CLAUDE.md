# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Slate Agent is a terminal-based AI coding assistant with two components:
- **slate** — Rust TUI client (ratatui + crossterm) that provides the interactive shell
- **slated** — C++ daemon that executes agent-initiated bash commands via worker processes

## Build & Test Commands

```bash
# Build everything (slated first, then slate — slate depends on slated for flatc)
make all

# Build individually
make slated          # C++ daemon: cmake + cmake --build in slated/build/
make slate           # Rust client: cargo build in slate/

# Run all tests
make test

# Run Rust tests only (most active test suite)
cd slate && cargo test

# Run a single Rust test
cd slate && cargo test test_execute_echo_hello

# Run C++ tests only
cd slated/build && ctest --output-on-failure

# Clean everything
make clean
```

## Architecture

### Two-Process Model

User commands go through slate's **persistent bash co-process** (PTY-based, maintains shell state like cd/aliases/env). Agent tool calls go through slated's **worker processes** (fresh bash per command, initialized from an env snapshot sent by the client).

### IPC: Unix Domain Socket + FlatBuffers

Schema at `schemas/ipc.fbs`. Wire format is 4-byte big-endian length prefix + FlatBuffer payload. The Rust `build.rs` generates FlatBuffers code and looks for `flatc` in `slated/build/_deps/flatbuffers-build/` first, then PATH — this is why slated must build before slate.

### Slate (Rust Client) Key Modules

- `shell/bash_coprocess.rs` — Core shell integration. Uses a **sentinel protocol**: wraps commands with unique markers to delimit output and capture exit codes. Supports blocking `execute()` and non-blocking `start_command()`/`try_read()`/`check_complete()`. All unit tests live here (~240 lines of tests).
- `shell/command_index.rs` — Scans PATH + builtins to classify user input as Execute/Interactive/AiQuery/NotFound/Exit/Empty.
- `shell/interactive.rs` — Raw TTY passthrough for programs like vim, ssh, htop.
- `ui/terminal.rs` — Main event loop and ratatui rendering. Three-zone layout: status bar, scrollable VT100 output canvas, input line.
- `ipc/client.rs` — Unix socket client with threaded reader and mpsc channel.
- `ipc/daemon_launcher.rs` — Finds/launches slated, resolves socket path (`~/.slate-agent/sock`).

### Slated (C++ Daemon) Key Modules

- `main.cpp` — Daemonization via fork, redirects stdio, signal handlers.
- `daemon.cpp` — Poll loop (50ms): accepts connections, polls workers for output, reaps finished workers, cleans stale sessions.
- `worker_process.cpp` — `spawn_worker()`: fork + pipe + exec pattern. Clears env, injects snapshot vars, execs `/bin/bash -c <command>`.
- `ipc/socket_server.cpp` — Non-blocking Unix domain socket accept/read/write.

## Testing Notes

- Rust PTY tests use a sequential execution mutex (`PTY_TEST_LOCK`) to prevent concurrent `forkpty()` contention. Tests are `#[ignore]`-gated and run with `cargo test -- --ignored` or just `cargo test` (the test harness handles it).
- C++ uses GoogleTest (fetched by CMake via FetchContent). Test directory is `slated/tests/`.

## FlatBuffers Schema Changes

When modifying `schemas/ipc.fbs`, both sides need rebuilding. The Rust side auto-regenerates via `build.rs`. The C++ side regenerates via CMake's `fbs_headers` target. Always `make clean && make all` after schema changes.
