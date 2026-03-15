# Codiv Agent — Implementation Plan

**Date**: 2026-02-26 **Author**: Subhagato **Status**: Active

---

## 1. Overview

**Codiv Agent** is a Rust terminal-native coding agent that replaces the traditional shell with an intelligent, multi-model AI assistant. The Rust client (`codiv`) provides a ratatui-based TUI that looks and behaves like a normal terminal, while the Rust daemon (`codivd`) handles AI orchestration, Work Item scheduling, and tool execution. Users type shell commands that execute instantly (zero-latency fast-pass), or natural language that triggers a recursive multi-agent system with different models assigned to different roles (planning, coding, review, research). Memory persists across sessions, project context auto-switches on `cd`, and a unified tool system supports binary tools, prompt tools, MCP bridges, hooks, and aliases.

**Current status**: Phase 1 (Terminal Foundation) and Phase 2 (Single-Agent AI Loop) are complete. Phase 3 (Work Item DAG + Scheduler) is next.

---

## 2. Phase Summary

| Phase | Name                            | Goal                                                        | Status       | Key Deliverables                                                                                                                             |
| ----- | ------------------------------- | ----------------------------------------------------------- | ------------ | -------------------------------------------------------------------------------------------------------------------------------------------- |
| **1** | Terminal Foundation             | Working terminal client with daemon IPC                     | **COMPLETE** | Rust TUI, Rust daemon (replaced original C++ daemon), serde+bincode IPC, command fast-pass, tab completion, interactive passthrough |
| **2** | Single-Agent AI Loop            | Natural language input routes to AI agent with tool calling | **COMPLETE** | aisdk.rs integration, single agent loop, session timeline, built-in tools, markdown rendering, basic safety                                |
| **3** | Work Item DAG + Scheduler       | Complex tasks decomposed into concurrent Work Items         | Planned      | Work Item schema, DAG construction, Tokio-based scheduler, budget enforcement                                                                |
| **4** | Multi-Agent Roles + Multi-Model | Specialized agent roles with dynamic model selection        | Planned      | Role separation, model catalog, TeamLead model selection, Reviewer gating                                                                    |
| **5** | Memory + Project Context        | Persistent bounded memory across sessions and projects      | Planned      | SQLite episodic store, semantic markdown, Narrator consolidation, project auto-switching                                                     |
| **6** | Tool System                     | Unified tool registry with MCP bridge and prompt tools      | Planned      | Tool registry, MCP bridge, prompt tool runtime, progressive loading, hooks, aliases                                                          |
| **7** | Advanced Safety & Audit         | Production-grade safety, audit trail, compliance            | Planned      | Full audit trail, privacy config, OS-level sandboxing roadmap, OWASP/NIST alignment                                                          |

---

## 3. Phase Dependency Graph

