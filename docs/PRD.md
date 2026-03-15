# Codiv Agent — Product Requirements Document

**Version**: 2.0 **Date**: 2026-02-26 **Author**: Subhagato **Status**: Draft

---

## 1. Executive Summary

**Codiv Agent** is a Rust terminal-native coding agent (Rust client, Rust daemon) that replaces the traditional shell with an intelligent, multi-model AI assistant. It looks and behaves like a normal terminal but seamlessly switches between instant command execution and AI-powered task orchestration.

**Key differentiators:**

- **Zero-latency Command mode**: shell commands in Command mode execute immediately without AI round-trip — no competitor offers this
- **Multi-model orchestration**: different AI models assigned to different roles (planning, coding, review, research) based on task complexity — not locked to a single provider
- **Recursive agent hierarchy**: TeamLead can deploy sub-TeamLeads, enabling arbitrarily deep task decomposition for complex work — validated by Anthropic's research showing orchestrator+subagent patterns outperform single agents by 90.2%
- **Shared state over agent chat**: agents coordinate through explicit artifacts and task state, not implicit message passing; single-writer ownership ensures no two agents can corrupt shared state — a pattern validated by Cursor's failure with reader-writer locks (agents held locks too long, 20 agents degraded to throughput of 2-3)
- **Rust performance**: Rust client with ratatui TUI for memory-safe terminal handling; Rust daemon with Tokio for async concurrent execution

**Vision**: The terminal becomes the IDE — developers think in natural language, and Codiv Agent decomposes, executes, reviews, and summarizes the work end-to-end.

---

## 2. Problem Statement

### Primary Pain Points

| Pain Point | Impact | Affected Segment |
| --- | --- | --- |
| **Context switching** between terminal and AI interfaces breaks developer flow | 15-25% productivity loss per task switch (studies on cognitive switching costs) | All terminal-centric developers |
| **Single-model lock-in** in existing tools (Claude Code = Anthropic only, Codex CLI = OpenAI only) | Suboptimal results — no single model excels at everything | Power users, polyglot developers |
| **Flat agent architectures** in competitors — one agent does everything | Poor handling of complex multi-step tasks; no specialization | Developers working on large codebases |
| **No command-level intelligence** — existing tools treat every input as an AI query | Unnecessary latency on simple commands; wasted API costs | High-frequency terminal users |
| **Ephemeral context** — most tools lose memory between sessions | Repeated explanations, no learning from user patterns | Long-term project developers |

### Competitive Landscape

| Tool | Type | Models | Agent Architecture | Memory | Command Mode | Pricing | Benchmark |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **Claude Code** (Anthropic) | CLI | Anthropic only | Recursive subagents (up to 7 parallel; unreleased Swarms) | Hierarchical CLAUDE.md + Auto Memory + Subagent Memory; compaction at ~95% capacity | Permission allowlist (closest analog) | $20-200/mo | 80.9% SWE-bench |
| **Codex CLI** (OpenAI) | Open-source Rust CLI | OpenAI family + Ollama local models | Flat agent (multi-agent via external Agents SDK + MCP) | AGENTS.md + session resume | No | $20-200/mo or API | N/A |
| **Aider** | Open-source Python CLI | Most model-agnostic (any LLM) | Flat; "architect mode" pairs 2 models | Git-centric (no cross-session persistence) | No | Free (pay LLM API) | 64% (architect mode) |
| **Cursor** | VS Code fork + CLI (Jan 2026) | 8+ models incl. proprietary Composer | Recursive subagents + background agents; Planner/Worker/Judge pattern | .cursorrules + codebase indexing (Turbopuffer vector DB, Merkle tree syncing) | N/A | $60-200/mo credits | N/A |
| **Cline** | Open-source VS Code ext + CLI | 40+ providers | Flat; Plan/Act modes | .clinerules + new_task handoff; human approval every action | No | Free (pay LLM API); Teams $20/mo | N/A |
| **Gemini CLI** (Google) | Open-source Node.js CLI | Gemini family; auto-routing Flash/Pro | ReAct loop + MCP | GEMINI.md + SaveMemory | No | Free tier (60 req/min, 1000/day) | 78% SWE-bench (Flash) |
| **Goose** (Block) | Open-source CLI | BYOK (any provider via MCP) | Native MCP; full workflow execution | Session-based | No | Free | N/A |
| **Amp** (Sourcegraph) | CLI + IDE | Multi-model | "Deep mode" extended reasoning; built-in code review agent | Session-based | No | Free ad-supported tier | N/A |
| **Warp AI** | Rust GPU-rendered terminal | Multi-model (OpenAI, Anthropic, Google) | "Full Terminal Control" — agent interacts with live processes | Session-based | No (terminal replacement, not agent) | Free tier + paid | N/A |
| **Devin** (Cognition) | Cloud VM (terminal+editor+browser) | Proprietary | Full autonomous environment | Cloud-persistent | N/A | $500/mo | N/A |
| **Codiv Agent** | **Rust CLI** | **Any provider via catalog** | **Recursive tree (TeamLead/Engineer/Reviewer)** | **Bounded + Narrator-curated (global + per-project)** | **Yes (<10ms overhead)** | **Free (pay LLM API)** | **TBD** |

**Key competitive insight**: No existing tool combines terminal-native dual-mode input with recursive multi-agent orchestration and multi-model support. Claude Code has the strongest agent architecture but is locked to Anthropic models. Aider and Cline have the broadest model support but flat agent architectures. Cursor pioneered multi-agent coding but is IDE-bound and learned hard lessons about coordination (see Section 6, FR-004).

---

## 3. Goals & Metrics

### Goals (SMART)

| Priority | Goal | Success Metric | Target |
| --- | --- | --- | --- |
| **P0** | Codiv binary + codivd daemon IPC with streaming | End-to-end command execution via Unix socket | Phase 1 complete |
| **P0** | Direct shell execution with near-zero latency | Commands in Command mode execute in <10ms overhead vs raw shell | Phase 1 complete |
| **P0** | Single-model agent loop (Orchestrator → Engineer → output) | Natural language task → file edits + test runs working | Phase 2 complete |
| **P1** | Basic safety controls (risk classification, confirmation prompts) | Destructive commands require confirmation; no auto-execution of critical-risk commands | Phase 2 complete |
| **P0** | Work Item DAG with concurrent scheduling | Independent Work Items execute in parallel; dependencies enforced | Phase 3 complete |
| **P1** | Multi-model dynamic assignment | TeamLead selects model per Work Item from catalog | Phase 4 complete |
| **P1** | Bounded memory with Narrator curation | Context persists across sessions; stays within size caps | Phase 5 complete |
| **P1** | Reviewer gating on Work Item outputs | Artifacts pass review before being applied | Phase 4 complete |
| **P2** | Recursive TeamLead deployment | TeamLead can spawn sub-TeamLeads for complex decomposition | Phase 5+ |
| **P2** | Unified tool system (binary + prompt tools, registry, MCP bridge) | Users can install, create, and invoke tools; MCP servers bridged as tools | Phase 6 complete |

### Key Performance Indicators

| KPI | Baseline (no tool) | Target |
| --- | --- | --- |
| Command execution overhead | 0ms (raw shell) | <10ms for Command mode |
| Time-to-first-token (AI response) | N/A | <500ms streaming |
| Work Item throughput (concurrent) | 1 (sequential) | 4+ parallel workers |
| Memory footprint (daemon) | N/A | <50MB resident |
| Context persistence accuracy | 0% (no memory) | >90% relevant recall |
| Multi-agent token overhead vs single agent | 1x | <8x (industry avg is ~15x) |

---

## 4. Non-Goals (Explicit Boundaries)

The following are **not** in scope for the MVP or near-term roadmap:

- **GUI or IDE integration** — this is terminal-only by design
- **Fish/Zsh native mode** — codiv uses its own persistent bash co-process; native Fish/Zsh integration is not planned
- **Voice agent integration** — text input only
- **MCP server hosting** — Codiv Agent bridges MCP servers as tools (MCP client), but does not host/expose its own MCP server (may add later)
- **Multi-model forked work trees** — deferred until real usage patterns emerge
- **Backpressure system** — deferred
- **Scoped capability tokens** — deferred
- **Circuit breaker with automatic tier fallback** — deferred
- **Open-source release** — not initially; may open-source later
- **Windows support** — Unix-only (macOS/Linux)

