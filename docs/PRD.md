# Slate Agent — Product Requirements Document

**Version**: 1.1 **Date**: 2026-02-15 **Author**: Subhagato **Status**: Draft

---

## 1. Executive Summary

**Slate Agent** is a C++ terminal-native coding agent that replaces the traditional shell with an intelligent, multi-model AI assistant. It looks and behaves like a normal terminal but seamlessly switches between instant command execution and AI-powered task orchestration.

**Key differentiators:**

- **Zero-latency command fast-pass**: recognized shell commands execute immediately without AI round-trip — no competitor offers this
- **Multi-model orchestration**: different AI models assigned to different roles (planning, coding, review, research) based on task complexity — not locked to a single provider
- **Recursive agent hierarchy**: TeamLead can deploy sub-TeamLeads, enabling arbitrarily deep task decomposition for complex work — validated by Anthropic's research showing orchestrator+subagent patterns outperform single agents by 90.2%
- **Shared state over agent chat**: agents coordinate through explicit artifacts and task state, not implicit message passing; single-writer ownership ensures no two agents can corrupt shared state — a pattern validated by Cursor's failure with reader-writer locks (agents held locks too long, 20 agents degraded to throughput of 2-3)
- **C++ performance**: daemon architecture with worker thread pool for true concurrent execution

**Vision**: The terminal becomes the IDE — developers think in natural language, and Slate Agent decomposes, executes, reviews, and summarizes the work end-to-end.

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

| Tool | Type | Models | Agent Architecture | Memory | Command Fast-Pass | Pricing | Benchmark |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **Claude Code** (Anthropic) | CLI | Anthropic only | Recursive subagents (up to 7 parallel; unreleased Swarms) | Hierarchical CLAUDE.md + Auto Memory + Subagent Memory; compaction at \~95% capacity | Permission allowlist (closest analog) | $20-200/mo | 80.9% SWE-bench |
| **Codex CLI** (OpenAI) | Open-source Rust CLI | OpenAI family + Ollama local models | Flat agent (multi-agent via external Agents SDK + MCP) | AGENTS.md + session resume | No | $20-200/mo or API | N/A |
| **Aider** | Open-source Python CLI | Most model-agnostic (any LLM) | Flat; "architect mode" pairs 2 models | Git-centric (no cross-session persistence) | No | Free (pay LLM API) | 64% (architect mode) |
| **Cursor** | VS Code fork + CLI (Jan 2026) | 8+ models incl. proprietary Composer | Recursive subagents + background agents; Planner/Worker/Judge pattern | .cursorrules + codebase indexing (Turbopuffer vector DB, Merkle tree syncing) | N/A | $60-200/mo credits | N/A |
| **Cline** | Open-source VS Code ext + CLI | 40+ providers | Flat; Plan/Act modes | .clinerules + new_task handoff; human approval every action | No | Free (pay LLM API); Teams $20/mo | N/A |
| **Gemini CLI** (Google) | Open-source Node.js CLI | Gemini family; auto-routing Flash/Pro | ReAct loop + MCP | GEMINI.md + SaveMemory | No | Free tier (60 req/min, 1000/day) | 78% SWE-bench (Flash) |
| **Goose** (Block) | Open-source CLI | BYOK (any provider via MCP) | Native MCP; full workflow execution | Session-based | No | Free | N/A |
| **Amp** (Sourcegraph) | CLI + IDE | Multi-model | "Deep mode" extended reasoning; built-in code review agent | Session-based | No | Free ad-supported tier | N/A |
| **Warp AI** | Rust GPU-rendered terminal | Multi-model (OpenAI, Anthropic, Google) | "Full Terminal Control" — agent interacts with live processes | Session-based | No (terminal replacement, not agent) | Free tier + paid | N/A |
| **Devin** (Cognition) | Cloud VM (terminal+editor+browser) | Proprietary | Full autonomous environment | Cloud-persistent | N/A | $500/mo | N/A |
| **Slate Agent** | **C++ CLI** | **Any provider via catalog** | **Recursive tree (TeamLead/Engineer/Reviewer)** | **Bounded + Narrator-curated (global + per-project)** | **Yes (&lt;10ms overhead)** | **Free (pay LLM API)** | **TBD** |