```
Phase 1: Terminal Foundation  [COMPLETE]
    |
    v
Phase 2: Single-Agent AI Loop + Basic Safety  [COMPLETE]
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

**Note**: The C++ daemon originally built in Phase 1 has been fully replaced by a Rust daemon (`codivd`). See the [Rust daemon design doc](plans/2026-02-26-rust-daemon-aisdk-design.md) for the current architecture.

The foundation layer provides a working terminal client that executes commands via daemon IPC with near-zero overhead.

**What was built:**

- `codiv` binary (~4,300 lines of Rust) with ratatui 0.30 + crossterm 0.28 for linear scroll-down terminal flow
- `codivd` daemon (Rust, Tokio async) listening on Unix socket with serde+bincode IPC protocol (replaced original C++20 daemon)
- Persistent bash co-process via portable-pty with sentinel-based output boundary detection
- Interactive command passthrough (vim, ssh, python REPL) with dedicated PTY and raw terminal mode
- Command fast-pass: PATH scanning + 65 bash builtins in O(1) hash map with input classification (Execute, Interactive, AiQuery, NotFound, Clear, Reset, Exit, Empty)
- 3-tier tab completion: programmable (bash-completion integration) → command → file
- Env snapshot protocol: session_id, env_vars, PATH, cwd captured on connect, stored per-session in daemon
- Heartbeat mechanism (5s interval from client, 30s stale timeout in daemon)
- Worker bash sessions: spawn-on-demand per Work Item, initialized from env snapshot, killed on completion
- Structured logging with `--debug` flag to `/tmp/codiv-debug.log`

**Tech stack**: Rust (Cargo workspace) for both `codiv` and `codivd`, serde+bincode over Unix domain socket, portable-pty, tui-term + vt100, aisdk 0.5.2. See the [PRD Appendix A](PRD.md#appendix-a-phase-1-module-details) for full module tables.

---

### Phase 2: Single-Agent AI Loop — COMPLETE

**Goal**: Unknown/natural language input routes to a single AI agent that can reason and use tools.

#### What was built:

- **aisdk.rs integration with streaming LLM access** — aisdk 0.5.2 added as workspace dependency. Streaming works via `LanguageModelRequest::builder().model(model).messages(messages).build()` with SSE chunk forwarding over IPC. Supports Anthropic, OpenAI, and Google providers via `DynamicModel`.

- **Unified session timeline** — Shell commands, user queries, and AI responses stored as `Vec<SessionEvent>` in the `Agent` struct. Commands are embedded as assistant/user message pairs so the AI has full terminal context. See [Session Timeline Design](plans/2026-02-27-session-timeline-design.md).

- **Real-time CommandResult IPC** — Client sends `ClientMessage::CommandResult` to daemon after every shell command completes. Daemon stores in agent timeline. Output truncated to first/last 20 lines via `codiv-common::truncate`.

- **Agent persistence per session** — Agent stored in `ClientSession`, taken via `Option::take()` for async streaming, returned via `oneshot` channel, polled by `collect_returned_agents()`.

- **Tool calling loop** — Agent receives `AiQuery` input, reasons, calls tools, receives output, continues until final response. Agent state managed in `codivd` daemon per session.

- **Built-in tools: Bash, Read, Write, Edit, Glob, Grep** — All tools implemented in `codiv-tools` shared crate with CLI subcommands, `--help`, and `--agent-guide` support.

- **Streamdown markdown rendering with syntax highlighting** — Streaming markdown parser (streamdown-rs) with ANSI rendering and syntax highlighting, wired through tui-term/vt100 into ratatui.

- **Basic risk classification + confirmation prompts** — Commands classified by risk level. High/critical risk triggers confirmation prompt. Allowlist/denylist configurable in TOML config.

- **Env snapshot refresh on cd/source** — Selective env refresh on directory changes, auto-capture and send to `codivd`.

- **Agent output streaming with tool block headers** — Tool calls rendered with colored header lines. Bordered boxes descoped to later phase.

- **TOML config for API keys and model selection** — Config at `~/.codiv/config.toml` with hot-reload support, `[models]` and `[safety]` sections.

**Testable outcome**: User types "create a hello world program, compile it, and run it" — agent creates the file, compiles, executes, and streams the output. User types `ls` — still fast-passes. Agent attempting `rm -rf /` triggers a confirmation prompt.

**Tech**: aisdk.rs (streaming + tool calling), streamdown-rs for markdown rendering.

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

3. **Tokio-based concurrent execution**
   - Use Tokio task spawning in `codivd`
   - Map Work Item DAG to Tokio tasks with dependency tracking
   - Tokio runtime manages concurrent task execution
   - Each scheduled Work Item gets its own worker bash session

4. **State machine (pending → running → completed/failed)**
   - Track Work Item state transitions in daemon memory
   - Stream state updates to `codiv` for progress display
   - Handle failure: Work Item → `failed`, dependents → `blocked`

5. **Artifact storage in Shared Project State**
   - Each Work Item's outputs (files, diffs, stdout) stored as artifacts
   - Storage at `~/.codiv/state/`
   - Artifacts keyed by Work Item ID

6. **Budget enforcement (token + cost caps)**
   - Track token usage per Work Item (input + output tokens from LLM responses)
   - Track cost per Work Item (tokens × model cost_tier rate)
   - When budget exceeded: pause Work Item, log the overage, fail the item

**Testable outcome**: User types "add input validation to the user registration form, write tests, and update the README" — agent creates 3+ Work Items with correct dependencies, runs independent ones in parallel, then tests after validation is done.

**Tech**: Tokio tasks (async/await, concurrent Work Item execution via `futures::future::join_all`).

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

**Tech**: Multiple concurrent aisdk.rs sessions, role-based system prompts, single-writer ownership (no reader-writer locks).

---

### Phase 5: Memory + Project Context

**Goal**: Persistent, bounded memory that survives sessions and auto-switches per project.

Architecture is defined in the [Memory System Design](design-docs/memory-system-design.md). This phase implements that design.

#### Sub-tasks

1. **SQLite episodic store**
   - Create `~/.codiv/memory/memory.db` with the episodes table schema
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
   - On switch: unload previous project.md, load new project.md, update `CODIV_PROJECT_FINGERPRINT` env var
   - In-flight Work Items continue with their original project context

**Testable outcome**: User completes a task in repo A ("always use pytest, not unittest"). User starts a new session in repo A — agent remembers the preference. User cd's to repo B — agent switches to repo B's context automatically.

**Tech**: SQLite (rusqlite), semantic markdown, Narrator LLM agent, project fingerprinting.

---

### Phase 6: Tool System

**Goal**: Unified tool system — registry, MCP bridge, prompt tool runtime, progressive loading, hooks, aliases.

Architecture is defined in the [Unified Tool Model Design](design-docs/unified-tool-model-design.md). This phase implements that design.

#### Sub-tasks

1. **Tool registry CLI**
   - `codiv install <tool>` / `codiv install mcp:<package>` / `codiv install ./path`
   - `codiv remove <tool>` / `codiv update [tool]` / `codiv search "query"`
   - `codiv tools search "query"` / `codiv tools list` / `codiv tools info <tool>`
   - `codiv import skill ./path` for converting Claude Code SKILL.md format

2. **CODIV_TOOLS_PATH discovery**
   - Scan directories: project-level (`.codiv/tools/`), user-level (`~/.codiv/tools/`), system-level (`/usr/local/share/codiv/tools/`)
   - Run `<tool> --help` for each discovered executable (cached, re-run on mtime change)
   - Build tool index: name → help text → optional agent-guide → location

3. **MCP bridge**
   - Thin wrapper executables that trancodiv CLI interface to MCP protocol
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

**Testable outcome**: `/commit` works. `codiv install ./my-tool` works. `codiv install mcp:@modelcontextprotocol/server-filesystem` makes the MCP server appear as a regular tool.

**Tech**: Tool registry, Rust MCP client (serde_json + tokio subprocess management), prompt tool synthesis, CODIV_TOOLS_PATH discovery.

---

### Phase 7: Advanced Safety & Audit

**Goal**: Production-grade safety controls, full audit trail, compliance alignment, and graceful degradation.

#### Sub-tasks

1. **Full audit trail**
   - Log every command, tool invocation, LLM call, output, diff, and decision to `~/.codiv/audit/`
   - Machine-parseable JSON lines format with timestamps, Work Item IDs, agent roles, model IDs
   - `codiv audit` command for session review and search
   - Configurable retention policy

2. **Privacy settings**
   - `[privacy]` section in config
   - Options: exclude command output from LLM context, redact env vars, mask file paths
   - Per-project overrides via `.codiv/config.toml` in repo root

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
| Rust daemon unit tests | `cargo test` (built-in) | IPC protocol, session management, worker execution, agent loop |
| Rust client unit tests | `cargo test` (built-in) | Input classification, command index, completion engine, IPC message building |
| Rust integration tests | `cargo test` (integration test modules) | End-to-end command execution, daemon connection, env snapshot round-trip |
| End-to-end tests | expect-style scripting (e.g., rexpect or custom) | Full user scenarios: launch `codiv`, type commands, verify output, test agent interactions |
| LLM interaction tests | Mock LLM server (record/replay) | Agent tool calling loop, Work Item creation, role delegation, error handling |

### CI/CD

- **GitHub Actions** with parallel jobs:
  - `cargo build --workspace` + `cargo test --workspace` + `cargo clippy --workspace` + `cargo fmt --check`
- Matrix: macOS (primary), Linux (secondary)
- LLM tests use recorded responses (no live API calls in CI)

### Documentation

- Design docs per phase in `docs/design-docs/`
- PRD kept up-to-date as phases complete
- Plan.md (this document) updated with status changes
- In-code documentation: Rustdoc for all crates (`codiv`, `codivd`)

### Code Quality

| Language | Linting | Formatting | Static Analysis |
| --- | --- | --- | --- |
| Rust (all crates) | `clippy` (deny warnings) | `rustfmt` | Built-in borrow checker + `clippy::pedantic` |

---

## 6. Open Questions / Decisions Needed

1. **aisdk.rs maturity and provider coverage**: Is the Rust AI SDK mature enough for production use across all target providers (Anthropic, OpenAI, Google)? Key risks: newer SDK with potential gaps in provider support. Mitigation: Rust ecosystem has strong HTTP/async primitives (reqwest + tokio) as fallback for direct HTTP client implementation for unsupported providers.

2. **Narrator model**: Is Gemini 2.5 Flash sufficient for the memory consolidation algorithm (topic matching, semantic merging, compression decisions), or does topic management need a more capable model?

3. **Topic granularity**: What is the minimum episode count before creating a new topic? Without a threshold, one-off tasks could cause topic explosion. Candidates: 3 episodes minimum, or require 2+ sessions contributing to the same tag cluster.

4. **Cross-project knowledge flow**: Should semantic knowledge ever flow from one project to another beyond `user.md`? Example: "this user always prefers X pattern" could be inferred from project-scoped data. Currently only `user.md` is cross-project.

5. **Vector DB trigger**: When should we evaluate adding a vector DB for memory retrieval? Proposed trigger: if keyword-based `memory_search` recall accuracy falls below 80% on a representative query set, investigate vector embeddings (e.g., SQLite + sqlite-vec extension).

6. **Recursive decomposition depth**: What is the practical limit for sub-TeamLead deployment? ADAPT research suggests 2-3 levels for most coding tasks. Should we enforce a hard limit or rely on budget caps to naturally bound depth?

---

*Codiv Agent Implementation Plan — 7 phases from terminal foundation to production-grade safety. Phase 1 and Phase 2 complete. Phase 3 (Work Item DAG + Scheduler) is next.*
