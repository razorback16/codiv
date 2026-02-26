# Slate Agent — Implementation Plan

**Date**: 2026-02-26 **Author**: Subhagato **Status**: Active

---

## 1. Overview

**Slate Agent** is a Rust/C++ terminal-native coding agent that replaces the traditional shell with an intelligent, multi-model AI assistant. The Rust client (`slate`) provides a ratatui-based TUI that looks and behaves like a normal terminal, while the C++ daemon (`slated`) handles AI orchestration, Work Item scheduling, and tool execution. Users type shell commands that execute instantly (zero-latency fast-pass), or natural language that triggers a recursive multi-agent system with different models assigned to different roles (planning, coding, review, research). Memory persists across sessions, project context auto-switches on `cd`, and a unified tool system supports binary tools, prompt tools, MCP bridges, hooks, and aliases.

**Current status**: Phase 1 (Terminal Foundation) is complete. Phase 2 (Single-Agent AI Loop) is next.

---

## 2. Phase Summary

| Phase | Name | Goal | Status | Key Deliverables |
| --- | --- | --- | --- | --- |
| **1** | Terminal Foundation | Working terminal client with daemon IPC | **COMPLETE** | Rust TUI, C++ daemon, FlatBuffers IPC, command fast-pass, tab completion, interactive passthrough |
| **2** | Single-Agent AI Loop | Natural language input routes to AI agent with tool calling | **NEXT** | ai-sdk-cpp integration, single agent loop, built-in tools, markdown rendering, basic safety |
| **3** | Work Item DAG + Scheduler | Complex tasks decomposed into concurrent Work Items | Planned | Work Item schema, DAG construction, Taskflow scheduler, budget enforcement |
| **4** | Multi-Agent Roles + Multi-Model | Specialized agent roles with dynamic model selection | Planned | Role separation, model catalog, TeamLead model selection, Reviewer gating |
| **5** | Memory + Project Context | Persistent bounded memory across sessions and projects | Planned | SQLite episodic store, semantic markdown, Narrator consolidation, project auto-switching |
| **6** | Tool System | Unified tool registry with MCP bridge and prompt tools | Planned | Tool registry, MCP bridge, prompt tool runtime, progressive loading, hooks, aliases |
| **7** | Advanced Safety & Audit | Production-grade safety, audit trail, compliance | Planned | Full audit trail, privacy config, OS-level sandboxing roadmap, OWASP/NIST alignment |

---

## 3. Phase Dependency Graph

```
Phase 1: Terminal Foundation  [COMPLETE]
    |
    v
Phase 2: Single-Agent AI Loop + Basic Safety  [NEXT]
    |
    v
Phase 3: Work Item DAG + Scheduler
    |
    v
Phase 4: Multi-Agent Roles + Multi-Model
    |
    v
Phase 5: Memory + Project Context
    |
    v
Phase 6: Tool System
    |
    v
Phase 7: Advanced Safety & Audit
```

Each phase is a **vertical slice** — fully functional and manually testable on its own. Later phases build incrementally on earlier ones.

---

## 4. Detailed Phase Breakdowns

### Phase 1: Terminal Foundation — COMPLETE

The foundation layer provides a working terminal client that executes commands via daemon IPC with near-zero overhead.

**What was built:**

- `slate` binary (~4,300 lines of Rust) with ratatui 0.30 + crossterm 0.28 for linear scroll-down terminal flow
- `slated` daemon (~1,240 lines of C++20) listening on Unix socket with FlatBuffers IPC protocol
- Persistent bash co-process via portable-pty with sentinel-based output boundary detection
- Interactive command passthrough (vim, ssh, python REPL) with dedicated PTY and raw terminal mode
- Command fast-pass: PATH scanning + 65 bash builtins in O(1) hash map with input classification (Execute, Interactive, AiQuery, NotFound, Clear, Reset, Exit, Empty)
- 3-tier tab completion: programmable (bash-completion integration) → command → file
- Env snapshot protocol: session_id, env_vars, PATH, cwd captured on connect, stored per-session in daemon
- Heartbeat mechanism (5s interval from client, 30s stale timeout in daemon)
- Worker bash sessions: spawn-on-demand per Work Item, initialized from env snapshot, killed on completion
- Structured logging with `--debug` flag to `/tmp/slate-debug.log`