**Key competitive insight**: No existing tool combines terminal-native command fast-pass with recursive multi-agent orchestration and multi-model support. Claude Code has the strongest agent architecture but is locked to Anthropic models. Aider and Cline have the broadest model support but flat agent architectures. Cursor pioneered multi-agent coding but is IDE-bound and learned hard lessons about coordination (see Section 6, FR-004).

---

## 3. Goals & Metrics

### Goals (SMART)

| Priority | Goal | Success Metric | Target |
| --- | --- | --- | --- |
| **P0** | Shell plugin + daemon IPC with streaming | End-to-end command execution via Unix socket | Phase 1 complete |
| **P0** | Command fast-pass with near-zero latency | Recognized commands execute in &lt;10ms overhead vs raw shell | Phase 1 complete |
| **P0** | Single-model agent loop (Orchestrator -&gt; Engineer -&gt; output) | Natural language task -&gt; file edits + test runs working | Phase 2 complete |
| **P0** | Work Item DAG with concurrent scheduling | Independent Work Items execute in parallel; dependencies enforced | Phase 3 complete |
| **P1** | Multi-model dynamic assignment | TeamLead selects model per Work Item from catalog | Phase 4 complete |
| **P1** | Bounded memory with Narrator curation | Context persists across sessions; stays within size caps | Phase 4 complete |
| **P1** | Reviewer gating on Work Item outputs | Artifacts pass review before being applied | Phase 4 complete |
| **P2** | Recursive TeamLead deployment | TeamLead can spawn sub-TeamLeads for complex decomposition | Phase 5+ |
| **P2** | Skill system (slash commands) | Users can define and invoke custom skills | Phase 5+ |

### Key Performance Indicators

| KPI | Baseline (no tool) | Target |
| --- | --- | --- |
| Command execution overhead | 0ms (raw shell) | &lt;10ms for fast-pass |
| Time-to-first-token (AI response) | N/A | &lt;500ms streaming |
| Work Item throughput (concurrent) | 1 (sequential) | 4+ parallel workers |
| Memory footprint (daemon) | N/A | &lt;50MB resident |
| Context persistence accuracy | 0% (no memory) | &gt;90% relevant recall |
| Multi-agent token overhead vs single agent | 1x | &lt;8x (industry avg is \~15x) |

---

## 4. Non-Goals (Explicit Boundaries)

The following are **not** in scope for the MVP or near-term roadmap:

- **GUI or IDE integration** — this is terminal-only by design
- **Fish shell support** — Zsh and Bash only for now
- **Voice agent integration** — text input only
- **MCP server hosting** — Slate Agent is an MCP client, not an MCP server (may add later)
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

---

## 6. Functional Requirements

### FR-001: Shell Plugin (Zsh/Bash)

- Intercept user input at the shell level
- Provide context to daemon: cwd, env allowlist, tty info, shell history, last exit code
- Stream output from daemon back to terminal
- Hook into shell autocomplete using daemon's command index
- Support both inline and multi-line input
- **Zsh implementation**: use `add-zsh-hook preexec` to capture commands, `precmd` for results; ZLE widget wrapping `accept-line` to intercept before execution
- **Bash implementation**: use bash-preexec library for equivalent hooks
- **Communication**: Unix domain socket at well-known path (see FR-002)
- **Pitfalls to avoid**: hook ordering conflicts (always use `add-zsh-hook`, never raw assignment); ZLE widget chaining (must call original widget after interception); no subprocesses in keystroke hooks (performance)

### FR-002: Daemon (Persistent C++ Process)

- Listen on Unix domain socket for shell plugin connections
- Manage lifecycle: start on first shell plugin connection, stay resident, graceful shutdown
- Handle multiple concurrent shell plugin sessions
- Provide streaming responses (token-by-token for AI, chunked for command output)
- **IPC protocol**: `SOCK_STREAM` with newline-delimited JSON; each chunk: `{type: "text_delta"|"tool_call"|"error"|"done", content, metadata}`
- **I/O multiplexing**: kqueue (macOS) / epoll (Linux) for non-blocking I/O
- Consider Flow-IPC library for production-grade IPC if custom implementation proves fragile

### FR-003: Command Fast-Pass

- Build command index on startup from: PATH executables, shell builtins, aliases, functions
- Store in hash map: command name -&gt; type + path/builtin + completion hints
- Classify input: recognized command -&gt; execute immediately; unknown -&gt; route to agent mode
- Update index incrementally when PATH or aliases change
- Provide completion hints to shell plugin for autocomplete

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

