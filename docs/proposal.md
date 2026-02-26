# Slate Agent — Technical Proposal

**Date**: 2026-02-26 | **Author**: Subhagato | **Status**: Draft | **Document type**: Proposal (approval/funding)

---

## 1. Executive Summary

**Slate Agent** is a terminal-native coding agent that replaces the traditional shell with an intelligent, multi-model AI assistant. Built as a dual-language system — a Rust TUI client (`slate`) and a C++ daemon (`slated`) — it looks and behaves like a normal terminal but seamlessly switches between instant command execution and AI-powered task orchestration.

The core insight is that most terminal interactions are simple commands that should execute instantly, while complex tasks benefit from a structured multi-agent system with specialized roles. Slate Agent bridges both: recognized commands run with near-zero latency through a command fast-pass, while natural language requests are decomposed and executed by a hierarchy of AI agents — each assigned a purpose-fit model based on the task at hand.

**Key differentiators:**

- **Command fast-pass (<10ms)**: Recognized shell commands execute immediately without any AI round-trip. No competitor offers this.
- **Multi-model orchestration**: Different AI models are assigned to different roles (planning, coding, review, research) based on task complexity — not locked to a single provider.
- **Recursive agent hierarchy**: An Orchestrator delegates to a TeamLead, who decomposes tasks into a Work Item DAG executed by Engineers and validated by Reviewers. Sub-TeamLeads enable arbitrarily deep decomposition for complex work.
- **Bounded memory with Narrator curation**: Global and per-project memories are size-capped and actively curated by a dedicated Narrator agent, with automatic project context switching.
- **Shared state over agent chat**: Agents coordinate through explicit artifacts and task state, not implicit message passing — a pattern validated by production multi-agent systems.

**Phase 1 (Terminal Foundation) is complete.** Development is entering Phase 2 (Single-Agent AI Loop).

---

## 2. Problem Statement

### The Developer's Terminal Problem

Developers live in the terminal. Yet when they need AI assistance, they must context-switch to a separate interface — a chat window, an IDE sidebar, or a web app. This switching cost is real: studies on cognitive switching estimate a 15-25% productivity loss per task switch.

Existing tools that attempt to solve this fall short in several ways:

| Pain Point | Impact |
| --- | --- |
| **Context switching** between terminal and AI interface | Flow disruption, lost productivity on every interaction |
| **Single-model lock-in** (Claude Code = Anthropic only, Codex CLI = OpenAI only) | No single model excels at every task; users lose access to the best model for each job |
| **Flat agent architectures** — one agent does everything | Poor decomposition of complex multi-step tasks; no specialization |
| **No command-level intelligence** — every input treated as an AI query | Unnecessary latency on `ls`, `git status`, `make`; wasted API costs |
| **Ephemeral context** — tools lose memory between sessions | Repeated explanations, no learning from user patterns or project conventions |

### Competitive Landscape

| Tool | Type | Models | Multi-Agent | Memory | Command Fast-Pass |
| --- | --- | --- | --- | --- | --- |
| **Claude Code** | CLI | Anthropic only | Recursive subagents | Hierarchical + compaction | No |
| **Codex CLI** | Rust CLI | OpenAI + local | Flat (external multi-agent) | Session-based | No |
| **Cursor** | IDE + CLI | 8+ models | Recursive (Planner/Worker/Judge) | Codebase indexing | N/A (IDE) |
| **Aider** | Python CLI | Most model-agnostic | Flat; architect mode (2 models) | Git-centric | No |
| **Gemini CLI** | Node.js CLI | Gemini family | ReAct loop + MCP | Session-based | No |
| **Warp AI** | Rust terminal | Multi-model | Agent with terminal control | Session-based | No |
| **Devin** | Cloud VM | Proprietary | Full autonomous environment | Cloud-persistent | N/A |
| **Slate Agent** | **Rust+C++ CLI** | **Any provider** | **Recursive tree** | **Bounded + Narrator-curated** | **Yes (<10ms)** |