---

## 5. User Personas

### Persona 1: Alex — Senior Backend Developer

- **Profile**: 8 years experience, lives in tmux/neovim, works on large C++ and Go codebases
- **Pain**: Uses Claude Code but frustrated by single-model limitations and latency on simple commands. Wants `ls`, `git status`, `make` to just run instantly, and complex refactoring to use the best available model.
- **Goal**: A single terminal that handles both shell commands and AI tasks without mode switching.
- **Success**: "I type `make test` and it runs immediately. I type 'refactor the auth module to use JWT' and it plans, executes across files, runs tests, and shows me the diff."

### Persona 2: Priya — Full-Stack Developer

- **Profile**: 4 years experience, works across TypeScript/Python/React projects, switches repos frequently
- **Pain**: Loses context when switching between projects. AI tools don't remember her preferences or project-specific patterns.
- **Goal**: An agent that auto-switches project context and remembers past decisions.
- **Success**: "I cd into a different repo and it already knows the test framework, build system, and coding conventions from last time."

### Persona 3: Marcus — DevOps / Platform Engineer

- **Profile**: 6 years experience, manages infrastructure, writes lots of shell scripts, does security reviews
- **Pain**: Needs an AI that can reason about security implications of commands and catch risky operations before execution.
- **Goal**: An agent with built-in safety gates and the ability to hire specialist roles (Security, Perf) when needed.
- **Success**: "When I ask it to modify the CI pipeline, it flags the security implications and asks for confirmation before running destructive commands."

### 5.5 User Stories with Acceptance Criteria

#### Command Mode & Shell Execution

**US-001**: As Alex (senior backend dev), I want to type `make test` in Command mode and have it execute instantly, so that my shell workflow has zero overhead.
- **AC**: Recognized commands execute in <10ms overhead. No network call to LLM. Command output renders inline with no border, indistinguishable from a normal terminal.

**US-002**: As Alex, I want to press Tab on an empty input to switch from Command mode to AI mode, so that I can ask the agent for help without leaving the terminal.
- **AC**: Tab on empty input toggles between AI mode (`>` gutter, cyan) and Command mode (`$` gutter, white). Input in AI mode is sent to the agent as a natural language query.

#### Agent Task Execution

**US-003**: As Alex, I want to type "refactor the auth module to use JWT" and have the agent decompose, execute, review, and present a diff, so that complex tasks are handled end-to-end.
- **AC**: Agent creates Work Items, executes tools (Edit, Bash, Read), Reviewer validates outputs, final diff shown. All intermediate output streams in color-coded agent blocks.

**US-004**: As Priya (full-stack dev), I want the agent to create a file, compile it, and run it when I describe a program in natural language, so that I can prototype quickly.
- **AC**: Agent uses Write tool to create the file, Bash tool to compile, Bash tool to run. Output streams inline. Exit code captured and reported.

#### Multi-Agent & Work Items

**US-005**: As Alex, I want complex tasks to be split into parallel Work Items that execute concurrently, so that large refactoring tasks complete faster.
- **AC**: Independent Work Items run in parallel (visible in DAG tree). Dependent Work Items wait. Progress updates stream to terminal per Work Item.

**US-006**: As Marcus (devops), I want the agent to hire a Security specialist role when my task involves infrastructure or credential changes, so that risky operations get expert scrutiny.
- **AC**: TeamLead detects security-relevant characteristics in the task, hires Security role, Security agent reviews before changes are applied. User sees Security agent's output in red-bordered block.

#### Memory & Context

**US-007**: As Priya, I want the agent to remember my project conventions across sessions, so that I don't repeat myself.
- **AC**: Memory persists across sessions. Preferences stated in session N are available in session N+1 without re-stating. Project context auto-switches on `cd`.

**US-008**: As Priya, I want the agent to automatically switch project context when I `cd` into a different repository, so that conventions from repo A don't bleed into repo B.
- **AC**: On `cd` to a different git root, agent loads the new project's `project.md` and unloads the previous one. Agent announces the switch.

#### Safety & Confirmation

**US-009**: As Marcus, I want destructive commands to require confirmation, so that the agent doesn't accidentally damage production.
- **AC**: Commands in the high/critical risk list (e.g., `rm -rf`, `git push --force`, `DROP TABLE`) trigger a confirmation prompt. No auto-execution. User must explicitly approve.

**US-010**: As Marcus, I want a full audit trail of every command the agent executes, so that I can review what happened during a session for compliance.
- **AC**: All commands, outputs, diffs, and decisions are logged to `~/.codiv/audit/`. Logs are machine-parseable and include timestamps, Work Item IDs, and agent roles.

#### Tool System

**US-011**: As Alex, I want to install a custom tool with `codiv install ./my-tool` and have it immediately available to the agent, so that I can extend the agent's capabilities.
- **AC**: After `codiv install`, the tool appears in `codiv tools list`. Agent sees it in Tier 0 context. Tool's `--help` and `--agent-guide` are accessible. No daemon restart required.

#### Multi-Model

**US-012**: As Alex, I want the TeamLead to assign different models to different Work Items based on task complexity, so that simple tasks use cheap/fast models and complex tasks use frontier models.
- **AC**: TeamLead selects model from catalog per Work Item. Model selection visible in Work Item metadata. Cost-tier and intelligence-level influence selection. User can override with `[roles.default_worker]` config.

---

## 6. Functional Requirements

### FR-001: Codiv Binary (Terminal Client)

- Standalone Rust binary (`codiv`) as the user-facing terminal client
- Uses **ratatui + crossterm** as the terminal UI framework — owns the entire terminal: input, output rendering, layout, and colors
- **Linear scroll-down flow** — everything scrolls down like a normal terminal (no fixed zones or panels):
  - Shell commands render inline with no border (normal terminal feel)
  - Agent tasks expand inline below the prompt as structured blocks:
    - **Task header** with goal
    - **DAG tree** (live-updating in-place with status indicators: ● running, ◐ in review, ○ pending, ✓ done)
    - **Color-bordered agent output blocks** (per active agent, auto-collapse on completion, expandable with Enter)
  - **Auto-scroll** — viewport always follows new content; scrolling up shows "↓ N new lines" indicator
- **Agent output blocks**: color-bordered per agent role (see color scheme below), showing Work Item ID and streaming tool output. Auto-collapse to single-line summary on completion; Enter to expand. Focus mode: `Ctrl+F` expands single agent block to full viewport.
- **Color scheme** for agent output blocks:
  | Role | Color | Usage |
  |------|-------|-------|
  | Engineer | Cyan | Block border + header; dimmed cyan for stdout |
  | Reviewer | Yellow | Block border + header; dimmed yellow for output |
  | Security | Red | Block border + header |
  | Researcher | Magenta | Block border + header |
  | Perf | Blue | Block border + header |
  | TeamLead | White/bold | Task tree entries |
  | User commands | Default terminal color | No border, no prefix |
  Multiple Engineers: cycle through cyan variants (cyan, bright cyan, teal) per Work Item ID.
- Spawns a **persistent bash co-process** (`bash --noediting --norc --noprofile -i`) at startup via **portable-pty**
- Commands piped to bash co-process stdin; output read from PTY with **sentinel-based boundary detection** (`cmd; __CODIV_EXIT=$?; echo "SENTINEL${__CODIV_EXIT}__"` to detect output boundaries and capture exit codes)
- Maintains shell state continuity: env vars, aliases, cwd persist across commands
- User command output renders inline with no border, no prefix (feels like normal terminal)
- **VT100 terminal emulation**: uses tui-term + vt100 crate for ANSI-preserved output rendering in the ratatui widget tree
- **Interactive command passthrough**: commands like vim, htop, ssh, python REPL detected — dedicated PTY with raw terminal mode for direct terminal control, resumes normal mode on exit
- **Signal forwarding**: Ctrl+C forwarded to bash co-process child process
- **10,000 line scrollback limit**
- **Mouse support**: scroll and click via crossterm mouse events
- **Clipboard integration**: copy/paste via arboard crate
- Provides context to daemon: cwd, env allowlist, tty info, shell history, last exit code
- Streams output from daemon back to terminal and renders in agent output blocks
- **Markdown rendering** (planned): comrak for CommonMark+GFM parsing + syntect for syntax highlighting in code blocks; emit ANSI escape codes; for streaming, maintain growing buffer and re-parse on significant updates
- **Communication**: Unix domain socket at well-known path (see FR-002)
- **Tab completion**: 3-tier system — programmable completions (bash-completion integration) → command completions → file completions
- **Structured logging**: `--debug` flag enables structured logging via log + env_logger to `/tmp/codiv-debug.log`