**Lessons from Cursor's multi-agent failures**: Cursor initially used reader-writer locks for multi-agent file coordination, but this failed in practice. Agents held locks too long, causing 20 concurrent agents to degrade to throughput of 2-3. They switched to a role-based Planner/Worker/Judge pattern — which validates Slate Agent's TeamLead/Engineer/Reviewer split.

**Coordination model**: Shared state over message passing. All coordination through Shared Project State reads/writes (FR-008), not inter-agent chat. This aligns with the blackboard architecture (Hayes-Roth 1985, revived for LLM agents) and AWS's Arbiter pattern (shared semantic blackboard where agents read/write task state). Claude Code uses a similar file-based artifact model (progress files + git history).

**Single-writer ownership**: One agent owns writes to a given resource at a time. Multiple agents can read concurrently. This is the emerging consensus across production systems and avoids the lock contention that Cursor encountered.

**Task decomposition strategy**: Follow ADAPT (ACL 2024) — demand-driven decomposition. Only decompose a task into subtasks when the assigned agent fails to execute it directly. Practical depth is 2-3 levels for most coding tasks. Avoid upfront deep decomposition that wastes tokens on tasks that could be handled atomically.

**Cost awareness**: Multi-agent systems use approximately 15x more tokens than single-agent chat (industry data). Budget fields on Work Items and cost_tier in the model catalog are critical controls from day one.

### FR-005: Work Item System

- Each Work Item contains: goal, acceptance criteria, inputs/constraints, dependencies, outputs, assigned role, model_id, risk level, budgets (token + cost caps)
- Work Items form a DAG (directed acyclic graph)
- Scheduler runs Work Items concurrently via worker thread pool, respecting dependencies
- Work Item states: pending -&gt; running -&gt; completed | failed | blocked
- Outputs are stored as artifacts in Shared Project State
- Budget enforcement: Work Items that exceed token or cost budget are paused and escalated to TeamLead

### FR-006: Tool System

- `Bash`: shell command execution with timeout, background support
- `Read`: file content retrieval with line range support
- `Write`: file creation/overwrite (requires prior read)
- `Edit`: exact string replacement in files
- `Glob`: file pattern matching
- `Grep`: content search (ripgrep-style)
- `WebFetch`: URL content retrieval
- `Task`: sub-agent spawning for complex subtasks

### FR-007: Memory System

- **Global memory**: user preferences, habits, cross-project patterns (size-capped, 64KB)
- **Project memory**: per-repo context, conventions, decisions (size-capped, 128KB per project)
- Narrator curates both aggressively to stay within bounds
- Orchestrator auto-switches project context based on: cwd, git remote, repo fingerprint, file references

**Compression strategy (MemGPT/Letta-informed)**:

The Narrator should implement MemGPT-style bounded memory management:

- **Cognitive triage**: The Narrator LLM evaluates the future value of each piece of information before deciding what to keep, compress, or evict. Approximately 70% of conversational messages should be evicted to maintain continuity on long tasks.
- **Recursive summarization**: When memory approaches capacity, older entries are recursively summarized into increasingly compressed forms. Core memory blocks default to \~2,000 characters each (MemGPT convention), though Slate Agent's 64KB/128KB caps are appropriate for a coding context.
- **Preservation priorities**: Architectural decisions, unresolved bugs, user-stated preferences, and project conventions are high-value and should resist eviction. This mirrors Claude Code's compaction behavior (preserves architectural decisions, keeps 5 most recently accessed files, compacts at \~95% of \~33,000-token buffer).
- **No vector DB for MVP**: Unlike Cursor (Turbopuffer vector DB + Merkle trees for re-indexing), Slate Agent's bounded text memory with Narrator curation is simpler and sufficient for the terminal-native use case. Vector-based RAG can be added later if recall accuracy falls below target.

### FR-008: Shared Project State

- **Artifacts**: command transcripts, stdout/stderr, diffs, files, benchmarks
- **Decisions**: short rationale for key choices made during execution
- **Task state**: Work Item status + pointers to artifacts
- Stored at `~/.slate-agent/` or XDG-compliant location
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

