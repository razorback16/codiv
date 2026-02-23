## One-pager: C++ Terminal Coding Agent (Claude Code–style) using ai-sdk-cpp (ClickHouse), FTXUI, FlatBuffers

**Version**: 1.0 **Date**: 2026-02-10 **Author**: Subhagato **Status**: Draft

### Goal

A **terminal-native** coding agent for **bash/zsh** that can:

- **Fast-pass** real shell commands with near-zero latency
- Interpret **natural language** tasks (debug, refactor, implement, explain) and execute them end-to-end
- See command outputs, edit files, run tests, and keep bounded long-term + project memory

Powered by **ClickHouse ai-sdk-cpp** for model access (streaming + tool calling).

---

### UX

- `slate` owns the terminal experience — looks and behaves like a normal shell
- **FTXUI** owns the entire terminal: input, output rendering, layout, and colors
- **Three-zone display layout** (no outer window border — terminal edge is the border):
  1. **Status Bar** (top, 1 line): project path, git branch, active agent count, daemon status
  2. **Main Canvas** (middle, scrollable, flex): user command output inline (no border), inline task DAG tree with live status (● running, ◐ in review, ○ pending, ✓ done), color-bordered agent output blocks per active Work Item (auto-collapse on done, Enter to expand)
  3. **Input Bar** (bottom, sticky, grows upward): `Input()` + `CatchEvent()` for history, tab completion, Ctrl+R search, multi-line, inline completion ghosts

- Input auto-detection:

  1. **Command mode**: recognized command → execute immediately via persistent bash co-process
  2. **Agent mode**: otherwise → plan + execute Work Items, stream progress, show diffs/results
  3. **Force AI**: `?` prefix → always route to agent mode

---

### Architecture

**`slate` binary + `slated` daemon**

- **`slate` binary** (per-terminal): FTXUI terminal UI (three-zone layout: status bar, main canvas, input bar), persistent bash co-process (`bash --noediting -i` via pty pair), command index classifier, agent output block rendering, task DAG tree display, markdown/syntax rendering, signal forwarding
- **`slated` daemon** (singleton): agent system, orchestration, memory, task graph, scheduling, **worker bash sessions** (fresh bash process per Work Item, initialized from env snapshot, killed on completion — enables true parallel agent command execution)
- **Env snapshot protocol**: `slate` captures env snapshot (env vars, PATH, aliases, functions, cwd) on connect; refreshed on `cd`/`source`/manual `slate sync-env`. Worker bash sessions in `slated` initialized from this snapshot.
- **Command execution routing**:
  - **User commands** → `slate`'s persistent bash co-process (interactive, stateful)
  - **Agent Bash tool calls** → `slated`'s worker bash sessions (parallel, env-snapshot-initialized)