**Key competitive insight**: No existing tool combines terminal-native command fast-pass with recursive multi-agent orchestration and multi-model support. Claude Code has the strongest agent architecture but is locked to one provider. Aider and Cline offer the broadest model support but have flat agent architectures. Cursor pioneered multi-agent coding but is IDE-bound.

---

## 3. Proposed Solution

### What We Are Building

Slate Agent is a shell replacement that functions as both a high-performance terminal and an intelligent coding assistant. The system has two components:

- **`slate`** (Rust binary, per-terminal): The user-facing TUI client. Owns the terminal experience — rendering, input, tab completion, command execution via a persistent bash co-process, and interactive program passthrough (vim, ssh, python REPL). Communicates with the daemon over IPC.

- **`slated`** (C++ daemon, singleton): The backend intelligence. Manages client sessions, spawns worker bash processes for agent tool calls, and hosts the agent system — orchestration, memory, task scheduling, and LLM access.

### Core Concepts

**Command Fast-Pass**: On startup, `slate` scans PATH directories and known bash builtins to build a command index. Every input is classified instantly: recognized commands execute immediately through the persistent bash co-process; natural language or ambiguous input routes to the agent system. This ensures the terminal never feels slow for everyday commands.

**Multi-Agent Hierarchy**: The agent system uses a tree of specialized roles:

- **Orchestrator** — owns the session, interprets user intent, delegates tasks, manages project context
- **TeamLead** — decomposes tasks into a Work Item DAG, sets acceptance criteria, selects models, hires specialist roles when needed
- **Engineer** — executes Work Items (file edits, command runs, code generation)
- **Reviewer** — validates outputs against acceptance criteria; approves or creates fix items
- **Narrator** — compresses and curates memory after each task
- **Optional specialists** — Security, Perf, Researcher — hired by TeamLead on demand

**Work Item DAG**: Every task the agent performs is represented as a Work Item with a goal, acceptance criteria, dependencies, outputs, and budgets. Work Items form a directed acyclic graph. Independent items execute concurrently; dependencies are enforced by the scheduler.

**Multi-Model Strategy**: Each agent role has different intelligence requirements. Rather than using a single model for everything, the system assigns purpose-fit models: a frontier model for the Orchestrator (infrequent, high-stakes decisions), a fast/cheap model for the Narrator (frequent, well-defined compression), and a strong planning model for the TeamLead. Worker agents (Engineers, Reviewers) receive models dynamically selected by the TeamLead based on task complexity, required capability, cost sensitivity, and latency needs. A simple file rename gets a cheap model; a security audit gets a frontier model.

**Bounded Memory**: All memory is size-capped. Global memory tracks user preferences and habits. Per-project memory tracks conventions, decisions, and patterns for each repository. The Narrator curates aggressively, using cognitive triage to decide what to keep, compress, or evict. The Orchestrator auto-switches project context when the user changes directories or repositories.

**Unified Tool System**: Agents interact with the codebase through a standard set of tools — Bash (shell execution), Read, Write, Edit, Glob, Grep, WebFetch, and Task (sub-agent spawning). External tools, MCP servers, and prompt-based tools all present the same interface to the agent.

**Shared State Coordination**: Agents do not chat with each other. All coordination happens through explicit shared state: artifacts (diffs, logs, test results), decisions (short rationale), and task state (Work Item status). Single-writer ownership ensures no two agents can corrupt shared state — a lesson learned from Cursor's experience with reader-writer locks degrading throughput.

---

## 4. High-Level Architecture