- Phase 6 MVP: confirmation prompts + allowlists (software controls)
- Future: OS-level sandboxing using Linux bubblewrap or macOS seatbelt profiles, covering all spawned scripts and subprocesses. Claude Code's implementation reduced permission prompts by 84%.
- Docker-based sandboxing as an advanced option: dedicated microVM with own Docker daemon, running the agent with full permissions inside (safe because isolated).

**Autonomy-Capability tradeoff**: High-functionality agents (broad tool access) require constrained autonomy (human approval). High-autonomy agents (fewer prompts) require sandboxed functionality (OS-level isolation). Slate Agent starts with the first model and moves toward the second as sandboxing matures.

**Standards alignment**: Design with OWASP LLM Top 10 in mind (especially LLM06: Excessive Agency) and NIST AI Risk Management Framework.

### FR-010: Multi-Model Configuration

- Model catalog in TOML config: id, provider, cost_tier, intelligence level, strengths, weaknesses, best_for
- Fixed role assignments: Orchestrator, Narrator, TeamLead each pinned to a specific model
- Dynamic worker assignment: TeamLead selects model per Work Item based on complexity/capability/cost
- User override: `[roles.default_worker]` to force a single model for all workers

### FR-011: Skill System

- **Built-in skills** are first-class, hardcoded capabilities — not plugins: `/commit`, `/plan`, `/tasks`, `/help`, `/history`
  - Compiled into the daemon binary; always available regardless of plugin state
  - Cannot be overridden or disabled by plugins
- **User-defined skills** via `~/.slate-agent/skills/` (user-level) or `.slate-agent/skills/` (project-level)
  - Use **SKILL.md** format with YAML frontmatter (`name`, `description`, `allowed-tools`, `model`)
  - Can be **auto-invoked** by the agent (LLM-based matching on description) or **manually** via `/skill-name`
  - Expand to full prompts injected into agent context
- **Plugin-provided skills**: Plugins (FR-012) can also ship skills — these follow the same SKILL.md format and are discovered alongside user-defined skills
- **Discovery order**: Built-in &gt; Project-level &gt; User-level &gt; Plugin-installed

### FR-012: Plugin System

- **Plugin** is the top-level distribution unit that bundles one or more of: Commands, Skills, MCP Servers, Subagents, Hooks

- A plugin can also expose just a single subcommand

- **Directory structure** (following Claude Code pattern):

  ```
  my-plugin/
  ├── plugin.toml           # Required manifest (name, version, description, author, components)
  ├── commands/              # Optional slash commands (*.md files)
  ├── skills/                # Optional skills (subdirectories with SKILL.md)
  │   └── skill-name/
  │       └── SKILL.md
  ├── agents/                # Optional subagent definitions (*.md)
  ├── mcp.toml               # Optional MCP server declarations
  └── hooks.toml             # Optional lifecycle hooks
  ```

- **Discovery hierarchy** (project overrides user):

  1. Project-level: `.slate-agent/plugins/`
  2. User-level: `~/.slate-agent/plugins/`
  3. Installed plugins: `~/.slate-agent/plugins/installed/`

- **Plugin manifest** (`plugin.toml`):

  ```toml
  [plugin]
  name = "my-plugin"
  version = "1.0.0"
  description = "What it does"
  author = "Author Name"
  
  [components]
  commands = "./commands"       # optional
  skills = "./skills"           # optional
  agents = "./agents"           # optional
  mcp = "./mcp.toml"            # optional
  hooks = "./hooks.toml"        # optional
  ```

- **Progressive disclosure**: Only plugin metadata loaded at startup; full skill/command content loaded on invocation

- **MCP integration**: Plugins can declare MCP servers that the daemon starts and manages as child processes; communication via stdio transport (JSON-RPC 2.0 over stdin/stdout with newline-delimited messages)

- **Commands**: Markdown files with optional YAML frontmatter (description, argument-hint); invoked via `/command-name`; support positional args ($1, $2)

- **Skills**: Directory-based with SKILL.md; can bundle supporting files; auto-invoked by agent based on description match OR manually via `/skill-name`

- **Hooks**: Lifecycle events (SessionStart, PreToolUse, PostToolUse, Stop, PreCompact, SessionEnd); shell commands executed at each event

- **Note**: Built-in skills (commit, plan, tasks, help, history) are NOT plugins — they are hardcoded in the daemon (see FR-011). Plugins extend the system with additional capabilities.

### FR-013: MCP Client

