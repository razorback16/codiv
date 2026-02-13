## Why

Developers need an intelligent terminal assistant that seamlessly integrates with their existing shell workflow. Rather than switching between a terminal and a separate AI interface, this agent acts as an extension to zsh/bash—interpreting both traditional commands and natural language instructions.

Built in C++ for performance and powered by **ClickHouse ai-sdk-cpp** for model access (streaming + tool calling), Slate Agent provides a Claude Code-like experience directly in the terminal with a clean separation between thinking (agents) and execution (worker pool).

## What We Build

- **Shell plugin + daemon architecture**: Shell plugin for UX/context, C++ daemon for orchestration, memory, scheduling, and tool execution
- **Command fast-pass**: Daemon-maintained command index for near-zero latency on recognized commands; agent mode only when needed
- **Work Item abstraction**: Single core unit for all tasks—goal, acceptance criteria, dependencies, outputs, risk level, budgets
- **Tree of agents, graph of tasks**: 3 agent types (Orchestrator, Executor roles, Narrator) with a DAG scheduler for Work Items
- **Role-based execution**: TeamLead decomposes tasks, Engineers execute, Reviewers gate—with optional Security/Perf/Researcher roles hired on demand
- **Bounded memory model**: Global user memory + per-project memories, curated by Narrator, with automatic project context switching
- **Shared Project State**: Coordination through explicit artifacts, decisions, and task state—no agent-to-agent chat

## UX

- Looks and behaves like a normal terminal
- Input auto-detection:
  1. **Command mode**: recognized command → execute immediately
  2. **Agent mode**: otherwise → plan + execute Work Items, stream progress, show diffs/results

## Capabilities

### Core Capabilities

- `shell-plugin`: Zsh/Bash plugin that hooks into shell (intercept input, provide context—cwd, env allowlist, tty, history, last exit code) and streams output from the daemon. Hooks autocomplete using the daemon's command index.

- `daemon`: Persistent C++ daemon that manages orchestration, memory, task graph, scheduling, tool execution, and the command cache. Communicates with the shell plugin via Unix domain socket.

- `command-fast-pass`: Daemon builds and maintains a **command index** from PATH executables, builtins, aliases, and functions:
  - Stores in a hash map: command → type + path/builtin + completion hints
  - Recognized input → run immediately with near-zero latency
  - Unknown/ambiguous → routes to agent mode (TeamLead can still choose to run a command)

- `agent-system`: Three agent types, always present:

  1. **Orchestrator**
     - Owns the session, user preferences, and **project auto-switching**
     - Delegates to executor roles via Work Items
     - Only component with long-term user context

  2. **Executor (role-based)**
     - Primary roles: **TeamLead, Engineer, Reviewer**
     - Optional roles: **Researcher, Security, Perf**
     - TeamLead decides whether to do it solo, split into multiple Work Items, or hire optional roles based on triggers:
       - **Security** for risky commands/secrets/supply-chain changes
       - **Perf** for hot paths/regressions
       - **Researcher** for unknown domains/large design choices
     - Runs Work Items under policies/budgets

  3. **Narrator / Memory Agent**
     - Compresses + curates memory
     - Produces succinct "state snapshots" for Orchestrator context

- `work-item-system`: The single core abstraction—everything is a **Work Item**:
  - Goal + acceptance criteria
  - Inputs/constraints (repo, cwd, tool allowlist, risk level, budgets)
  - Dependencies (forming a DAG)
  - Outputs (diff, logs, test report, summary)
  - Assigned role (Engineer/Reviewer/Researcher/etc.)
  - The scheduler runs ready Work Items concurrently via a worker thread pool

- `tool-system`: Claude Code style tools for agent execution:
  - `Bash`: Shell command execution with timeout and background support
  - `Read`: File content retrieval
  - `Write`: File creation/overwrite
  - `Edit`: String replacement in files
  - `Glob`: File pattern matching
  - `Grep`: Content search (ripgrep-based)
  - `WebFetch`: URL content retrieval
  - `Task`: Sub-agent spawning for complex subtasks

- `memory-system`: Bounded memory model:
  - **Global memory** (user + operating habits): size-capped, curated by Narrator
  - **Project memories** (per repo/project): many, each size-capped
  - Orchestrator **auto-switches** project context using cwd/repo fingerprint/file references/task semantics