### FR-002: Daemon (Persistent Rust Process — `codivd`)

- Rust singleton daemon process (Tokio async runtime)
- Listen on Unix domain socket (`/tmp/codivd-{uid}.sock`) for codiv client connections
- PID file at `~/.codiv/codivd.pid`
- Manage lifecycle: start on first codiv client connection, stay resident, graceful shutdown
- Handle multiple concurrent codiv client sessions
- Provide streaming responses (token-by-token for AI, chunked for command output)
- **serde+bincode IPC protocol**: 4-byte big-endian length prefix + bincode payload; ClientMessage variants: AgentRequest (with SessionContext), EnvSnapshot, Confirmation, Heartbeat, Shutdown. DaemonMessage variants: AgentStreamChunk, AgentComplete, ConfirmationRequest, TaskTreeUpdate, Heartbeat, Error
- **Env snapshot protocol**: `codiv` captures an env snapshot on connect to `codivd` — session_id, env_vars, PATH, cwd. Stored per-session in `codivd` (each codiv client has its own snapshot).
- **Heartbeat mechanism**: 5-second interval from client, 30-second stale timeout in daemon for detecting disconnected clients
- **Worker bash sessions**: `codivd` spawns a **fresh bash process** per Work Item that needs shell execution. Each worker initialized from the latest env snapshot: env vars injected, cwd set. Worker killed when Work Item completes — no reuse, no stale state. Workers are non-interactive (pipe stdin/stdout/stderr). Bash startup is ~5-10ms — negligible vs LLM latency, no need for a warm pool. Multiple Work Items run their own bash processes concurrently (true parallelism).
- **Command execution routing**:
  - **User commands** → `codiv`'s persistent bash co-process (interactive, stateful)
  - **Agent Bash tool calls** → `codivd`'s worker bash sessions (parallel, env-snapshot-initialized)
  - **Bidirectional IPC** carries: env snapshots (codiv→codivd), streaming output from worker sessions + confirmation requests (codivd→codiv)
- **Daemon logging**: logs to `~/.codiv/codivd.log`
- **Async I/O**: Tokio runtime with async/await

### FR-003: Dual-Mode Input

- Build command index on startup in `codiv` binary from: PATH executables (scanning) + 65 bash builtins
- Store in **O(1) hash map**: command name → type + path/builtin
- **Dual-mode input**: AI mode (default, `>` gutter) and Command mode (`$` gutter) with Tab toggle on empty input
- In Command mode, input executes through the bash co-process; in AI mode, input is sent to the agent as a natural language query
- Provide completion hints via tab completion in `codiv` binary

### FR-004: Agent System

- **Orchestrator**: owns session, interprets user intent, delegates via Work Items, manages project context switching
- **Executor roles**:
  - **TeamLead**: decomposes tasks into Work Item DAG, sets acceptance criteria, selects models, optionally hires specialist roles
  - **Engineer**: executes Work Items (file edits, command runs, code generation)
  - **Reviewer**: validates Work Item outputs against acceptance criteria
  - **Researcher** (optional): investigates unfamiliar domains, reads docs
  - **Security** (optional): audits risky commands, secrets, supply-chain changes
  - **Perf** (optional): analyzes hot paths, benchmarks
- **Narrator**: compresses and curates memory, produces state snapshots

**Architecture rationale (research-validated)**:

The Orchestrator-Worker pattern is the dominant architecture across all production multi-agent systems (Microsoft Agent Framework, CrewAI, LangGraph, Anthropic's internal research system). Anthropic's multi-agent research system with Opus orchestrator + Sonnet subagents outperformed single-agent Opus by 90.2% on complex tasks.

**Lessons from Cursor's multi-agent failures**: Cursor initially used reader-writer locks for multi-agent file coordination, but this failed in practice. Agents held locks too long, causing 20 concurrent agents to degrade to throughput of 2-3. They switched to a role-based Planner/Worker/Judge pattern — which validates Codiv Agent's TeamLead/Engineer/Reviewer split.

**Coordination model**: Shared state over message passing. All coordination through Shared Project State reads/writes (FR-008), not inter-agent chat. This aligns with the blackboard architecture (Hayes-Roth 1985, revived for LLM agents) and AWS's Arbiter pattern (shared semantic blackboard where agents read/write task state). Claude Code uses a similar file-based artifact model (progress files + git history).

**Single-writer ownership**: One agent owns writes to a given resource at a time. Multiple agents can read concurrently. This is the emerging consensus across production systems and avoids the lock contention that Cursor encountered.

**Task decomposition strategy**: Follow ADAPT (ACL 2024) — demand-driven decomposition. Only decompose a task into subtasks when the assigned agent fails to execute it directly. Practical depth is 2-3 levels for most coding tasks. Avoid upfront deep decomposition that wastes tokens on tasks that could be handled atomically.

**Cost awareness**: Multi-agent systems use approximately 15x more tokens than single-agent chat (industry data). Budget fields on Work Items and cost_tier in the model catalog are critical controls from day one.

### FR-005: Work Item System

- Each Work Item contains: goal, acceptance criteria, inputs/constraints, dependencies, outputs, assigned role, model_id, risk level, budgets (token + cost caps)
- Work Items form a DAG (directed acyclic graph)
- Scheduler runs Work Items concurrently via worker thread pool, respecting dependencies
- Work Item states: pending → running → completed | failed | blocked
- Each running Work Item gets a **dedicated worker bash session** in `codivd` (spawned on demand, killed on completion)
- Workers initialized from the latest env snapshot with independent cwd — no cross-contamination between concurrent Work Items
- Worker output streamed to `codiv` and rendered in color-coded agent output blocks
- Outputs are stored as artifacts in Shared Project State
- Budget enforcement: Work Items that exceed token or cost budget are paused and escalated to TeamLead

### FR-006: Built-in Tools

The following tools are **built-in** — compiled into the daemon binary, always available regardless of installed tool packages:

- `Bash`: shell command execution via `codivd`'s worker bash sessions (parallel per Work Item, env-snapshot-initialized), with timeout, background support, output streamed to `codiv` for display
- `Read`: file content retrieval with line range support
- `Write`: file creation/overwrite (requires prior read)
- `Edit`: exact string replacement in files
- `Glob`: file pattern matching
- `Grep`: content search (ripgrep-style)
- `WebFetch`: URL content retrieval
- `Task`: sub-agent spawning for complex subtasks

External tools (binary and prompt) are discovered via `CODIV_TOOLS_PATH` and present the same interface to the agent (see FR-011, FR-012). All tools — built-in and external — expose `--help` and optionally `--agent-guide` for progressive loading (Tier 0 → Tier 2), so the agent interacts with them uniformly.

### FR-007: Memory System

- **Global memory**: user preferences, habits, cross-project patterns (size-capped, 64KB)
- **Project memory**: per-repo context, conventions, decisions (size-capped, 128KB per project)
- Narrator curates both aggressively to stay within bounds
- Orchestrator auto-switches project context based on: cwd, git remote, repo fingerprint, file references

**Compression strategy (MemGPT/Letta-informed)**:

The Narrator should implement MemGPT-style bounded memory management:

- **Cognitive triage**: The Narrator LLM evaluates the future value of each piece of information before deciding what to keep, compress, or evict. Approximately 70% of conversational messages should be evicted to maintain continuity on long tasks.
- **Recursive summarization**: When memory approaches capacity, older entries are recursively summarized into increasingly compressed forms. Core memory blocks default to ~2,000 characters each (MemGPT convention), though Codiv Agent's 64KB/128KB caps are appropriate for a coding context.
- **Preservation priorities**: Architectural decisions, unresolved bugs, user-stated preferences, and project conventions are high-value and should resist eviction. This mirrors Claude Code's compaction behavior (preserves architectural decisions, keeps 5 most recently accessed files, compacts at ~95% of ~33,000-token buffer).
- **No vector DB for MVP**: Unlike Cursor (Turbopuffer vector DB + Merkle trees for re-indexing), Codiv Agent's bounded text memory with Narrator curation is simpler and sufficient for the terminal-native use case. Vector-based RAG can be added later if recall accuracy falls below target.

### FR-008: Shared Project State

- **Artifacts**: command transcripts, stdout/stderr, diffs, files, benchmarks
- **Decisions**: short rationale for key choices made during execution
- **Task state**: Work Item status + pointers to artifacts
- Stored at `~/.codiv/` or XDG-compliant location
- **No agent-to-agent chat:** all coordination through state reads/writes
- **Single-writer ownership**: one agent owns writes to a given resource at a time; multiple agents can read concurrently — avoids the lock contention problems Cursor encountered with reader-writer locks

### FR-009: Safety & Audit

- Tool/command allowlist with risk classification (low/medium/high/critical)
- Confirmation prompts for destructive actions (see list below)
- Full audit trail: every command, output, diff, and decision logged
- Configurable privacy settings (e.g., don't send command output to LLM)

**Commands requiring confirmation** (industry consensus):

- Destructive file operations: `rm -rf`, `chmod 777`, recursive deletes
- Git destructive ops: `git push --force`, `git reset --hard`, `git clean -f`, `git branch -D`
- Database destructive ops: `DROP TABLE`, `DROP DATABASE`, `TRUNCATE`
- Credential/secret touching: modifying `.env`, `credentials.json`, SSH keys
- Production changes: deployment commands, infrastructure modifications
- Privilege escalation: `sudo`, `su`, `chown` to root
- External writes: network requests that modify remote state
- Irreversible operations: anything that cannot be undone

**Sandboxing strategy** (informed by Claude Code 2025 approach):

- Phase 2 MVP: basic risk classification + confirmation prompts + allowlists (software controls)
- Phase 7: OS-level sandboxing using Linux bubblewrap or macOS seatbelt profiles, covering all spawned scripts and subprocesses. Claude Code's implementation reduced permission prompts by 84%.
- Docker-based sandboxing as an advanced option: dedicated microVM with own Docker daemon, running the agent with full permissions inside (safe because isolated).

**Autonomy-Capability tradeoff**: High-functionality agents (broad tool access) require constrained autonomy (human approval). High-autonomy agents (fewer prompts) require sandboxed functionality (OS-level isolation). Codiv Agent starts with the first model and moves toward the second as sandboxing matures.

**Standards alignment**: Design with OWASP LLM Top 10 in mind (especially LLM06: Excessive Agency) and NIST AI Risk Management Framework.

### FR-010: Multi-Model Configuration

- Model catalog in TOML config: id, provider, cost_tier, intelligence level, strengths, weaknesses, best_for
- Fixed role assignments: Orchestrator, Narrator, TeamLead each pinned to a specific model
- Dynamic worker assignment: TeamLead selects model per Work Item based on complexity/capability/cost
- User override: `[roles.default_worker]` to force a single model for all workers

### FR-011: Unified Tool System (Skills absorbed into Tools)

*Note: This FR has been redesigned per the [Unified Tool Model](../design-docs/unified-tool-model-design.md). Skills are no longer a separate concept — they are prompt tools (tools without a binary).*

- **Built-in skills** remain first-class, hardcoded capabilities: `/commit`, `/plan`, `/tasks`, `/help`, `/history`
  - Compiled into the daemon binary; always available regardless of installed tools
  - Can be overridden by project-level or user-level aliases in config (see design doc Section 8)
- **Prompt tools** replace user-defined skills and plugin-provided skills
  - A prompt tool is a tool package with `tool.toml` (including a `[skill]` section) + `guide.md` (expertise prompt) — no binary required
  - The daemon synthesizes `--help` and `--agent-guide` from the manifest and guide file
  - Can be **auto-invoked** by the agent (relevance matching on `[tool] description`) or **manually** via `/tool-name`
  - Support bundled resources: `scripts/` (executable helpers), `references/` (on-demand docs), `assets/` (output files)
  - Support `[skill.inject]` for dynamic context injection (shell commands run at Tier 2 loading, subject to same safety controls as Bash tool)
  - Execution model: `inline` (inject into current context) or `fork` (sub-session via Task tool pattern)
- **Skill import**: see FR-012 (`codiv import skill`) for converting Claude Code SKILL.md format to Codiv tool packages

### FR-012: Tool Registry & Distribution

*Redesigned per the [Unified Tool Model](../design-docs/unified-tool-model-design.md). The original plugin bundle with 6 component types is replaced by the tool package primitive.*

- **Tool package** is the single distribution unit: `tool.toml` manifest + either a `bin/` directory (binary tool) or `guide.md` + bundled resources (prompt tool)
- **Registry CLI**:
  ```bash
  codiv install <tool>              # install from registry
  codiv install mcp:<package>       # install MCP server as a tool (bridge)
  codiv install ./path/to/tool      # install from local path
  codiv import skill ./path         # convert Claude Code skill to Codiv tool
  codiv remove <tool>               # uninstall
  codiv list                        # show installed tools
  codiv update [tool]               # update one or all
  codiv search "query"              # search registry (remote)
  codiv tools search "query"        # search installed tools (local)
  ```
- **Discovery via `CODIV_TOOLS_PATH`** (colon-separated, first match wins):
  1. Project-level: `.codiv/tools/`
  2. User-level: `~/.codiv/tools/`
  3. System-level: `/usr/local/share/codiv/tools/`
- **Hooks** are tool invocations bound to lifecycle events (not a separate concept):
  ```toml
  [[hooks]]
  event = "pre-commit"
  tool = "lint-staged"
  args = ["--config", ".lintstagedrc"]
  ```
  Available events: `session-start`, `session-end`, `pre-tool-use`, `post-tool-use`, `pre-commit`, `post-task`, `pre-compact`
- **Slash commands** are aliases to tool invocations (not a separate concept):
  ```toml
  [[aliases]]
  name = "commit"
  tool = "git-commit"
  args = ["run"]
  ```
  Prompt tools with `user-invocable = true` auto-register as slash commands without explicit alias configuration

### FR-013: MCP Bridge

*Redesigned per the [Unified Tool Model](../design-docs/unified-tool-model-design.md). MCP is an implementation detail — MCP servers appear as regular tools.*

- **Install**: `codiv install mcp:<package>` creates a thin wrapper executable in `CODIV_TOOLS_PATH` that trancodivs the standard tool interface (`--help`, `--agent-guide`, commands) to MCP protocol calls
- **Transparent to agent**: The agent sees MCP-bridged tools identically to native tools — no special handling
- **Transport support**: stdio (subprocess spawning) and HTTP/SSE (remote servers)
- **Protocol**: JSON-RPC 2.0; `--help` → MCP `tools/list`, commands → MCP `tools/call`, `--json-out` → passthrough
- **State management**: Daemon manages MCP server subprocesses — spawned lazily on first tool call, stay alive for session, health-checked via MCP `ping`, shut down with daemon
- **Configuration**:
  ```toml
  # ~/.codiv/config.toml
  [[tools.mcp]]
  name = "filesystem"
  package = "@modelcontextprotocol/server-filesystem"
  args = ["/Users/subhagato/Development"]
  transport = "stdio"

  [[tools.mcp]]
  name = "github"
  package = "@modelcontextprotocol/server-github"
  transport = "http"
  url = "https://api.github.com/mcp"
  env = { GITHUB_TOKEN = "${GITHUB_TOKEN}" }
  ```
- **Rust implementation**: custom client (serde_json + tokio subprocess management)

---

## 7. Implementation Phases

Each phase produces a **fully functional, manually testable** deliverable. Later phases build incrementally on earlier ones.

---

### Phase 1: Terminal Foundation ✅ COMPLETE

**Goal**: A working terminal client that can execute commands via daemon IPC.

**Status**: Complete. The `codiv` binary (~4,300 lines of Rust) and `codivd` daemon (~1,240 lines of C++20) are fully implemented and functional. *Note: The C++ daemon from Phase 1 is being replaced by a Rust daemon in Phase 2. See the [Rust Daemon Design Doc](design-docs/rust-daemon-rewrite-design.md) for the current architecture.*

**Delivered summary**:

- Rust TUI client (`codiv`) with ratatui + crossterm for linear scroll-down terminal flow
- Persistent bash co-process via portable-pty with sentinel-based output boundary detection
- Interactive command passthrough (vim, ssh, python REPL) with dedicated PTY and raw terminal mode
- C++ daemon (`codivd`) with Unix socket IPC, FlatBuffers protocol, streaming output
- Command fast-pass: PATH scanning + bash builtins in O(1) hash map, input classification routing
- 3-tier tab completion: programmable (bash-completion) → command → file
- Env snapshot protocol, heartbeat mechanism, worker bash sessions
- Structured logging with `--debug` flag

**Verified outcome**: User launches `codiv`, types `ls`, `git status`, `make` — commands execute via persistent bash co-process with near-zero overhead. Interactive commands work correctly with PTY passthrough. Tab completion works across all three tiers. Unknown commands are classified for future agent routing.

**Dependencies**: None (greenfield)

**Key tech choices**: Rust (Cargo) for `codiv`, C++20 (CMake) for `codivd`, FlatBuffers IPC over Unix socket, portable-pty for PTY management, tui-term + vt100 for terminal emulation.

See [Appendix A: Phase 1 Module Details](#appendix-a-phase-1-module-details) for full module tables and tech stack.

---

### Phase 2: Single-Agent AI Loop

**Goal**: Unknown/natural language input routes to a single AI agent that can reason and use tools.

**Deliverables**:

- Integration with aisdk.rs for streaming LLM access + tool calling
- Single agent (combined Orchestrator+Engineer role) that receives user input, reasons, calls tools
- Tool implementations: Bash, Read, Write, Edit, Glob, Grep
- Streaming token output back to terminal via codiv binary
- Basic TOML config for API keys and model selection
- Terminal markdown rendering: comrak for CommonMark+GFM parsing + syntect for syntax highlighting in code blocks; walk AST to emit ANSI escape codes; for streaming, maintain growing buffer and re-parse on significant updates
- **Basic safety controls**:
  - Basic risk classification for commands (low/medium/high/critical)
  - Confirmation prompts for destructive commands (`rm -rf`, `git push --force`, `DROP TABLE`, etc. — see FR-009 for full list)
  - Command allowlist/denylist (configurable via TOML config)
  - No agent auto-execution of critical-risk commands
- **Worker bash sessions in `codivd`**: spawn fresh bash per Work Item, initialize from env snapshot, kill on completion
- **Env snapshot refresh**: auto-refresh on `cd`/`source`/manual `codiv sync-env`
- **Color-coded output streaming**: worker bash output streamed to `codiv`, rendered in color-bordered agent output blocks per Work Item

**Testable outcome**: User types "create a hello world C++ program, compile it, and run it" — agent creates the file, runs g++, executes the binary, and streams the output. User types `ls` — still fast-passes. Agent attempting `rm -rf /` triggers a confirmation prompt.

**Dependencies**: Phase 1

**Backwards compatibility**: IPC protocol is rewritten from FlatBuffers to serde+bincode. ClientMessage variants: AgentRequest (with SessionContext), EnvSnapshot, Confirmation, Heartbeat, Shutdown. DaemonMessage variants: AgentStreamChunk, AgentComplete, ConfirmationRequest, TaskTreeUpdate, Heartbeat, Error. Phase 1 FlatBuffers protocol is fully replaced.

**Tech**:

- aisdk.rs (streaming + tool calling for OpenAI + Anthropic endpoints)
- Single model (e.g., Claude Sonnet 4.5 for cost-effective development)
- comrak + syntect for terminal rendering

---

### Phase 3: Work Item DAG + Scheduler

**Goal**: Agent decomposes complex tasks into a Work Item DAG and executes items concurrently.

**Deliverables**:

- Work Item data structure (goal, acceptance criteria, dependencies, outputs, state, token/cost budgets)
- DAG construction: agent produces a plan as a set of Work Items with dependency edges
- Scheduler: Tokio-based concurrent execution respecting dependencies
- Worker bash sessions serve as the execution substrate — each scheduled Work Item gets its own bash process via `codivd`
- Work Item state machine: pending → running → completed | failed
- Artifact storage: each Work Item's output stored in Shared Project State
- User-visible progress: streaming status of Work Items as they execute
- Budget enforcement: token and cost caps per Work Item

**Testable outcome**: User types "add input validation to the user registration form, write tests, and update the README" — agent creates 3+ Work Items, runs independent ones in parallel (validation + README), then tests after validation is done. User sees progress updates for each.

**Dependencies**: Phase 2

**Backwards compatibility**: Work Item schema is new (no prior schema to break). IPC protocol extends with Work Item status messages; existing Phase 2 agent messages unchanged. Daemon API extends with scheduler endpoints.

**Tech**:

- **Tokio tasks**: async task spawning with `tokio::spawn`, `futures::future::join_all` for concurrent Work Item execution. Dependency edges enforced via async/await sequencing and channel-based coordination.

---

### Phase 4: Multi-Agent Roles + Multi-Model

**Goal**: Separate agent roles (Orchestrator, TeamLead, Engineer, Reviewer, Narrator) with different models assigned per role.

**Deliverables**:

- Agent role separation:
  - **Orchestrator**: session management, intent classification, project context
  - **TeamLead**: task decomposition, Work Item DAG construction, model selection
  - **Engineer**: Work Item execution (tools + code generation)
  - **Reviewer**: validates outputs against acceptance criteria, creates fix Work Items
  - **Narrator**: memory compression and curation
- Model catalog in config (TOML): multiple models with metadata
- TeamLead dynamically selects model per Work Item based on catalog
- Fixed role assignments (Orchestrator = frontier, Narrator = fast/cheap, TeamLead = strong planner)
- Reviewer gating: Work Item outputs reviewed before being applied
- Optional role hiring: TeamLead triggers Security/Perf/Researcher based on task characteristics
- Single-writer ownership enforced: one agent writes to a resource at a time, others read

**Testable outcome**: User types "refactor the auth module to use JWT and audit it for security issues" — Orchestrator delegates to TeamLead, TeamLead creates Work Items and assigns Engineer (Sonnet) for refactoring + Security role (Opus) for audit. Reviewer validates. User sees which model handled which part.

**Dependencies**: Phase 3

**Backwards compatibility**: Config format extends with model catalog section; existing config keys preserved. Work Item schema gains `model_id` and `assigned_role` fields (additive). Single-agent mode from Phase 2 continues to work (Orchestrator+Engineer combined role is a valid degenerate case).

**Tech**:

- Multiple concurrent aisdk.rs sessions with different models
- Role-based system prompts
- Single-writer resource ownership (no reader-writer locks — learned from Cursor)

---

### Phase 5: Memory + Project Context

**Goal**: Persistent, bounded memory that survives sessions and auto-switches per project.

**Deliverables**:

- Global memory store: user preferences, habits, cross-project patterns (64KB cap)
- Project memory store: per-repo conventions, decisions, patterns (128KB cap per project)
- Narrator agent actively curates memory after each task using MemGPT-style cognitive triage (compress, prune, prioritize based on future value)
- Recursive summarization when memory approaches capacity
- Project auto-switching: Orchestrator detects project context from cwd, git remote, repo fingerprint
- Memory injected into Orchestrator's context at session start
- Storage at `~/.codiv/memory/` (or XDG path)

**Testable outcome**: User completes a task in repo A ("always use pytest, not unittest"). User starts a new session in repo A — agent remembers the preference. User cd's to repo B — agent switches to repo B's context automatically.

**Dependencies**: Phase 4

**Backwards compatibility**: Memory storage is entirely new (`~/.codiv/memory/`). No prior data to migrate. Config extends with `[memory]` section; existing config keys preserved.

---

### Phase 6: Tool System

**Goal**: Unified tool system — registry, MCP bridge, prompt tool runtime, progressive loading, hooks, aliases.

**Deliverables**:

- Tool registry with `codiv install/remove/update/search` and `CODIV_TOOLS_PATH` discovery
- MCP bridge: `codiv install mcp:<pkg>` wraps MCP servers as tools, daemon manages lifecycle
- Prompt tool runtime: daemon synthesizes `--help`/`--agent-guide` for tools without binaries; `guide.md` + bundled resources (`scripts/`, `references/`, `assets/`)
- `codiv import skill` for converting Claude Code skills to Codiv tool packages
- Built-in tools (commit, plan, tasks, help, history) hardcoded in daemon; always available
- Hooks as event-bound tool invocations; aliases as slash command mappings
- Progressive loading: Tier 0 (names) → Tier 1 (help) → Tier 2 (agent-guide) → Tier 3 (deep docs)

**Testable outcome**: User types `/commit` — built-in tool produces a commit message and stages changes. User runs `codiv install ./my-tool` and the tool becomes available. User runs `codiv install mcp:@modelcontextprotocol/server-filesystem` and the MCP server appears as a regular tool.

**Dependencies**: Phase 5

**Backwards compatibility**: Tool discovery is new (`CODIV_TOOLS_PATH`). Built-in tools (Bash, Read, Write, Edit, Glob, Grep) remain unchanged and always available. Config extends with `[[tools.mcp]]`, `[[hooks]]`, and `[[aliases]]` sections; existing config keys preserved.

---

### Phase 7: Advanced Safety & Audit

**Goal**: Production-grade safety controls, full audit trail, compliance alignment, and graceful degradation.

**Deliverables**:

- Full audit trail: all commands, outputs, diffs, decisions logged to disk
- Privacy settings: configurable what gets sent to LLM
- OS-level sandboxing roadmap (bubblewrap on Linux, seatbelt on macOS) covering all spawned scripts and subprocesses
- Alignment with OWASP LLM Top 10 (LLM06: Excessive Agency) and NIST AI RMF
- Error handling and graceful degradation (API failures, model timeouts)
- Config validation and helpful error messages

**Testable outcome**: Full session audit log on disk with every command, output, and decision. Privacy config excludes sensitive output from LLM context. Config with invalid model ID produces a clear error message pointing to the problem.

**Dependencies**: Phase 6

**Backwards compatibility**: Safety policies are additive — existing behavior preserved. Audit trail is new storage. Config extends with `[privacy]` and `[safety]` sections; existing config keys preserved. OS-level sandboxing is opt-in.

---

### Phase Dependency Graph

```
Phase 1 (Terminal Foundation) ✅ COMPLETE
    |
    v
Phase 2 (Single-Agent AI Loop + Basic Safety)
    |
    v
Phase 3 (Work Item DAG + Scheduler)
    |
    v
Phase 4 (Multi-Agent Roles + Multi-Model)
    |
    v
Phase 5 (Memory + Project Context)
    |
    v
Phase 6 (Tool System)
    |
    v
Phase 7 (Advanced Safety & Audit)
```

Each phase is a **vertical slice** — fully functional and testable on its own.

---

## 8. Risks & Mitigations

| Risk | Likelihood | Impact | Mitigation |
| --- | --- | --- | --- |
| **aisdk.rs maturity** — Rust LLM SDK from lazy-hq; supports streaming + tool calling for OpenAI + Anthropic. Provider coverage may lag behind the rapidly evolving LLM API landscape. | Medium | Medium | Evaluate SDK early in Phase 2. Rust ecosystem has strong HTTP and async primitives (reqwest, tokio) for extending provider support. For unsupported providers, extend with direct HTTP (reqwest + custom SSE parser). |
| **Multi-agent token costs** — multi-agent systems use ~15x more tokens than single-agent chat | High | High | Budget fields on every Work Item; cost_tier in model catalog; TeamLead considers cost in model selection; user-configurable spending limits; demand-driven decomposition (ADAPT) to avoid unnecessary subtask explosion |
| **Lock contention in multi-agent coordination** — Cursor's reader-writer locks failed; agents held locks too long, 20 agents degraded to throughput of 2-3 | Medium | High | Use single-writer ownership pattern instead of reader-writer locks. One agent owns writes to a resource; others read. Role-based separation (Planner/Worker/Judge) reduces contention by design. |
| **Multi-model latency** — orchestrating multiple LLM calls adds overhead | Medium | Medium | Keep fast-pass path completely AI-free; pipeline model calls where possible; cache model selections |
| **Memory bloat** — unbounded context accumulation | Low | Medium | Hard size caps enforced by Narrator; MemGPT-style cognitive triage with recursive summarization; ~70% eviction rate for conversational messages |
| **DAG scheduler complexity** — concurrent execution with dependencies is error-prone | Medium | Medium | Use Tokio task spawning with async/await and `futures::future::join_all` instead of hand-rolling scheduler; channel-based coordination for dependency edges |
| **API cost overruns** — multi-model usage can be expensive | Medium | Low | Budget fields on Work Items; TeamLead considers cost_tier; user-configurable spending limits |
| **Security of executed commands** — agent could run destructive commands | Low | Critical | Basic risk classification + confirmation prompts (Phase 2); allowlists; audit trail (Phase 7); never auto-execute critical-risk commands; future OS-level sandboxing (bubblewrap/seatbelt, Phase 7) |
| **Env snapshot drift** — snapshot captured at connect may diverge from codiv's actual environment if user modifies env outside of tracked operations (e.g., manual `export` in a subshell) | Low | Medium | Auto-refresh on `cd`/`source`; manual `codiv sync-env` command; snapshot includes timestamp for staleness detection |
| **Unified Rust build** — single language and build system simplifies CI/CD and contributor onboarding | Low | Low | Cargo workspace builds all crates with `cargo build --workspace`. Shared types via `codiv-common` crate eliminate schema generation. No cross-language complexity. |

---

## 9. Technical Decisions

| Decision | Choice | Rationale |
| --- | --- | --- |
| Client language | Rust (`codiv` binary) | ratatui ecosystem, memory safety for UI handling untrusted input, excellent cross-platform PTY support via portable-pty. |
| Daemon language | Rust (`codivd` daemon) | aisdk.rs compatibility, Tokio async runtime for concurrent execution, unified language with client. |
| Terminal UI framework | ratatui 0.30 + crossterm 0.28 | Most mature terminal UI ecosystem in Rust, proven by Zellij. Linear scroll-down flow with inline agent blocks — everything scrolls down like a normal terminal. |
| PTY management | portable-pty 0.9 | Cross-platform PTY abstraction (macOS + Linux), clean API for spawning and managing pseudo-terminal pairs, avoids platform-specific `forkpty()` calls. |
| VT100 emulation | tui-term 0.3 + vt100 0.16 | ANSI escape sequence parsing and rendering within ratatui's widget tree, preserving colors and formatting from command output. |
| Build system | Cargo workspace (all crates) | Unified build system — single `cargo build --workspace` for all crates. |
| IPC serialization | serde + bincode over Unix domain socket with 4-byte BE length prefix | Both sides are Rust — no need for cross-language serialization. Zero schema compiler, compile-time checked. |
| IPC schema | `codiv-common/src/messages.rs` with ClientMessage/DaemonMessage enums | Shared Rust crate — types checked at compile time across client and daemon. |
| Config format | TOML | Human-readable, well-supported in Rust, good for nested config (model catalog). |
| LLM SDK | aisdk.rs (lazy-hq) | Rust LLM SDK with streaming + multi-step tool calling for OpenAI + Anthropic. Native async/await integration with Tokio runtime. |
| LLM SDK fallback | Direct HTTP via reqwest + custom SSE parser | For providers not yet in aisdk.rs. reqwest is the standard Rust HTTP client with async support. llama.cpp server supports OpenAI-compatible + Anthropic Messages API for local models. |
| DAG scheduler | Tokio tasks | Tokio task spawning with async/await. `futures::future::join_all` for concurrent Work Item execution. |
| Terminal markdown rendering | comrak + syntect (planned) | comrak for CommonMark+GFM parsing, syntect for syntax highlighting (same engine as Sublime Text). Walk AST, emit ANSI escape codes. Streaming: maintain growing buffer, re-parse on significant updates, diff rendered output. |
| Command execution (user) | Persistent bash co-process via portable-pty | Maintains shell state (env, aliases, cwd) across commands. Sentinel-based output boundary detection. Interactive passthrough via dedicated PTY with raw terminal mode. |
| Command execution (agent) | Spawn-on-demand bash in `codivd` | Fresh bash process per Work Item, initialized from env snapshot (env vars, cwd), killed on completion. No pool management, no reuse, no stale state. ~5-10ms startup negligible vs LLM latency. |
| Tab completion | 3-tier: programmable → command → file | Programmable completions via bash-completion integration, command name completions from command index, file path completions as fallback. |
| Storage location | `~/.codiv/` | Simple, user-local, follows common CLI tool conventions. |
| Agent coordination | Single-writer ownership (no reader-writer locks) | Cursor's lock-based approach failed at scale. One writer per resource, concurrent readers. Role separation reduces contention. |
| Memory architecture | Bounded text with Narrator curation (MemGPT-informed) | 64KB global + 128KB per project caps. Cognitive triage + recursive summarization. No vector DB for MVP (add later if needed). |
| Tool package format | `tool.toml` manifest + binary or `guide.md` + bundled resources | Single primitive for both binary and prompt tools; TOML consistent with rest of config. |
| MCP bridge | Custom Rust client (serde_json + tokio subprocess) | JSON-RPC 2.0 over stdio; thin wrapper makes MCP servers appear as regular tools. |
| Tool discovery | `CODIV_TOOLS_PATH` (project > user > system) | First-match-wins, mirrors Unix `PATH` semantics; built-in tools always available regardless. |
| Testing | cargo test (built-in) | Unified testing across all crates. |

---

## 10. Data Models

### Work Item

| Field | Type | Description |
| --- | --- | --- |
| `id` | string (UUID) | Unique identifier |
| `goal` | string | Natural language description of what the Work Item must accomplish |
| `acceptance_criteria` | list\<string\> | Conditions that must be true for the Work Item to be considered complete |
| `dependencies` | list\<string\> | IDs of Work Items that must complete before this one can start |
| `state` | enum | `pending` \| `running` \| `completed` \| `failed` \| `blocked` |
| `assigned_role` | enum | `Engineer` \| `Reviewer` \| `Security` \| `Perf` \| `Researcher` |
| `model_id` | string | ID from the model catalog (selected by TeamLead) |
| `risk_level` | enum | `low` \| `medium` \| `high` \| `critical` |
| `token_budget` | integer | Maximum tokens this Work Item may consume across all LLM calls |
| `cost_budget` | float | Maximum cost in USD this Work Item may consume |
| `inputs` | object | `{ repo: string, cwd: string, tool_allowlist: list<string> }` |
| `outputs` | list\<string\> | Artifact IDs produced by this Work Item |
| `parent_work_item_id` | string \| null | ID of the parent Work Item (for recursive decomposition by sub-TeamLeads) |
| `created_at` | timestamp | ISO 8601 creation time |
| `completed_at` | timestamp \| null | ISO 8601 completion time |

### Env Snapshot

| Field | Type | Description |
| --- | --- | --- |
| `session_id` | string | Identifies the `codiv` client session |
| `env_vars` | map\<string, string\> | Captured environment variables (filtered by allowlist) |
| `path` | string | `PATH` value at capture time |
| `cwd` | string | Working directory at capture time |
| `timestamp` | timestamp | ISO 8601 capture time |

### Memory Entry (Episodic)

The full schema is defined in the [Memory System Design](design-docs/memory-system-design.md). Summary of the `episodes` table:

| Field | Type | Description |
| --- | --- | --- |
| `id` | integer | Auto-increment primary key |
| `session_id` | string | Session that produced this episode |
| `work_item_id` | string \| null | Associated Work Item (null for non-Work-Item events) |
| `scope` | string | `'user'` or `'project:<fingerprint>'` |
| `timestamp` | timestamp | ISO 8601 |
| `event_type` | string | `command_executed`, `file_modified`, `decision_made`, `error_encountered`, `error_resolved`, `pattern_observed`, `user_preference`, `task_completed`, `task_failed` |
| `summary` | string | One-line human-readable summary |
| `detail` | string \| null | Longer description when needed |
| `tags` | string | Comma-separated, lowercase |
| `files` | string | Comma-separated file paths involved |
| `outcome` | string \| null | `'success'` \| `'failure'` \| `'partial'` |
| `consolidated` | boolean | Whether the Narrator has processed this episode |
| `high_value` | boolean | Resists age-based pruning |
| `raw_context` | string \| null | Optional JSON blob for structured data |

Semantic memory is stored as markdown files (`user.md`, `project.md`, `topics/*.md`) — see the [Memory System Design](design-docs/memory-system-design.md) for format and size caps.

### Shared Project State

| Field | Type | Description |
| --- | --- | --- |
| **Artifacts** | | |
| `id` | string (UUID) | Unique identifier |
| `work_item_id` | string | Work Item that produced this artifact |
| `type` | enum | `file`, `diff`, `stdout`, `stderr`, `transcript`, `benchmark` |
| `path` | string | File path or storage key |
| `created_at` | timestamp | ISO 8601 |
| **Decisions** | | |
| `id` | string (UUID) | Unique identifier |
| `work_item_id` | string | Work Item during which this decision was made |
| `rationale` | string | Why this decision was made |
| `created_at` | timestamp | ISO 8601 |
| **Task State** | | |
| `work_item_id` | string | Work Item this state tracks |
| `status` | enum | Mirrors Work Item state |
| `artifact_ids` | list\<string\> | Artifacts produced so far |

---

### 10.5 Analytics & Instrumentation Requirements

All metrics are stored locally at `~/.codiv/metrics/` — no telemetry is sent externally.

| Metric | Granularity | Description |
| --- | --- | --- |
| Token usage | Per Work Item, per agent role, per model | Total input/output tokens consumed |
| Command execution latency | Per command | Time from input to first output byte; segmented by fast-pass vs agent-routed |
| Work Item throughput | Per session | Concurrent execution count, completion rate, failure rate |
| Memory utilization | Per project, per scope | Current size vs cap, compression ratio after Narrator passes |
| Session duration & command count | Per session | Wall-clock session time and total commands executed |
| Model selection distribution | Per session | Which models TeamLead selects most often; cost per model |
| Agent turn count | Per Work Item | Number of LLM turns needed to complete a Work Item |
| Tool invocation frequency | Per tool, per session | Which tools are called most often; average execution time |

Metrics are written as append-only structured JSON lines (`metrics.jsonl`) per session. A `codiv metrics` command provides summary reports.

---

## 11. Edge Cases & Error States

### Edge Cases

| Scenario | Behavior |
| --- | --- |
| **Daemon (`codivd`) crashes mid-task** | Work Items in `running` state transition to `failed`. The `codiv` client detects the crash via heartbeat timeout, notifies the user, and offers to restart the daemon or continue in standalone mode (fast-pass only, no agent). |
| **LLM API times out** | Retry with exponential backoff (3 attempts: 2s, 4s, 8s). If all retries fail, the Work Item transitions to `failed` and the TeamLead is notified for re-planning or model fallback. Token budget is debited for the failed attempt. |
| **Work Item exceeds token/cost budget** | The Work Item is paused immediately. TeamLead is notified and can re-plan (split into smaller items), request user approval for budget increase, or fail the Work Item. |
| **Two Work Items try to edit the same file** | Single-writer ownership prevents this by design. Only one agent holds write ownership of a file at a time. If a second Work Item needs the same file, it is blocked until the first completes and releases ownership. |
| **User's environment changes while agent is running** | Env snapshot is refreshed on `cd`, `source`, and `codiv sync-env`. Running Work Items continue with their original snapshot (they use their own bash process initialized at spawn time). New Work Items use the latest snapshot. |
| **Memory hits its size cap** | The Narrator compresses aggressively: merges related entries, removes low-value content, performs recursive summarization, and as a last resort evicts the oldest low-value entries. See the [Memory System Design](design-docs/memory-system-design.md) for retention policy. |
| **Command index is stale** | PATH is scanned on `codiv` startup. The user can force a rescan with `codiv rescan`. Commands installed mid-session are detected on the next tab completion attempt (incremental PATH check). |
| **Interactive program (vim) running when agent needs attention** | Agent work queues until the interactive session exits. The `codiv` client buffers agent output and displays a notification indicator. When the user exits the interactive program, queued agent output is rendered. |

### Error States

| Error | Detection | Recovery |
| --- | --- | --- |
| **Daemon dies** | `codiv` detects via heartbeat timeout (30s with no response) | Offers restart (auto-launch `codivd`) or standalone mode (fast-pass only). In-progress Work Items are lost and must be re-run. |
| **LLM times out** | HTTP response timeout or SSE stream stalls | Retry with exponential backoff (3 attempts). After 3 failures, fail the Work Item and notify TeamLead for re-planning or model substitution. |
| **Tool execution fails** | Non-zero exit code from tool | Work Item transitions to `failed`. Reviewer analyzes the failure and creates a fix Work Item, or escalates to TeamLead for re-planning. |
| **IPC connection lost** | Socket read/write returns error | `codiv` attempts reconnection with exponential backoff (3 attempts: 1s, 2s, 4s). User input is buffered during reconnection. If reconnection fails, falls back to standalone mode. |
| **Disk full** | Write syscall returns `ENOSPC` | Memory writes fail gracefully — agent continues with in-memory context only. Audit log writes are best-effort. User is warned that persistence is degraded. |
| **Invalid config** | TOML parse error or schema validation failure on startup | `codiv`/`codivd` prints a clear error message identifying the problematic key and expected format, then exits with non-zero status. |

---

## 12. Research Sources

- Anthropic (2025). Multi-agent research system with orchestrator-subagent architecture. 90.2% improvement over single-agent Opus.
- Cursor engineering blog (2025-2026). Multi-agent coordination: reader-writer lock failure, Planner/Worker/Judge pattern.
- MemGPT / Letta (Packer et al., 2023). "MemGPT: Towards LLMs as Operating Systems." Cognitive triage, recursive summarization, bounded memory management.
- ADAPT (ACL 2024). Demand-driven task decomposition for LLM agents.
- Hayes-Roth (1985). Blackboard architecture for AI systems.
- AWS Arbiter pattern. Shared semantic blackboard for multi-agent coordination.
- aisdk.rs (lazy-hq). github.com/lazy-hq/aisdk. Rust LLM SDK.
- tokio. github.com/tokio-rs/tokio. Async runtime for Rust.
- serde. github.com/serde-rs/serde. Serialization framework for Rust.
- bincode. github.com/bincode-org/bincode. Binary serialization for Rust.
- ratatui. github.com/ratatui/ratatui. Rust terminal UI framework.
- crossterm. github.com/crossterm-rs/crossterm. Cross-platform terminal manipulation for Rust.
- portable-pty. github.com/wez/wezterm/tree/main/pty. Cross-platform PTY abstraction from wezterm.
- vt100. github.com/doy/vt100-rust. VT100 terminal emulator for Rust.
- tui-term. Terminal widget for ratatui using vt100 backend.
- comrak. github.com/kivikakk/comrak. CommonMark+GFM parsing in Rust.
- syntect. github.com/trishume/syntect. Syntax highlighting in Rust (Sublime Text engine).
- OWASP LLM Top 10 (2025). LLM06: Excessive Agency.
- NIST AI Risk Management Framework (AI RMF 1.0).
- Claude Code documentation (Anthropic, 2025). Sandboxing, compaction, subagent architecture.
- Claude Code skill architecture (Anthropic, 2025-2026). Skill format, command hierarchy, hook lifecycle — informed unified tool model design.
- Model Context Protocol specification (Anthropic, 2024-2025). JSON-RPC 2.0 based tool integration protocol.
- reqwest. github.com/seanmonstar/reqwest. Async HTTP client for Rust.

---

---

## Appendix A: Phase 1 Module Details

> **Note**: The C++ daemon described below has been replaced by a Rust daemon in Phase 2. See the [Rust Daemon Design Doc](design-docs/rust-daemon-rewrite-design.md) for current architecture.

### Module structure (`codiv` binary, ~4,300 lines)

| Module | Responsibility | Lines |
| --- | --- | --- |
| `main.rs` | Entry point, signal handling, logging init | — |
| `app.rs` | App initialization, daemon connection | — |
| `shell/bash_coprocess.rs` | PTY management via portable-pty | ~600 |
| `shell/command_index.rs` | PATH scanning, input classification | ~250 |
| `shell/completion_engine.rs` | Tab completion with bash-completion | ~350 |
| `shell/interactive.rs` | Interactive session passthrough | ~200 |
| `ui/terminal.rs` | Main event loop, rendering, VT100 parsing | ~1000 |
| `ui/input.rs` | Line editor with history | ~250 |
| `ui/completion_popup.rs` | Tab completion popup UI | ~250 |
| `ui/selection.rs` | Text selection for clipboard | — |
| `ipc/client.rs` | Unix socket client to daemon | — |
| `ipc/daemon_launcher.rs` | Daemon discovery/startup | — |
| `ipc/messages.rs` | FlatBuffers message building | — |

### Module structure (`codivd` daemon, ~1,240 lines)

| Module | Responsibility |
| --- | --- |
| `main.cpp` | Entry point, daemonization, signal handlers |
| `daemon.h/.cpp` | CodivdDaemon class, message dispatch, session management |
| `types.h` | Constants, paths, version |
| `session.h` | ClientSession state tracking |
| `worker_process.h/.cpp` | Subprocess management for command execution |
| `ipc_protocol.h/.cpp` | FlatBuffers framing |
| `ipc/socket_server.h/.cpp` | Socket server |
| `ipc/event_loop.h/.cpp` | Event loop |

### Full delivered feature list

- `codiv` binary in **Rust** with **ratatui 0.30 + crossterm 0.28** for linear scroll-down terminal flow, input handling, history, and tab completion
- Persistent bash co-process (`bash --noediting --norc --noprofile -i`) spawned via **portable-pty 0.9** at startup
- Sentinel-based output boundary detection for command completion and exit code capture (`cmd; __CODIV_EXIT=$?; echo "SENTINEL${__CODIV_EXIT}__"`)
- Interactive command passthrough with dedicated PTY and raw terminal mode (vim, ssh, python REPL, etc.)
- Signal forwarding (Ctrl+C → bash child process)
- VT100 terminal emulation via **tui-term 0.3 + vt100 0.16** for ANSI-preserved output rendering
- 10,000 line scrollback limit
- Mouse support (scroll, click) and clipboard integration (arboard 3)
- `codivd` daemon (C++20) that listens on Unix socket (`/tmp/codivd-{uid}.sock`), receives commands, streams output back
- FlatBuffers IPC protocol (flatbuffers 24.12.23 for Rust, v24.3.25 for C++) with message types: ExecuteCommand, CommandOutput, CommandComplete, EnvSnapshot, Heartbeat, Shutdown, Error
- Command index built in `codiv` from PATH scanning + 65 bash builtins (O(1) hash map lookup)
- Input classification: Execute, Interactive, AiQuery, NotFound, Clear, Reset, Exit, Empty
- 3-tier tab completion: programmable completions (bash-completion integration) → command completions → file completions
- Env snapshot protocol: session_id, env_vars, path, cwd (captured on connect, stored per-session in daemon)
- Heartbeat mechanism (5s interval from client, 30s stale timeout in daemon)
- Worker bash sessions: spawn-on-demand per Work Item, initialized from env snapshot, killed on completion
- Graceful daemon lifecycle (start, stay resident, shutdown) with PID file at `~/.codiv/codivd.pid`
- Structured logging with `--debug` flag to `/tmp/codiv-debug.log` (log + env_logger); daemon logs to `~/.codiv/codivd.log`

### Tech stack

- **Rust** (Cargo) for `codiv`: ratatui 0.30, crossterm 0.28, portable-pty 0.9, vt100 0.16, tui-term 0.3, nix 0.29, arboard 3, log + env_logger
- **C++20** (CMake 3.20+) for `codivd` *(Phase 1 historical — replaced by Rust daemon in Phase 2)*: FlatBuffers v24.3.25, GoogleTest v1.14.0
- **IPC (Phase 1 historical)**: FlatBuffers over Unix domain socket (4-byte BE length prefix) — *replaced by serde+bincode in Phase 2*
- **IPC (current)**: serde + bincode over Unix domain socket (4-byte BE length prefix)
- **Schema (current)**: `codiv-common/src/messages.rs` (shared Rust crate)
- **Build (current)**: Cargo workspace (`cargo build --workspace`)
- No AI/LLM needed yet

---

*PRD v2.0 for Codiv Agent — a Rust terminal-native multi-model coding agent. Rust client (`codiv`) using ratatui + crossterm with portable-pty for PTY management, tui-term + vt100 for terminal emulation, 3-tier tab completion, and linear scroll-down terminal flow. Rust daemon (`codivd`) with serde+bincode IPC, Tokio async runtime, worker bash sessions for parallel agent command execution, heartbeat-based session management, and env snapshot protocol. Phase 1 (Terminal Foundation) complete. Future phases: agent system, Work Item DAG, multi-model orchestration, memory, unified tool system, and safety/audit.*