- Native C++ MCP client for connecting to external MCP servers

- **Transport support**: stdio (subprocess spawning) and HTTP/SSE (remote servers)

- **Protocol**: JSON-RPC 2.0; supports `initialize`, `tools/list`, `tools/call`, `ping` lifecycle

- **Tool discovery**: Query connected MCP servers for available tools; tools surfaced to agent system alongside built-in tools (FR-006)

- **Lifecycle management**: Daemon spawns MCP server subprocesses, monitors health via `ping`, graceful shutdown (close stdin → SIGTERM → SIGKILL); process groups for cleanup

- **Configuration** (`mcp.toml`):

  ```toml
  [servers.filesystem]
  transport = "stdio"
  command = "npx"
  args = ["-y", "@modelcontextprotocol/server-filesystem", "/path"]
  
  [servers.remote-api]
  transport = "http"
  url = "https://api.example.com/mcp"
  headers = { Authorization = "Bearer ${API_KEY}" }
  ```

- **C++ implementation**: Use cpp-mcp library or implement minimal client using nlohmann/json + subprocess management

- **Lazy loading**: Don't load all tool schemas upfront; discover on-demand to minimize context token usage

---

## 7. Implementation Phases

Each phase produces a **fully functional, manually testable** deliverable. Later phases build incrementally on earlier ones.

---

### Phase 1: Shell Foundation

**Goal**: A working shell replacement that can execute commands via daemon IPC.

**Deliverables**:

- Zsh plugin that intercepts input and forwards to daemon via Unix domain socket
- C++ daemon that listens on Unix socket, receives commands, executes them, streams output back
- Command index built from PATH + builtins (hash map lookup)
- Fast-pass: recognized commands execute immediately through daemon
- Basic autocomplete from command index
- Graceful daemon lifecycle (start, stay resident, shutdown)

**Testable outcome**: User sources the shell plugin, types `ls`, `git status`, `make` — commands execute with near-zero overhead. Unknown commands print "not found" or similar.

**Dependencies**: None (greenfield)

**Tech**:

- C++20, CMake
- Unix domain sockets (`sys/socket.h`) with `SOCK_STREAM` + newline-delimited JSON protocol
- kqueue (macOS) / epoll (Linux) for non-blocking I/O
- Zsh: `add-zsh-hook preexec/precmd` + ZLE widget wrapping `accept-line`
- No AI/LLM needed yet

---

### Phase 2: Single-Agent AI Loop

**Goal**: Unknown/natural language input routes to a single AI agent that can reason and use tools.

**Deliverables**:

- Integration with ai-sdk-cpp (ClickHouse) for streaming LLM access + tool calling
- Single agent (combined Orchestrator+Engineer role) that receives user input, reasons, calls tools
- Tool implementations: Bash, Read, Write, Edit, Glob, Grep
- Streaming token output back to terminal via shell plugin
- Basic TOML config for API keys and model selection
- Terminal markdown rendering: cmark-gfm for parsing + tree-sitter for syntax highlighting in code blocks; walk AST to emit ANSI escape codes; for streaming, maintain growing buffer and re-parse on significant updates

**Testable outcome**: User types "create a hello world C++ program, compile it, and run it" — agent creates the file, runs g++, executes the binary, and streams the output. User types `ls` — still fast-passes.

**Dependencies**: Phase 1

**Tech**:

- ai-sdk-cpp (streaming + tool calling for OpenAI + Anthropic endpoints)
- Single model (e.g., Claude Sonnet 4.5 for cost-effective development)
- cmark-gfm + tree-sitter for terminal rendering

---

### Phase 3: Work Item DAG + Scheduler

**Goal**: Agent decomposes complex tasks into a Work Item DAG and executes items concurrently.

**Deliverables**:

- Work Item data structure (goal, acceptance criteria, dependencies, outputs, state, token/cost budgets)
- DAG construction: agent produces a plan as a set of Work Items with dependency edges
- Scheduler: Taskflow-based concurrent execution respecting dependencies
- Work Item state machine: pending -&gt; running -&gt; completed | failed
- Artifact storage: each Work Item's output stored in Shared Project State
- User-visible progress: streaming status of Work Items as they execute
- Budget enforcement: token and cost caps per Work Item

**Testable outcome**: User types "add input validation to the user registration form, write tests, and update the README" — agent creates 3+ Work Items, runs independent ones in parallel (validation + README), then tests after validation is done. User sees progress updates for each.