- `shared-project-state`: All coordination through minimal shared state:
  - **Artifacts**: command transcripts, stdout/stderr, diffs, files, benchmarks
  - **Decisions**: short rationale for key choices
  - **Task state**: Work Item status + pointers to artifacts
  - No agent-to-agent "chat"; agents read/write state

- `skill-system`: Extensible slash-command plugins
  - Built-in skills: `/commit`, `/plan`, `/tasks`, `/help`, `/history`
  - User-defined skills via `~/.config/slate-agent/skills/`

## Control Loop

For every user input:

1. Classify command vs task
2. TeamLead builds/updates the Work Item DAG + acceptance criteria
3. Scheduler runs ready Work Items concurrently
4. Engineer/optional roles execute → produce artifacts
5. Reviewer gates → approve or create fix Work Items
6. Narrator summarizes → updates memory (bounded)

## Key Design Principles

1. **Separate thinking from execution**: Agents decide (Orchestrator + role executors). Worker threads execute Work Items. Scale concurrency by the scheduler/worker pool, not by spawning more agents.

2. **One core abstraction—Work Item**: Everything flows through Work Items. This is the only unit the scheduler runs. Uniform handling of all task types.

3. **Tree of agents, graph of tasks**: Agent structure is a simple tree (Orchestrator → Executor roles → Narrator). Task plan is a DAG of Work Items—parallelize independent steps safely, enforce dependencies.

4. **Shared Project State over agent chat**: All coordination through explicit artifacts, decisions, and task state. No implicit agent-to-agent communication.

5. **Dynamic role hiring without complexity**: TeamLead decides scope—solo, multi-step, or with optional specialist roles—based on simple triggers rather than complex strategy selection.

6. **Shell Plugin + Daemon separation**: Plugin handles UX/shell hooks; daemon handles AI/tools. Clean separation, no PTY complexity.

7. **Command fast-pass**: Recognized commands execute immediately without AI round-trip. Command index provides both routing and autocomplete.

8. **Bounded memory**: All memory is size-capped. Narrator curates aggressively. Project context auto-switches based on cwd/repo fingerprint.

## Impact

- **Dependencies**: ClickHouse ai-sdk-cpp (streaming + tool calling), C++20 compiler, Bazel or CMake
- **Shell Integration**: Plugin files for Zsh/Bash, Unix domain socket IPC
- **API Keys**: Requires configuration for LLM provider API credentials
- **User Environment**: Needs access to user's filesystem, ability to execute shell commands
- **Storage**: Shared Project State + memory at ~/.slate-agent/ or XDG-compliant location
- **Performance**: Near-zero latency for fast-pass commands, streaming responses, concurrent Work Item execution via worker pool

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           Slate Agent                                   │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  ┌──────────────┐     ┌──────────────┐     ┌──────────────────┐         │
│  │ Shell Plugin │────▶│    Daemon    │────▶│  Command Index   │         │
│  │ (zsh/bash)   │◀────│ (Unix Socket)│◀────│  (Fast-Pass)     │         │
│  └──────────────┘     └──────────────┘     └──────────────────┘         │
│                              │                      │                   │
│                              ▼                      ▼                   │
│  ┌──────────────────────────────────────────────────────────────┐       │
│  │                       Agent System                           │       │
│  │                                                              │       │
│  │  ┌────────────────────────────────────────────────────────┐  │       │
│  │  │ Orchestrator                                           │  │       │
│  │  │ - Owns session + user preferences                      │  │       │
│  │  │ - Project auto-switching (cwd/repo fingerprint)        │  │       │
│  │  │ - Delegates to executor roles via Work Items           │  │       │
│  │  └─────────────────────────┬──────────────────────────────┘  │       │
│  │                            │ delegates                        │       │
│  │  ┌─────────────────────────▼──────────────────────────────┐  │       │
│  │  │ Executor Roles                                         │  │       │
│  │  │ - Primary: TeamLead, Engineer, Reviewer                │  │       │
│  │  │ - Optional: Researcher, Security, Perf                 │  │       │
│  │  │ - TeamLead decomposes → Work Item DAG                  │  │       │
│  │  │ - Engineer executes → produces artifacts               │  │       │
│  │  │ - Reviewer gates → approve or create fix items         │  │       │
│  │  └────────────────────────────────────────────────────────┘  │       │
│  │                                                              │       │
│  │  ┌────────────────────────────────────────────────────────┐  │       │
│  │  │ Narrator / Memory Agent                                │  │       │
│  │  │ - Compresses + curates memory                          │  │       │
│  │  │ - Global memory (user habits) + Project memories       │  │       │
│  │  │ - Produces state snapshots for Orchestrator            │  │       │
│  │  └────────────────────────────────────────────────────────┘  │       │
│  └──────────────────────────────────────────────────────────────┘       │
│                                                                         │
│  ┌──────────────────────────────────────────────────────────────┐       │
│  │ Scheduler + Worker Pool                                     │       │
│  │ - Runs Work Item DAG concurrently                           │       │
│  │ - Respects dependencies, budgets, risk policies             │       │
│  └──────────────────────────────────────────────────────────────┘       │
│                                                                         │
│  ┌──────────────────────────────────────────────────────────────┐       │
│  │ Tools (Claude Code Style)                                    │       │
│  │ Bash | Read | Write | Edit | Glob | Grep | WebFetch | Task   │       │
│  └──────────────────────────────────────────────────────────────┘       │
│                                                                         │
│  ┌──────────────────────────────────────────────────────────────┐       │
│  │ Shared Project State                                         │       │
│  │ ┌────────────┐  ┌────────────┐  ┌──────────────────────────┐ │       │
│  │ │ Artifacts  │  │ Decisions  │  │ Task State (Work Items)  │ │       │
│  │ │ (diffs,    │  │ (rationale │  │ (status + artifact ptrs) │ │       │
│  │ │  logs,     │  │  for key   │  │                          │ │       │
│  │ │  outputs)  │  │  choices)  │  │                          │ │       │
│  │ └────────────┘  └────────────┘  └──────────────────────────┘ │       │
│  └──────────────────────────────────────────────────────────────┘       │
│                                                                         │
│  ┌──────────────────────────────────────────────────────────────┐       │
│  │ ai-sdk-cpp (ClickHouse)                                      │       │
│  │ - Streaming LLM access + tool calling                        │       │
│  └──────────────────────────────────────────────────────────────┘       │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