- **Bidirectional IPC**: FlatBuffers over Unix domain socket (4-byte length prefix + FlatBuffer payload). Carries env snapshots (slate→slated), streaming worker output + confirmations (slated→slate).

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           Slate Agent                                   │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  ┌────────────────────────────────────────────────────────────────────┐ │
│  │ slate binary (per-terminal)                                        │ │
│  │                                                                    │ │
│  │  ┌──────────────┐    ┌──────────────────┐    ┌─────────────────┐   │ │
│  │  │   FTXUI      │    │ Persistent Bash  │    │ Command Index   │   │ │
│  │  │ (terminal UI,│    │  Co-Process      │    │ (Fast-Pass)     │   │ │
│  │  │  3-zone      │───▶│ (bash --noediting│◀──▶│ PATH + builtins │   │ │
│  │  │  layout,     │    │  -i via pty)     │    │ + aliases       │   │ │
│  │  │  input,      │    │                  │    │                 │   │ │
│  │  │  rendering)  │    │  [user commands] │    │                 │   │ │
│  │  └──────────────┘    └──────────────────┘    └─────────────────┘   │ │
│  │         │                     ▲                                    │ │
│  │         │                     │ env snapshots (slate→slated)       │ │
│  │         │                     │ streaming output (slated→slate)    │ │
│  │         ▼                     │                                    │ │
│  │  ┌──────────────────────────────────────────────────────────────┐  │ │
│  │  │ FlatBuffers IPC (Unix domain socket)                         │  │ │
│  │  │ 4-byte length prefix + FlatBuffer payload                    │  │ │
│  │  └──────────────────────────────────────────────────────────────┘  │ │
│  └────────────────────────────────────────────────────────────────────┘ │
│                              │                                          │
│                              ▼                                          │
│  ┌────────────────────────────────────────────────────────────────────┐ │
│  │ slated daemon (singleton)                                          │ │
│  │                                                                    │ │
│  │  ┌──────────────────────────────────────────────────────────────┐  │ │
│  │  │                       Agent System                           │  │ │
│  │  │                                                              │  │ │
│  │  │  ┌────────────────────────────────────────────────────────┐  │  │ │
│  │  │  │ Orchestrator                                           │  │  │ │
│  │  │  │ - Owns session + user preferences                      │  │  │ │
│  │  │  │ - Project auto-switching (cwd/repo fingerprint)        │  │  │ │
│  │  │  │ - Delegates to executor roles via Work Items           │  │  │ │
│  │  │  └─────────────────────────┬──────────────────────────────┘  │  │ │
│  │  │                            │ delegates                       │  │ │
│  │  │  ┌─────────────────────────▼──────────────────────────────┐  │  │ │
│  │  │  │ Executor Roles                                         │  │  │ │
│  │  │  │ - Primary: TeamLead, Engineer, Reviewer                │  │  │ │
│  │  │  │ - Optional: Researcher, Security, Perf                 │  │  │ │
│  │  │  │ - TeamLead decomposes → Work Item DAG                  │  │  │ │
│  │  │  │ - Engineer executes → produces artifacts               │  │  │ │
│  │  │  │ - Reviewer gates → approve or create fix items         │  │  │ │
│  │  │  └────────────────────────────────────────────────────────┘  │  │ │
│  │  │                                                              │  │ │
│  │  │  ┌────────────────────────────────────────────────────────┐  │  │ │
│  │  │  │ Narrator / Memory Agent                                │  │  │ │
│  │  │  │ - Compresses + curates memory                          │  │  │ │
│  │  │  │ - Global memory (user habits) + Project memories       │  │  │ │
│  │  │  │ - Produces state snapshots for Orchestrator            │  │  │ │
│  │  │  └────────────────────────────────────────────────────────┘  │  │ │
│  │  └──────────────────────────────────────────────────────────────┘  │ │
│  │                                                                    │ │
│  │  ┌──────────────────────────────────────────────────────────────┐  │ │
│  │  │ Scheduler + Worker Pool                                      │  │ │
│  │  │ - Runs Work Item DAG concurrently                            │  │ │
│  │  │ - Respects dependencies, budgets, risk policies              │  │ │
│  │  └──────────────────────────────────────────────────────────────┘  │ │
│  │                                                                    │ │
│  │  ┌──────────────────────────────────────────────────────────────┐  │ │
│  │  │ Worker Bash Sessions (spawn-on-demand)                       │  │ │
│  │  │ - Fresh bash per Work Item, initialized from env snapshot    │  │ │
│  │  │ - Parallel execution, killed on Work Item completion         │  │ │
│  │  │ - Output streamed to slate for display in agent blocks       │  │ │
│  │  └──────────────────────────────────────────────────────────────┘  │ │
│  │                                                                    │ │
│  │  ┌──────────────────────────────────────────────────────────────┐  │ │
│  │  │ Tools (Claude Code Style)                                    │  │ │
│  │  │ Bash | Read | Write | Edit | Glob | Grep | WebFetch | Task   │  │ │
│  │  └──────────────────────────────────────────────────────────────┘  │ │
│  │                                                                    │ │
│  │  ┌──────────────────────────────────────────────────────────────┐  │ │
│  │  │ Shared Project State                                         │  │ │
│  │  │ ┌────────────┐  ┌────────────┐  ┌──────────────────────────┐ │  │ │
│  │  │ │ Artifacts  │  │ Decisions  │  │ Task State (Work Items)  │ │  │ │
│  │  │ │ (diffs,    │  │ (rationale │  │ (status + artifact ptrs) │ │  │ │
│  │  │ │  logs,     │  │  for key   │  │                          │ │  │ │
│  │  │ │  outputs)  │  │  choices)  │  │                          │ │  │ │
│  │  │ └────────────┘  └────────────┘  └──────────────────────────┘ │  │ │
│  │  └──────────────────────────────────────────────────────────────┘  │ │
│  │                                                                    │ │
│  │  ┌──────────────────────────────────────────────────────────────┐  │ │
│  │  │ ai-sdk-cpp (ClickHouse)                                      │  │ │
│  │  │ - Streaming LLM access + tool calling                        │  │ │
│  │  └──────────────────────────────────────────────────────────────┘  │ │
│  └────────────────────────────────────────────────────────────────────┘ │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

---

### Key design principles (baked in)