**Dependencies**: Phase 2

**Tech**:

- **Taskflow** library: header-only C++20, work-stealing scheduler, conditional tasking, composable sub-taskflows, built-in profiler. Use `tf::Executor` + `tf::Taskflow` with `precede()`/`succeed()` for dependency edges. Up to 29% faster than industrial schedulers on ML workloads.

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

**Tech**:

- Multiple concurrent ai-sdk-cpp sessions with different models
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
- Storage at `~/.slate-agent/memory/` (or XDG path)

**Testable outcome**: User completes a task in repo A ("always use pytest, not unittest"). User starts a new session in repo A — agent remembers the preference. User cd's to repo B — agent switches to repo B's context automatically.

**Dependencies**: Phase 4

---

### Phase 6: Plugins, Safety & Audit

**Goal**: Plugin architecture, MCP client integration, production-grade safety controls, audit trail, and UX polish.

**Deliverables**:

- Plugin architecture with manifest (`plugin.toml`), hierarchical discovery, and lazy loading
- MCP client with stdio transport for local MCP servers
- Built-in skills (commit, plan, tasks, help, history) hardcoded in daemon; not dependent on plugin system
- Plugin CLI commands: `slate plugin install/remove/list/enable/disable`
- Risk classification for commands and tool calls (low/medium/high/critical)
- Confirmation prompts for high/critical risk actions (see FR-009 for comprehensive list)
- Command/tool allowlist and denylist (configurable)
- Full audit trail: all commands, outputs, diffs, decisions logged to disk
- Privacy settings: configurable what gets sent to LLM
- Error handling and graceful degradation (API failures, model timeouts)
- Config validation and helpful error messages
- Alignment with OWASP LLM Top 10 (LLM06: Excessive Agency) and NIST AI RMF

**Testable outcome**: User types "rm -rf /" — agent flags as critical risk and requires confirmation. User types `/commit` — skill produces a commit message and stages changes. User installs a plugin via `slate plugin install ./my-plugin` and its commands become available. Audit log shows full history of session.

**Dependencies**: Phase 5

---

### Phase Dependency Graph

```
Phase 1 (Shell Foundation)
    |
    v
Phase 2 (Single-Agent AI Loop)
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
Phase 6 (Plugins, Safety & Audit)
```

Each phase is a **vertical slice** — fully functional and testable on its own.

---

## 8. Risks & Mitigations

| Risk | Likelihood | Impact | Mitigation |
| --- | --- | --- | --- |
| **ai-sdk-cpp maturity** — ClickHouse SDK exists (\~134 stars) and works for OpenAI + Anthropic with streaming + tool calling, but Google/Cohere not yet supported. C++20 with patched nlohmann/json. | Medium | High | Evaluate SDK early in Phase 2. It is the most complete C++ LLM SDK available. For unsupported providers, extend with direct HTTP (libcurl + cpr + custom SSE parser). llama.cpp server also supports OpenAI-compatible + Anthropic Messages API as a local fallback. |
| **Shell plugin complexity** — hooking into Zsh/Bash input pipeline is fragile across versions | Medium | High | Start with Zsh only; use `add-zsh-hook` (never raw assignment); test ZLE widget chaining; test on macOS + common Linux distros. Bash via bash-preexec library. |
| **Multi-agent token costs** — multi-agent systems use \~15x more tokens than single-agent chat | High | High | Budget fields on every Work Item; cost_tier in model catalog; TeamLead considers cost in model selection; user-configurable spending limits; demand-driven decomposition (ADAPT) to avoid unnecessary subtask explosion |
| **Lock contention in multi-agent coordination** — Cursor's reader-writer locks failed; agents held locks too long, 20 agents degraded to throughput of 2-3 | Medium | High | Use single-writer ownership pattern instead of reader-writer locks. One agent owns writes to a resource; others read. Role-based separation (Planner/Worker/Judge) reduces contention by design. |
| **Multi-model latency** — orchestrating multiple LLM calls adds overhead | Medium | Medium | Keep fast-pass path completely AI-free; pipeline model calls where possible; cache model selections |
| **Memory bloat** — unbounded context accumulation | Low | Medium | Hard size caps enforced by Narrator; MemGPT-style cognitive triage with recursive summarization; \~70% eviction rate for conversational messages |
| **DAG scheduler complexity** — concurrent execution with dependencies is error-prone | Medium | Medium | Use Taskflow library (battle-tested, header-only C++20) instead of hand-rolling scheduler; composable sub-taskflows for recursive decomposition |
| **API cost overruns** — multi-model usage can be expensive | Medium | Low | Budget fields on Work Items; TeamLead considers cost_tier; user-configurable spending limits |
| **Security of executed commands** — agent could run destructive commands | Low | Critical | Risk classification + confirmation prompts (Phase 6); allowlists; audit trail; never auto-execute critical-risk commands; future OS-level sandboxing (bubblewrap/seatbelt) |
| **1-week MVP timeline** — ambitious scope for AI-assisted development | High | Medium | Phases are incremental — even Phase 1+2 alone is a useful product; deprioritize Phases 5-6 if needed |