## Safety & Audit

- Tool/command allowlist, risk classification, confirmations for destructive actions
- Full audit trail in Shared Project State: commands, outputs, diffs, decisions

## Multi-Model Strategy

Each agent role has different intelligence requirements. Rather than using a single model, the system assigns **purpose-fit models** to each role and lets the TeamLead dynamically select models for worker agents based on Work Item characteristics.

### Fixed Role Assignments

| Role | Model | Rationale |
|------|-------|-----------|
| **Orchestrator** | Claude Opus 4.6 | Highest reasoning capability. Owns session-level decisions, intent interpretation, project switching. Needs deep understanding of user goals and long-term context. Worth the cost since it runs infrequently. |
| **Narrator** | Gemini 2.5 Flash | Fast, cheap, high-throughput. Memory compression and summarization is a well-defined task that doesn't require frontier reasoning. Runs frequently—cost and latency matter. |
| **TeamLead** | GPT-5.2 Thinking High | Strong planning and decomposition. Needs to analyze tasks, design Work Item DAGs, set acceptance criteria, and decide which specialist roles to hire. Thinking mode gives it structured reasoning for upfront planning. |

### Dynamic Model Selection for Workers

The TeamLead selects the model for each Work Item's assigned worker (Engineer, Reviewer, Researcher, Security, Perf) at planning time. The selection is guided by a **model catalog** defined in the config file. Each entry describes a model's strengths, weaknesses, cost tier, and the intelligence level it's suited for.

The TeamLead's system prompt includes the full model catalog and instructions to match Work Items to models based on:

- **Task complexity**: simple file operations vs. multi-file refactoring vs. architectural reasoning
- **Required capability**: code generation, code review, security analysis, research synthesis
- **Cost sensitivity**: routine tasks use cheaper models; critical-path tasks use stronger ones
- **Latency requirements**: interactive feedback loops favor fast models; batch tasks can use slower ones

This means the same "Engineer" role might use a cheap fast model for a trivial file rename but a frontier model for a complex refactoring—decided per Work Item, not per role.

### Model Catalog (Config)