```
┌─────────────────────────────────────────────────────────┐
│                     User's Terminal                     │
│                                                         │
│  ┌───────────────────────────────────────────────────┐  │
│  │  slate (Rust, per-terminal)                       │  │
│  │                                                   │  │
│  │  TUI Rendering    Persistent Bash    Command      │  │
│  │  (ratatui)        Co-Process (PTY)   Index        │  │
│  │                                                   │  │
│  │  Tab Completion   Interactive        Clipboard    │  │
│  │  (3-tier)         Passthrough                     │  │
│  └──────────────────────┬────────────────────────────┘  │
│                         │ FlatBuffers IPC               │
│                         │ (Unix domain socket)          │
│  ┌──────────────────────▼────────────────────────────┐  │
│  │  slated (C++20, singleton daemon)                 │  │
│  │                                                   │  │
│  │  Session Manager         Worker Bash Sessions     │  │
│  │  (env snapshots,         (spawn per Work Item,    │  │
│  │   heartbeat)              parallel execution)     │  │
│  │                                                   │  │
│  │  Agent System            Memory & State           │  │
│  │  (Orchestrator,          (global + per-project,   │  │
│  │   TeamLead, Engineer,     Narrator-curated,       │  │
│  │   Reviewer, Narrator)     shared project state)   │  │
│  │                                                   │  │
│  │  Scheduler (DAG)         LLM Integration          │  │
│  │  (concurrent Work        (multi-provider,         │  │
│  │   Item execution)         streaming + tools)      │  │
│  └───────────────────────────────────────────────────┘  │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

The architecture enforces a clean separation: `slate` owns everything the user touches (rendering, input, command execution), while `slated` owns everything the agent does (orchestration, memory, scheduling, LLM access). They communicate over a binary IPC protocol on a Unix domain socket. This separation enables independent development, deployment, and language-appropriate optimizations for each component.

---

## 5. Deliverables by Phase

### Phase 1: Terminal Foundation -- COMPLETE

**Goal**: A working terminal client that executes commands via daemon IPC.

- `slate` Rust binary: TUI with persistent bash co-process, command index classifier, 3-tier tab completion, interactive program passthrough, VT100 terminal emulation, clipboard support
- `slated` C++ daemon: Unix socket server, IPC protocol, session management with env snapshots, worker bash process spawning, heartbeat detection
- Binary IPC protocol between `slate` and `slated`

### Phase 2: Single-Agent AI Loop

**Goal**: Natural language input routes to a single AI agent that can reason and use tools.

- LLM integration with streaming and tool calling
- Single agent (Orchestrator + Engineer combined) that receives input, reasons, and calls tools
- Built-in tool implementations: Bash, Read, Write, Edit, Glob, Grep
- Basic safety controls: risk classification, confirmation prompts for destructive commands
- Terminal markdown rendering for agent output

### Phase 3: Work Item DAG + Scheduler

**Goal**: Complex tasks decompose into a Work Item DAG and execute concurrently.

- Work Item data structure with goals, acceptance criteria, dependencies, budgets
- DAG construction and concurrent scheduler
- Artifact storage in shared project state
- Budget enforcement (token and cost caps per Work Item)

### Phase 4: Multi-Agent Roles + Multi-Model

**Goal**: Separate agent roles with different models assigned per role and per task.

- Role separation: Orchestrator, TeamLead, Engineer, Reviewer, Narrator
- Model catalog in config; TeamLead dynamically selects models per Work Item
- Reviewer gating on Work Item outputs
- Optional specialist roles (Security, Perf, Researcher) hired on demand

### Phase 5: Memory + Project Context

**Goal**: Persistent, bounded memory that survives sessions and auto-switches per project.

- Global memory store (user preferences, habits) and per-project memory stores
- Narrator agent curates memory with cognitive triage and recursive summarization
- Automatic project context switching based on cwd and repository fingerprint

### Phase 6: Tool System

**Goal**: Unified tool registry with MCP bridge, prompt tools, and extensibility.

- Tool registry with install/remove/update/search CLI
- MCP bridge: MCP servers appear as regular tools
- Prompt tools (expertise without a binary), hooks, and slash command aliases

### Phase 7: Advanced Safety + Audit

**Goal**: Production-grade safety, full audit trail, and compliance alignment.

- Complete audit trail of all commands, outputs, diffs, and decisions
- Privacy settings for what gets sent to LLMs
- OS-level sandboxing roadmap (Linux bubblewrap, macOS seatbelt)
- Alignment with OWASP LLM Top 10 and NIST AI Risk Management Framework

---

## 6. Timeline

| Phase | Duration (estimated) | Status |
| --- | --- | --- |
| Phase 1: Terminal Foundation | 6 weeks | **Complete** |
| Phase 2: Single-Agent AI Loop | 6-8 weeks | Starting now |
| Phase 3: Work Item DAG + Scheduler | 4-6 weeks | Planned |
| Phase 4: Multi-Agent Roles + Multi-Model | 6-8 weeks | Planned |
| Phase 5: Memory + Project Context | 4-6 weeks | Planned |
| Phase 6: Tool System | 6-8 weeks | Planned |
| Phase 7: Advanced Safety + Audit | 4-6 weeks | Planned |

**Total estimated timeline**: ~9-12 months from Phase 2 start to Phase 7 completion. Phases are sequential — each builds on the previous — but each phase produces a fully functional, testable deliverable.

---

## 7. Team & Ownership

**Developer**: Subhagato (solo developer)

Slate Agent is currently a solo project. All design, implementation, and testing across both the Rust client and C++ daemon are handled by a single developer.

**Future considerations**: As the project matures past Phase 4, specific phases may benefit from contributors — particularly Phase 6 (Tool System / MCP Bridge) and Phase 7 (Safety / Sandboxing), which involve well-scoped, relatively independent work that could be parallelized.

---

## 8. Budget / Cost Breakdown

### Development Costs (LLM API)

LLM API usage is the primary ongoing cost during development. Testing requires exercising multiple providers and models.

| Phase | Estimated Monthly API Cost | Notes |
| --- | --- | --- |
| Phase 2 (Single-Agent) | $50-80/month | Single model, iterating on tool calling and prompt engineering |
| Phase 3 (DAG + Scheduler) | $60-100/month | More complex task decomposition testing |
| Phase 4 (Multi-Agent + Multi-Model) | $100-150/month | Testing multiple providers (Anthropic, OpenAI, Google) concurrently |
| Phases 5-7 | $50-100/month | Lower LLM intensity; focus on memory, tools, and safety |

**Estimated total development API cost**: $500-1,000 over the full build.

### Production Per-User Costs (Estimated)

End users pay their own LLM API costs (bring-your-own-key model). Typical usage estimates:

| Usage Pattern | Estimated Monthly Cost | Model Mix |
| --- | --- | --- |
| Light (few AI queries/day, mostly shell) | $5-10/month | Mostly fast/cheap models |
| Moderate (regular AI tasks, mixed complexity) | $15-25/month | Mix of standard and frontier models |
| Heavy (complex multi-file refactoring, research) | $25-50/month | More frontier model usage |

### Compute & Infrastructure

- **Compute**: Minimal. Both `slate` and `slated` run on the user's local machine. The daemon's memory footprint target is <50MB resident.
- **Cloud backend**: None. Slate Agent has no cloud infrastructure — all processing is local. Users connect directly to LLM provider APIs.
- **CI/CD**: Standard GitHub Actions for building Rust + C++ — negligible cost.
- **No recurring infrastructure costs.**

---

## 9. Risks & Mitigations

| Risk | Likelihood | Impact | Mitigation |
| --- | --- | --- | --- |
| **C++ LLM SDK maturity** — limited ecosystem for streaming + tool calling in C++ | Medium | High | Evaluate early in Phase 2; extend with direct HTTP for unsupported providers; local model fallback via llama.cpp |
| **Multi-agent token costs** — multi-agent systems use ~15x more tokens than single-agent | High | High | Budget fields on every Work Item; cost-aware model selection by TeamLead; demand-driven decomposition to avoid unnecessary subtask explosion |
| **Multi-agent coordination** — concurrent agents can corrupt shared state | Medium | High | Single-writer ownership pattern (no locks); role-based separation reduces contention; validated by Cursor's lessons |
| **API cost overruns** for end users | Medium | Medium | Per-Work-Item budget caps; user-configurable spending limits; TeamLead considers cost tier in model selection |
| **Solo developer risk** — bus factor of 1 | Medium | High | Clean architecture with well-defined boundaries; comprehensive documentation; each phase is self-contained |
| **Security of agent-executed commands** | Low | Critical | Risk classification + confirmation prompts (Phase 2); allowlists; audit trail; OS-level sandboxing (Phase 7) |
| **Dual-language build complexity** (Rust + C++) | Medium | Medium | Clean boundary at IPC layer — Cargo and CMake are fully independent; shared FlatBuffers schema with per-language code generation |

---

## 10. Success Criteria

| Criterion | Target | Measured By |
| --- | --- | --- |
| Command fast-pass latency | <10ms overhead vs raw shell | Benchmark recognized commands |
| Time-to-first-token (AI response) | <500ms streaming | Measure from input to first token display |
| Work Item throughput | 4+ parallel workers | Concurrent task execution benchmark |
| Daemon memory footprint | <50MB resident | Runtime measurement |
| Context persistence accuracy | >90% relevant recall across sessions | Manual evaluation of memory quality |
| Multi-agent token overhead | <8x vs single agent (industry avg ~15x) | Token counting per task |

---

## 11. Deferred Features

The following are explicitly out of scope for the current roadmap:

- **Fish/Zsh native mode** — `slate` uses its own persistent bash co-process
- **Voice agent integration** — text input only
- **Multi-model forked work trees** — running the same task on multiple models in parallel and picking the best result
- **Backpressure system** for tool output and Work Item spawning
- **Scoped capability tokens** for fine-grained session/cwd/repo permissions
- **Circuit breaker with automatic model tier fallback**
- **MCP server hosting** — Slate Agent bridges MCP servers as tools (client), but does not expose its own MCP server
- **Windows support** — Unix-only (macOS and Linux)
- **Open-source release** — not initially; may open-source later

These features can be revisited when real-world usage patterns emerge.

---

## 12. Competitive Comparison (Extended)

### Why Not Just Use Claude Code / Codex CLI / Cursor?

**Claude Code** is the closest competitor in capability. It has recursive subagents, hierarchical memory, and strong benchmarks (80.9% SWE-bench). However, it is locked to Anthropic models — no model selection per task, no cost optimization across providers. It also lacks a true terminal experience: every input goes through the AI, adding latency to simple commands.

**Codex CLI** is open-source and supports OpenAI models plus local inference via Ollama, but has a flat agent architecture. Multi-agent workflows require external orchestration through the Agents SDK and MCP.

**Cursor** pioneered multi-agent coding with its Planner/Worker/Judge pattern, but it is IDE-bound and learned hard lessons about coordination — reader-writer locks degraded 20 concurrent agents to the throughput of 2-3. Slate Agent avoids this with single-writer ownership and shared state coordination.

**Aider** is the most model-agnostic tool (supports any LLM), but has a flat architecture with no task decomposition, no memory persistence, and no concurrent execution.

Slate Agent combines the best ideas from each: terminal-native command execution (unique), recursive multi-agent hierarchy (Claude Code's strength), multi-model flexibility (Aider's strength), and shared-state coordination (Cursor's hard-won lesson).

### Pricing Model

Slate Agent uses a bring-your-own-key model. Users configure their own API keys for the LLM providers they want to use. There are no subscription fees, no cloud infrastructure costs, and no vendor lock-in. The software is free to use; users pay only for the LLM API calls their agent system makes.

This aligns with tools like Aider and Cline, and contrasts with the subscription models of Claude Code ($20-200/month), Cursor ($60-200/month), and Devin ($500/month).