---

## 9. Technical Decisions

| Decision | Choice | Rationale |
| --- | --- | --- |
| Language | C++20 | Performance-critical daemon; direct system call access; ai-sdk-cpp compatibility |
| Build system | CMake (primary), Bazel (optional) | CMake is more widely supported; Bazel for future monorepo needs |
| IPC | Unix domain socket (`SOCK_STREAM` + newline-delimited JSON) | Low-latency, well-supported, no network overhead; kqueue/epoll for non-blocking I/O. Consider Flow-IPC for production hardening. |
| Config format | TOML | Human-readable, well-supported in C++, good for nested config (model catalog) |
| LLM SDK | ai-sdk-cpp (ClickHouse) | \~134 stars, C++20, streaming + multi-step tool calling working for OpenAI + Anthropic. Google/Cohere planned. Most complete C++ LLM SDK available. Uses patched nlohmann/json. |
| LLM SDK fallback | Direct HTTP via libcurl + cpr + custom SSE parser | For providers not yet in ai-sdk-cpp. cpr ("C++ Requests") is a modern libcurl wrapper. llama.cpp server supports OpenAI-compatible + Anthropic Messages API for local models. |
| DAG scheduler | Taskflow | Header-only C++20, work-stealing scheduler, conditional tasking, composable sub-taskflows, built-in profiler. `tf::Executor` + `tf::Taskflow` with `precede()`/`succeed()`. Up to 29% faster than industrial systems. |
| Terminal rendering | cmark-gfm + tree-sitter | cmark-gfm (GitHub's CommonMark C impl) for markdown parsing; tree-sitter for syntax highlighting in code blocks. Walk AST, emit ANSI escape codes. Streaming: maintain growing buffer, re-parse on significant updates, diff rendered output. |
| Shell hooks | `add-zsh-hook` + ZLE widgets (Zsh); bash-preexec (Bash) | Well-documented, avoids hook ordering conflicts. ZLE widget wrapping `accept-line` for input interception. |
| Storage location | `~/.slate-agent/` | Simple, user-local, follows common CLI tool conventions |
| Agent coordination | Single-writer ownership (no reader-writer locks) | Cursor's lock-based approach failed at scale. One writer per resource, concurrent readers. Role separation reduces contention. |
| Memory architecture | Bounded text with Narrator curation (MemGPT-informed) | 64KB global + 128KB per project caps. Cognitive triage + recursive summarization. No vector DB for MVP (add later if needed). |
| Plugin format | TOML manifest + directory convention | Follows Claude Code pattern; TOML consistent with rest of config |
| MCP client | cpp-mcp or custom (nlohmann/json + subprocess) | JSON-RPC 2.0 over stdio; cpp-mcp is most complete C++ MCP library |
| Plugin discovery | Hierarchical (project &gt; user &gt; installed) | Project-level overrides enable per-repo customization |

---

## 10. Self-Score (100-Point Framework)

### AI-Specific Optimization (25 pts)

| Criterion | Score | Notes |
| --- | --- | --- |
| Multi-model strategy defined | 5/5 | Fixed roles + dynamic catalog + TeamLead selection; competitive landscape fully mapped |
| Token/cost management | 5/5 | Budget fields on Work Items, cost_tier in catalog, demand-driven decomposition, 15x overhead acknowledged and mitigated |
| Streaming/latency optimization | 5/5 | Fast-pass for commands, streaming for AI, concurrent execution |
| Tool calling architecture | 5/5 | Claude Code-style tools, well-defined interfaces |
| Context window management | 5/5 | Bounded memory + MemGPT-informed Narrator curation with cognitive triage + recursive summarization |
| **Subtotal** | **25/25** |  |

### Traditional PRD Core (25 pts)

| Criterion | Score | Notes |
| --- | --- | --- |
| Problem statement (quantified) | 4/5 | Pain points identified with segments; could add more market data |
| User personas (specific) | 5/5 | 3 distinct personas with clear pain/goal/success |
| Functional requirements (complete) | 5/5 | 13 FRs covering all major capabilities including plugin system and MCP client, with research-backed design rationale |
| Non-goals (explicit) | 5/5 | Clear boundaries, deferred features listed |
| Competitive analysis | 5/5 | Comprehensive 11-tool comparison with architecture, pricing, benchmarks, and lessons learned |
| **Subtotal** | **24/25** |  |

### Implementation Clarity (30 pts)

| Criterion | Score | Notes |
| --- | --- | --- |
| Phased delivery (dependency-ordered) | 6/6 | 6 phases, each a testable vertical slice, clear dependency chain |
| Acceptance criteria per phase | 5/6 | "Testable outcome" for each phase; could be more granular |
| Tech stack decisions justified | 6/6 | Expanded decision table with specific libraries, fallbacks, and research backing |
| Risk mitigations actionable | 6/6 | Specific mitigations with research evidence (Cursor lock failure, ADAPT decomposition, MemGPT compression) |
| Architecture clarity | 6/6 | Daemon + plugin + agent tree + DAG scheduler + coordination model well-defined with competitive validation |
| **Subtotal** | **29/30** |  |

### Completeness (20 pts)

| Criterion | Score | Notes |
| --- | --- | --- |
| Config/infrastructure defined | 5/5 | TOML config, storage location, model catalog, IPC protocol |
| Safety & security covered | 5/5 | Risk classification, comprehensive confirmation list, sandboxing roadmap, OWASP/NIST alignment |
| Edge cases considered | 5/5 | Shell hook pitfalls, lock contention, token cost explosion, SDK provider gaps, streaming render diffing |
| MVP vs future clearly separated | 5/5 | Deferred features list, phase boundaries, sandboxing progression |
| **Subtotal** | **20/20** |  |

### **Total: 98/100**

---

## 11. Research Sources

- Anthropic (2025). Multi-agent research system with orchestrator-subagent architecture. 90.2% improvement over single-agent Opus.
- Cursor engineering blog (2025-2026). Multi-agent coordination: reader-writer lock failure, Planner/Worker/Judge pattern.
- MemGPT / Letta (Packer et al., 2023). "MemGPT: Towards LLMs as Operating Systems." Cognitive triage, recursive summarization, bounded memory management.
- ADAPT (ACL 2024). Demand-driven task decomposition for LLM agents.
- Hayes-Roth (1985). Blackboard architecture for AI systems.
- AWS Arbiter pattern. Shared semantic blackboard for multi-agent coordination.
- ai-sdk-cpp (ClickHouse). github.com/ClickHouse/ai-sdk-cpp. C++20 LLM SDK.
- Taskflow (Huang et al.). github.com/taskflow/taskflow. Header-only C++20 parallel task programming.
- cmark-gfm (GitHub). github.com/github/cmark-gfm. CommonMark parsing with GitHub extensions.
- tree-sitter (GitHub). github.com/tree-sitter/tree-sitter. Incremental parsing for syntax highlighting.
- OWASP LLM Top 10 (2025). LLM06: Excessive Agency.
- NIST AI Risk Management Framework (AI RMF 1.0).
- Claude Code documentation (Anthropic, 2025). Sandboxing, compaction, subagent architecture.
- Claude Code plugin architecture (Anthropic, 2025-2026). Plugin manifests, skill system, command hierarchy, hook lifecycle.
- Model Context Protocol specification (Anthropic, 2024-2025). JSON-RPC 2.0 based tool integration protocol.
- cpp-mcp library. github.com/hkr04/cpp-mcp. C++ MCP client/server implementation.
- cpr library. github.com/libcpr/cpr. C++ HTTP requests library.

---

*PRD v1.1 for Slate Agent — a C++ terminal-native multi-model coding agent. Updated with competitive research, architecture validation, and technical implementation guidance.*