#### 1) Separate thinking hierarchy from execution concurrency

- **Agents decide** (Orchestrator + role executors).
- **Worker threads execute** Work Items.
- You scale concurrency by the **scheduler/worker pool**, not by spawning more "agents".

#### 2) One core abstraction: Work Item

Everything is a **Work Item**:

- goal + acceptance criteria
- inputs/constraints (repo, cwd, tool allowlist, risk level, budgets)
- dependencies
- outputs (diff, logs, test report, summary)
- assigned role (Engineer/Reviewer/Researcher/etc.)

This is the only unit the scheduler runs.

#### 3) Tree of agents, graph of tasks

- **Agent structure = tree** (simple):

  - Orchestrator (always)
  - Executor roles (hired per task)
  - Narrator (always)

- **Task plan = DAG of Work Items** (powerful):

  - parallelize independent steps safely
  - enforce dependencies (e.g., "tests" after "patch")

#### 4) Shared Project State: small, explicit

All coordination happens through a minimal shared state:

- **Artifacts**: command transcripts, stdout/stderr, diffs, files, benchmarks
- **Decisions**: short rationale for key choices
- **Task state**: Work Item status + pointers to artifacts

No agent-to-agent "chat"; agents read/write state.

#### 5) Dynamic "how many directs?" without complexity

The **TeamLead** role decides whether to:

- do it solo

- split into multiple Work Items

- hire optional roles based on simple triggers:

  - **Security** for risky commands/secrets/supply-chain changes
  - **Perf** for hot paths/regressions
  - **Researcher** for unknown domains/large design choices

#### 6) Simple control loop

For every user input:

1. classify command vs task
2. TeamLead builds/updates the Work Item DAG + acceptance criteria
3. scheduler runs ready Work Items concurrently
4. Engineer/optional roles execute → produce artifacts
5. Reviewer gates → approve or create fix Work Items
6. Narrator summarizes → updates memory (bounded)

#### 7) `slate` binary + `slated` daemon separation

6. **`slate` binary + `slated` daemon separation**: `slate` owns the terminal experience (FTXUI three-zone layout, bash co-process, command routing, agent output rendering); `slated` handles AI orchestration, memory, task scheduling, and worker bash sessions for parallel agent command execution. Clean separation of concerns.

---

### Agents (always 3)

1. **Orchestrator**

- Owns the session, user preferences, and **project auto-switching**
- Delegates to executor roles via Work Items
- Only component with long-term user context

2. **Executor (role-based)**

- Primary roles: **TeamLead, Engineer, Reviewer**
- Optional roles: **Researcher, Security, Perf**
- Runs Work Items under policies/budgets

3. **Narrator / Memory agent**

- Compresses + curates memory
- Produces succinct "state snapshots" for Orchestrator context

---

### Memory model (bounded)

- **Global memory** (user + operating habits): size-capped, curated by Narrator
- **Project memories** (per repo/project): many, each size-capped
- Orchestrator **auto-switches** project context using cwd/repo fingerprint/file references/task semantics.

---

### Command fast-pass + autocomplete cache

- `slate` binary builds a **command index** (PATH executables + builtins + aliases/functions from the persistent bash co-process's environment).

- Stores in a hash map:

  - command → type + path/builtin + completion hints

- Recognized input → **run immediately**

- Unknown/ambiguous → agent mode (TeamLead can still choose to run a command)

---

### Dynamic multi-model assignment

- **Fixed roles**: Orchestrator = frontier model, Narrator = fast/cheap model, TeamLead = strong planner
- **Dynamic workers**: TeamLead selects model per Work Item from a **model catalog** in config
- Each catalog entry describes: strengths, weaknesses, cost tier, intelligence level
- TeamLead matches Work Item complexity/capability needs to the right model at planning time
- Same role (e.g., Engineer) gets different models depending on the task
- **Future**: fork Work Items across multiple models, TeamLead picks best result or merges them

---

### Safety & audit

- Tool/command allowlist, risk classification, confirmations for destructive actions
- Full audit trail in Shared Project State: commands, outputs, diffs, decisions

---

### Impact

- **Dependencies**: ClickHouse ai-sdk-cpp, FTXUI, FlatBuffers, C++20 compiler, Bazel or CMake

---

### MVP definition

- `slate` binary + `slated` daemon IPC with FlatBuffers + streaming
- Work Item DAG + scheduler + 3-agent loop
- Fast-pass command cache + autocomplete
- Global + project memory with strict size limits and Narrator summarization
- File edits + git diffs + test runs end-to-end via tools