**Tech stack**: Rust (Cargo) for `slate`, C++20 (CMake) for `slated`, FlatBuffers over Unix domain socket, portable-pty, tui-term + vt100, GoogleTest. See the [PRD Appendix A](PRD.md#appendix-a-phase-1-module-details) for full module tables.

---

### Phase 2: Single-Agent AI Loop — NEXT

**Goal**: Unknown/natural language input routes to a single AI agent that can reason and use tools.

#### Sub-tasks

1. **Integrate ai-sdk-cpp for streaming LLM access**
   - Add ai-sdk-cpp (ClickHouse) as a dependency in the CMake build
   - Configure streaming HTTP connections to OpenAI and Anthropic endpoints
   - Implement SSE parsing for token-by-token streaming
   - Handle tool calling protocol (function calls in the LLM response, tool results sent back)

2. **Implement single agent (Orchestrator+Engineer combined) with tool calling loop**
   - Agent receives user input classified as `AiQuery` from the `slate` client
   - System prompt defines the combined Orchestrator+Engineer role
   - Agent reasons, decides which tool to call, receives tool output, continues reasoning
   - Loop terminates when agent produces a final response (no more tool calls)
   - Agent state managed in `slated` daemon per session

3. **Implement built-in tools: Bash, Read, Write, Edit, Glob, Grep**
   - `Bash`: execute commands via worker bash sessions (already built in Phase 1), capture stdout/stderr/exit code
   - `Read`: read file contents with optional line range (offset + limit)
   - `Write`: write file to disk (requires prior read for overwrite safety)
   - `Edit`: exact string replacement in files (old_string → new_string)
   - `Glob`: file pattern matching (returns sorted file paths)
   - `Grep`: content search using ripgrep-style regex
   - Each tool exposes `--help` for agent consumption

4. **Add terminal markdown rendering (comrak + syntect)**
   - Use comrak for CommonMark+GFM parsing
   - Use syntect for syntax highlighting in code blocks (same engine as Sublime Text)
   - Walk AST to emit ANSI escape codes
   - For streaming: maintain growing buffer, re-parse on significant updates, diff rendered output

5. **Basic risk classification + confirmation prompts for destructive commands**
   - Classify commands as low/medium/high/critical risk
   - High/critical risk triggers confirmation prompt sent from `slated` → `slate` → user
   - Command allowlist/denylist configurable in TOML config
   - No agent auto-execution of critical-risk commands (e.g., `rm -rf /`, `git push --force`, `DROP TABLE`)

6. **Env snapshot refresh on cd/source**
   - Detect `cd` and `source` commands in the `slate` bash co-process
   - Auto-capture fresh env snapshot and send to `slated`
   - Manual `slate sync-env` command for edge cases

7. **Color-coded agent output streaming to slate**
   - Agent output streams from `slated` to `slate` via IPC
   - Rendered in color-bordered blocks (cyan for Engineer role)
   - Auto-collapse to single-line summary on completion; Enter to expand

8. **TOML config for API keys and model selection**
   - Config file at `~/.slate-agent/config.toml`
   - `[models]` section with API key and model ID
   - `[safety]` section with allowlist/denylist

**Testable outcome**: User types "create a hello world C++ program, compile it, and run it" — agent creates the file, runs g++, executes the binary, and streams the output. User types `ls` — still fast-passes. Agent attempting `rm -rf /` triggers a confirmation prompt.

**Tech**: ai-sdk-cpp (streaming + tool calling), Claude Sonnet 4.5 as initial model, comrak + syntect for rendering.

---

### Phase 3: Work Item DAG + Scheduler

**Goal**: Agent decomposes complex tasks into a Work Item DAG and executes items concurrently.

#### Sub-tasks

1. **Work Item data structure with full schema**
   - Fields: id, goal, acceptance_criteria, dependencies, state, assigned_role, model_id, risk_level, token_budget, cost_budget, inputs, outputs, parent_work_item_id, created_at, completed_at
   - State machine: `pending` → `running` → `completed` | `failed` | `blocked`
   - Serializable to/from JSON for IPC and storage

2. **DAG construction from agent planning**
   - Agent (still single combined role) produces a plan as a set of Work Items
   - Dependencies expressed as edges between Work Item IDs
   - Validation: no cycles, all referenced IDs exist, at least one root node

3. **Taskflow-based concurrent execution**
   - Integrate Taskflow library (header-only C++20) into `slated`
   - Map Work Item DAG to `tf::Taskflow` with `precede()`/`succeed()` for dependency edges
   - `tf::Executor` manages the worker thread pool
   - Each scheduled Work Item gets its own worker bash session

4. **State machine (pending → running → completed/failed)**
   - Track Work Item state transitions in daemon memory
   - Stream state updates to `slate` for progress display
   - Handle failure: Work Item → `failed`, dependents → `blocked`

5. **Artifact storage in Shared Project State**
   - Each Work Item's outputs (files, diffs, stdout) stored as artifacts
   - Storage at `~/.slate-agent/state/`
   - Artifacts keyed by Work Item ID

6. **Budget enforcement (token + cost caps)**
   - Track token usage per Work Item (input + output tokens from LLM responses)
   - Track cost per Work Item (tokens × model cost_tier rate)
   - When budget exceeded: pause Work Item, log the overage, fail the item

**Testable outcome**: User types "add input validation to the user registration form, write tests, and update the README" — agent creates 3+ Work Items with correct dependencies, runs independent ones in parallel, then tests after validation is done.

**Tech**: Taskflow (header-only C++20, work-stealing scheduler, composable sub-taskflows).

---

### Phase 4: Multi-Agent Roles + Multi-Model

**Goal**: Separate agent roles with different models assigned per role.

#### Sub-tasks

1. **Agent role separation**
   - **Orchestrator**: session management, intent classification, project context switching
   - **TeamLead**: task decomposition into Work Item DAG, acceptance criteria, model selection per Work Item
   - **Engineer**: Work Item execution (file edits, command runs, code generation)
   - **Reviewer**: validates Work Item outputs against acceptance criteria, creates fix Work Items on failure
   - **Narrator**: memory compression and curation (runs after Work Item completion)
   - Each role gets a distinct system prompt defining its capabilities and constraints

2. **Model catalog in TOML config**
   - `[[models]]` array with fields: id, provider, api_key_env, cost_tier, intelligence_level, strengths, weaknesses, best_for
   - Fixed role assignments: Orchestrator = frontier model, Narrator = fast/cheap model, TeamLead = strong planner
   - User override: `[roles.default_worker]` to force a single model for all workers

3. **TeamLead dynamic model selection per Work Item**
   - TeamLead evaluates Work Item complexity and selects model from catalog
   - Selection factors: task complexity, cost_tier, intelligence_level, model strengths
   - Model ID recorded on the Work Item for observability

4. **Reviewer gating**
   - Every Work Item output passes through Reviewer before being applied
   - Reviewer checks outputs against acceptance criteria
   - On failure: Reviewer creates a fix Work Item or escalates to TeamLead

5. **Optional role hiring (Security, Perf, Researcher)**
   - TeamLead detects task characteristics that warrant specialist roles
   - Security: infrastructure changes, credential handling, supply-chain modifications
   - Perf: performance-sensitive code, benchmark tasks
   - Researcher: unfamiliar domains, external documentation lookup
   - Hired roles are additional Work Item assignees, not permanent

6. **Single-writer ownership enforcement**
   - Resource ownership tracked per file/artifact
   - One agent holds write access at a time; others can read
   - Ownership released on Work Item completion

**Testable outcome**: User types "refactor the auth module to use JWT and audit it for security issues" — Orchestrator delegates to TeamLead, TeamLead assigns Engineer (Sonnet) for refactoring + Security (Opus) for audit. Reviewer validates. User sees which model handled which part.

**Tech**: Multiple concurrent ai-sdk-cpp sessions, role-based system prompts, single-writer ownership (no reader-writer locks).

---

### Phase 5: Memory + Project Context

**Goal**: Persistent, bounded memory that survives sessions and auto-switches per project.

Architecture is defined in the [Memory System Design](design-docs/memory-system-design.md). This phase implements that design.

#### Sub-tasks

1. **SQLite episodic store**
   - Create `~/.slate-agent/memory/memory.db` with the episodes table schema
   - Write episodes at tool execution, Work Item completion, and explicit `memory_write_episode` calls
   - Append-only during sessions; Narrator marks episodes as `consolidated`
   - Retention policy: consolidated episodes pruned after 90 days (365 for high-value), DB size cap 50MB

2. **Semantic markdown files**
   - `user.md` (64KB cap): cross-project user preferences, habits
   - `project.md` (128KB cap per project): conventions, architecture, decisions
   - `topics/*.md` (32KB cap per topic): topic-scoped knowledge
   - `topics.toml` index: name, file, tags, last_updated, episode_count

3. **Narrator consolidation algorithm**
   - Runs after each completed Work Item
   - Steps: Extract unconsolidated episodes → Classify by scope → Update semantic files → Manage topics → Mark consolidated
   - Narrator is an LLM agent with Read, Write, memory_search, memory_mark_consolidated tools
   - Uses the fast/cheap model (e.g., Gemini 2.5 Flash)

4. **Memory tools**
   - `memory_search`: query episodic store by keyword, scope, event type, recency
   - `memory_read`: load a topic file into agent context
   - `memory_list_topics`: list available topics from topics.toml
   - `memory_write_episode`: explicitly record information worth remembering
   - Available to all agent roles

5. **Project auto-switching with fingerprint detection**
   - Detection signals (priority order): explicit config → git remote → git root → cwd path
   - Fingerprint is a stable hash of the primary identifier
   - On switch: unload previous project.md, load new project.md, update `SLATE_PROJECT_FINGERPRINT` env var
   - In-flight Work Items continue with their original project context

**Testable outcome**: User completes a task in repo A ("always use pytest, not unittest"). User starts a new session in repo A — agent remembers the preference. User cd's to repo B — agent switches to repo B's context automatically.

**Tech**: SQLite (C++ integration), semantic markdown, Narrator LLM agent, project fingerprinting.

---

### Phase 6: Tool System

**Goal**: Unified tool system — registry, MCP bridge, prompt tool runtime, progressive loading, hooks, aliases.

Architecture is defined in the [Unified Tool Model Design](design-docs/unified-tool-model-design.md). This phase implements that design.

#### Sub-tasks

1. **Tool registry CLI**
   - `slate install <tool>` / `slate install mcp:<package>` / `slate install ./path`
   - `slate remove <tool>` / `slate update [tool]` / `slate search "query"`
   - `slate tools search "query"` / `slate tools list` / `slate tools info <tool>`
   - `slate import skill ./path` for converting Claude Code SKILL.md format

2. **SLATE_TOOLS_PATH discovery**
   - Scan directories: project-level (`.slate-agent/tools/`), user-level (`~/.slate-agent/tools/`), system-level (`/usr/local/share/slate-agent/tools/`)
   - Run `<tool> --help` for each discovered executable (cached, re-run on mtime change)
   - Build tool index: name → help text → optional agent-guide → location

3. **MCP bridge**
   - Thin wrapper executables that translate CLI interface to MCP protocol
   - `--help` → MCP `tools/list`; commands → MCP `tools/call`; `--json-out` → passthrough
   - Daemon manages MCP server subprocesses: lazy spawn, session-lifetime, health-check via `ping`
   - Transport: stdio (subprocess) and HTTP/SSE (remote)

4. **Prompt tool runtime**
   - Daemon synthesizes `--help` and `--agent-guide` for tools without binaries
   - `guide.md` content injected after argument substitution and `[skill.inject]` execution
   - Execution models: `inline` (inject into current context) or `fork` (sub-session)
   - Bundled resources: `scripts/` (runtime), `references/` (Tier 3), `assets/` (output only)

5. **Progressive loading (Tier 0-3)**
   - Tier 0 (always): tool name + one-line description
   - Tier 1 (on relevance): full `--help` output
   - Tier 2 (on invocation): `--agent-guide` content
   - Tier 3 (on demand): `--agent-guide <topic>` deep docs

6. **Hooks and aliases**
   - Hooks: `[[hooks]]` in config — event + tool + args, with optional `match` filter
   - Events: session-start, session-end, pre-tool-use, post-tool-use, pre-commit, post-task, pre-compact
   - Aliases: `[[aliases]]` in config — slash command → tool invocation
   - Prompt tools with `user-invocable = true` auto-register as slash commands

**Testable outcome**: `/commit` works. `slate install ./my-tool` works. `slate install mcp:@modelcontextprotocol/server-filesystem` makes the MCP server appear as a regular tool.

**Tech**: Tool registry, MCP bridge (cpp-mcp or custom), prompt tool synthesis, SLATE_TOOLS_PATH discovery.

---

### Phase 7: Advanced Safety & Audit

**Goal**: Production-grade safety controls, full audit trail, compliance alignment, and graceful degradation.

#### Sub-tasks

1. **Full audit trail**
   - Log every command, tool invocation, LLM call, output, diff, and decision to `~/.slate-agent/audit/`
   - Machine-parseable JSON lines format with timestamps, Work Item IDs, agent roles, model IDs
   - `slate audit` command for session review and search
   - Configurable retention policy

2. **Privacy settings**
   - `[privacy]` section in config
   - Options: exclude command output from LLM context, redact env vars, mask file paths
   - Per-project overrides via `.slate-agent/config.toml` in repo root

3. **OS-level sandboxing roadmap**
   - Linux: bubblewrap (bwrap) for filesystem and network isolation of spawned scripts
   - macOS: seatbelt profiles for sandbox-exec
   - Docker-based sandboxing as advanced option (dedicated microVM with own Docker daemon)
   - Agent runs with full permissions inside the sandbox (safe because isolated)

4. **OWASP/NIST alignment**
   - OWASP LLM Top 10 (LLM06: Excessive Agency): tool allowlists, risk classification, confirmation gates
   - NIST AI Risk Management Framework: audit trail, explainability (decisions logged with rationale), governance (config-driven policies)

5. **Error handling and graceful degradation**
   - Daemon crash recovery: heartbeat detection, auto-restart offer, standalone fallback
   - LLM timeout: exponential backoff retries, model substitution, Work Item failure with notification
   - IPC connection loss: reconnection with backoff, input buffering
   - Disk full: graceful degradation to in-memory context
   - Invalid config: clear error messages with key identification and expected format

**Testable outcome**: Full session audit log on disk with every command, output, and decision. Privacy config excludes sensitive output from LLM context. Config with invalid model ID produces a clear error message pointing to the problem.

**Tech**: Audit logging, seatbelt/bwrap sandboxing, config validation.

---

## 5. Cross-Cutting Concerns

### Testing Strategy

| Layer | Framework | Scope |
| --- | --- | --- |
| C++ daemon unit tests | GoogleTest v1.14.0 | Worker process management, IPC protocol, Work Item state machine, tool implementations, scheduler |
| Rust client unit tests | `cargo test` (built-in) | Input classification, command index, completion engine, IPC message building |
| Rust integration tests | `cargo test` (integration test modules) | End-to-end command execution, daemon connection, env snapshot round-trip |
| End-to-end tests | expect-style scripting (e.g., rexpect or custom) | Full user scenarios: launch `slate`, type commands, verify output, test agent interactions |
| LLM interaction tests | Mock LLM server (record/replay) | Agent tool calling loop, Work Item creation, role delegation, error handling |

### CI/CD

- **GitHub Actions** with parallel jobs:
  - `cargo build` + `cargo test` + `cargo clippy` + `cargo fmt --check` for `slate`
  - `cmake --build` + `ctest` + `clang-tidy` + `clang-format --dry-run` for `slated`
- Matrix: macOS (primary), Linux (secondary)
- FlatBuffers schema compilation verified in both Rust and C++ builds
- LLM tests use recorded responses (no live API calls in CI)

### Documentation

- Design docs per phase in `docs/design-docs/`
- PRD kept up-to-date as phases complete
- Plan.md (this document) updated with status changes
- In-code documentation: Rustdoc for `slate`, Doxygen-style comments for `slated`

### Code Quality

| Language | Linting | Formatting | Static Analysis |
| --- | --- | --- | --- |
| Rust | `clippy` (deny warnings) | `rustfmt` | Built-in borrow checker + `clippy::pedantic` |
| C++ | `clang-tidy` | `clang-format` (Google style, 4-space indent) | `-Wall -Wextra -Werror` + AddressSanitizer in debug |

---

## 6. Open Questions / Decisions Needed

1. **ai-sdk-cpp evaluation**: Is the ClickHouse SDK (~134 stars) mature enough for production use, or do we need a custom HTTP client (libcurl + cpr + custom SSE parser)? Evaluate early in Phase 2. Key risks: Google/Cohere not yet supported, unknown edge cases in tool calling protocol.

2. **Narrator model**: Is Gemini 2.5 Flash sufficient for the memory consolidation algorithm (topic matching, semantic merging, compression decisions), or does topic management need a more capable model?

3. **Topic granularity**: What is the minimum episode count before creating a new topic? Without a threshold, one-off tasks could cause topic explosion. Candidates: 3 episodes minimum, or require 2+ sessions contributing to the same tag cluster.

4. **Cross-project knowledge flow**: Should semantic knowledge ever flow from one project to another beyond `user.md`? Example: "this user always prefers X pattern" could be inferred from project-scoped data. Currently only `user.md` is cross-project.

5. **Vector DB trigger**: When should we evaluate adding a vector DB for memory retrieval? Proposed trigger: if keyword-based `memory_search` recall accuracy falls below 80% on a representative query set, investigate vector embeddings (e.g., SQLite + sqlite-vec extension).

6. **ai-sdk-cpp provider coverage**: If Google and Cohere support is needed before ai-sdk-cpp adds it, what is the fallback strategy? Options: contribute upstream, maintain a fork, or build a minimal custom client for those providers only.

7. **Recursive decomposition depth**: What is the practical limit for sub-TeamLead deployment? ADAPT research suggests 2-3 levels for most coding tasks. Should we enforce a hard limit or rely on budget caps to naturally bound depth?

---

*Slate Agent Implementation Plan — 7 phases from terminal foundation to production-grade safety. Phase 1 complete. Phase 2 next.*