```toml
# ~/.config/slate-agent/config.toml

# --- Fixed role assignments ---

[roles.orchestrator]
provider = "anthropic"
model = "claude-opus-4-6"
api_key_env = "ANTHROPIC_API_KEY"

[roles.narrator]
provider = "google"
model = "gemini-2.5-flash"
api_key_env = "GOOGLE_API_KEY"

[roles.teamlead]
provider = "openai"
model = "gpt-5.2"
reasoning_effort = "high"
api_key_env = "OPENAI_API_KEY"

# --- Model catalog for dynamic worker assignment ---
# TeamLead reads this catalog via system prompt to select models per Work Item.
# Each entry describes what the model is good/bad at so TeamLead can make
# informed decisions.

[[models]]
id = "claude-opus-4-6"
provider = "anthropic"
cost_tier = "high"                     # "low", "medium", "high"
intelligence = "frontier"              # "basic", "standard", "advanced", "frontier"
strengths = "Deep reasoning, complex architecture, nuanced code review, multi-file refactoring, security analysis"
weaknesses = "Slow, expensive—overkill for simple tasks"
best_for = "Critical-path work items requiring highest accuracy"

[[models]]
id = "claude-sonnet-4-5"
provider = "anthropic"
cost_tier = "medium"
intelligence = "advanced"
strengths = "Strong code generation, good reasoning, fast for its capability level"
weaknesses = "May miss subtle architectural issues that frontier models catch"
best_for = "Standard engineering tasks: implement features, write tests, code review"

[[models]]
id = "claude-haiku-4-5"
provider = "anthropic"
cost_tier = "low"
intelligence = "standard"
strengths = "Very fast, cheap, good at well-defined tasks"
weaknesses = "Limited reasoning on ambiguous or complex problems"
best_for = "Simple file operations, formatting, linting, straightforward edits"

[[models]]
id = "gemini-2.5-pro"
provider = "google"
cost_tier = "medium"
intelligence = "advanced"
strengths = "Large context window, strong at research synthesis, good code understanding"
weaknesses = "Can be verbose, occasionally less precise on surgical edits"
best_for = "Research tasks, large codebase analysis, documentation"

[[models]]
id = "gpt-5.2"
provider = "openai"
cost_tier = "high"
intelligence = "frontier"
reasoning_effort = "high"
strengths = "Excellent structured planning, strong reasoning chains, good at decomposition"
weaknesses = "Expensive, slower with thinking enabled"
best_for = "Complex planning, architectural decisions, difficult debugging"

[[models]]
id = "gpt-4o"
provider = "openai"
cost_tier = "medium"
intelligence = "advanced"
strengths = "Fast, multimodal, good general-purpose coding"
weaknesses = "Less deep reasoning than thinking models"
best_for = "General engineering, quick iterations, visual/UI tasks"

# --- Other settings ---

[tools]
bash_timeout_ms = 120000
read_line_limit = 2000
write_require_read_first = true

[memory]
global_max_size_kb = 64
project_max_size_kb = 128

[privacy]
send_command_output_to_llm = false
send_file_contents_to_llm = true
```

### How TeamLead Selects Models

The TeamLead receives the model catalog as part of its system prompt. When creating a Work Item, it assigns a `model_id` field based on its assessment of the task. Example decision flow:

1. **"Rename variable across 3 files"** → `claude-haiku-4-5` (simple, well-defined, cheap)
2. **"Refactor auth module to use JWT"** → `claude-sonnet-4-5` (standard engineering, good balance)
3. **"Audit this PR for security vulnerabilities"** → `claude-opus-4-6` (security-critical, needs deep reasoning)
4. **"Research how this unfamiliar framework handles routing"** → `gemini-2.5-pro` (large context, research task)
5. **"Debug a complex race condition"** → `gpt-5.2` thinking high (needs structured reasoning chains)

The Work Item's `model_id` is respected by the scheduler when spawning the worker's LLM context. Users can override with a `[roles.default_worker]` setting to force a single model for all workers if preferred.

## MVP Definition

- Shell plugin + daemon IPC working with streaming
- Work Item DAG + scheduler + 3-agent loop (Orchestrator, Executors, Narrator)
- Fast-pass command cache + autocomplete
- Global + project memory with strict size limits and Narrator summarization
- File edits + git diffs + test runs end-to-end via tools

## Deferred Features

The following are explicitly NOT in MVP:

- Fish shell support
- Backpressure system (tool output, Work Item spawning)
- Scoped capability tokens (session/cwd/repo permissions)
- Circuit breaker with automatic tier fallback
- Voice agent integration
- MCP server support
- **Multi-model forked work trees**: TeamLead assigns the same Work Item to multiple models in parallel, producing independent implementations. TeamLead then either judges which result is best or creates an auxiliary "merge" Work Item to combine the strongest parts of each into a final implementation. Useful for critical-path tasks where getting the best possible output justifies the extra cost.

These can be added when real-world usage patterns emerge.
