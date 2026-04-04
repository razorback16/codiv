---
title: Code Structure
description: Cargo workspace layout, crate responsibilities, and how to extend Codiv.
section: Contributing
order: 2
---

## Workspace Layout

Codiv is a Cargo workspace with four crates:

```
codiv/
├── Cargo.toml          # Workspace root
├── codiv/              # TUI client
├── codivd/             # Async daemon
├── codiv-tools/        # Tool implementations
└── codiv-common/       # Shared types
```

## Crate Responsibilities

### codiv (TUI Client)

The terminal interface. Key modules:

- **main** — entry point, argument parsing, daemon connection
- **tui** — ratatui application loop, rendering, event handling
- **input** — command classification (Execute, Interactive, AiQuery, etc.)
- **command_index** — PATH scanning and hash map for O(1) command lookup
- **completion** — tab completion (programmable, command, file)
- **bash** — persistent bash co-process management
- **pty** — pseudo-terminal handling for interactive passthrough
- **cli/login** — `codiv login` interactive auth wizard
- **cli/migrate** — `codiv migrate-env` environment variable migration
- **ui/tool_presenters** — extracted tool-specific rendering logic for conversation blocks

### codivd (Daemon)

The AI backend. Key modules:

- **main** — daemon startup, Unix socket listener
- **agent** — AI agent implementation, tool calling loop
- **session** — session management, context timeline
- **streaming** — LLM response streaming to client
- **worker** — bash worker processes for agent tool execution
- **daemon_shell** — `DaemonShell` implementation for independent agent shells (pipe-based `bash -i`)
- **agent/relay_manager** — `RelayManager` lease-based state machine: manages shell lease acquisition, queuing, timeout, and cancellation for orchestrator commands routed through the client's co-process
- **handlers/** — decomposed daemon dispatch handlers:
  - **handlers/agent** — agent lifecycle (start, complete, error)
  - **handlers/compaction** — conversation compaction requests and completion
  - **handlers/permission** — permission confirmation flow
  - **handlers/session** — session creation and management
  - **handlers/shell** — shell lease protocol handling
- **config** — configuration loading and hot-reload

### codiv-tools (Shared Library)

Tool implementations used by both client and daemon:

- **bash** — shell command execution
- **read** — file reading with line numbers
- **write** — file writing
- **edit** — find-and-replace editing
- **glob** — file pattern matching
- **grep** — regex search

Each tool module exports a struct implementing a common tool trait with `execute()`, `help()`, and `agent_guide()` methods.

### codiv-common (Shared Types)

Types shared between client and daemon:

- **ipc** — message types (AgentRequest, StreamChunk, shell lease messages, etc.)
- **config** — configuration structs
- **types** — common enums and utility types
- **shell** — shell abstraction layer (ShellIO trait, ShellSession generic over IO, PtyIO for co-process, PipeIO for daemon shells)
- **auth/** — authentication system types and flows:
  - **auth/types** — provider credentials, token types, auth config structs
  - **auth/flows** — OAuth code flow and API key entry logic
  - **auth/storage** — credential persistence in config.toml
  - **auth/provider_registry** — built-in provider definitions (Anthropic, OpenAI, Claude Code OAuth, Codex OAuth)

## How to Add a New Tool

1. Create a new module in `codiv-tools/src/` (e.g., `my_tool.rs`)
2. Implement the tool trait with `execute()`, `help()`, and `agent_guide()`
3. Register the tool in `codiv-tools/src/lib.rs`
4. Add CLI subcommand in `codiv/src/main.rs`
5. Register the tool with the agent in `codivd/src/agent.rs`
6. Add tests in the tool module

## How to Modify IPC Messages

1. Edit the message types in `codiv-common/src/ipc.rs`
2. Update the client handler in `codiv/src/tui/` to handle the new/changed message
3. Update the daemon sender in `codivd/src/` to produce the new/changed message
4. Both crates depend on `codiv-common`, so the compiler enforces that all message variants are handled
