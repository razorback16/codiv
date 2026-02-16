## One-pager: C++ Terminal Coding Agent (Claude Code–style) using ai-sdk-cpp

**Version**: 1.0 **Date**: 2026-02-10 **Author**: Subhagato **Status**: Draft

### Goal

A **terminal-native** coding agent for **bash/zsh** that can:

- **Fast-pass** real shell commands with near-zero latency
- Interpret **natural language** tasks (debug, refactor, implement, explain) and execute them end-to-end
- See command outputs, edit files, run tests, and keep bounded long-term + project memory

Powered by **ClickHouse ai-sdk-cpp** for model access (streaming + tool calling).

---

### UX

- Looks and behaves like a normal terminal.

- Input auto-detection:

  1. **Command mode**: recognized command → execute immediately
  2. **Agent mode**: otherwise → plan + execute Work Items, stream progress, show diffs/results

---

### Architecture

**C++ daemon + shell plugin (IPC)**

- **Daemon (persistent)**: orchestration, memory, task graph, scheduling, tool execution, command cache
- **Shell plugin**: intercept input, provide context (cwd/env allowlist/tty), stream output, hook autocomplete
- IPC: Unix domain socket (default)

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

- Daemon builds a **command index** (PATH executables + builtins + aliases/functions from the shell plugin).

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

### MVP definition

- Shell plugin + daemon IPC working with streaming
- Work Item DAG + scheduler + 3-agent loop
- Fast-pass command cache + autocomplete
- Global + project memory with strict size limits and Narrator summarization
- File edits + git diffs + test runs end-to-end via tools