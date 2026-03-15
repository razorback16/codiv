# Hierarchical Memory: How Codiv Agent Remembers

## The analogy: human memory, not file storage

Most AI agents treat memory as a flat key-value store or an append-only log. Codiv Agent models memory on how human cognition actually works — information flows through temporal stages, gets consolidated from raw experience into lasting knowledge, and is recalled associatively by context rather than by filename.

This gives the agent three capabilities no flat memory system provides:

1. **Forgetting** — not everything is worth keeping. The Narrator actively prunes low-value information.
2. **Abstraction** — raw events ("I fixed a deadlock in worker_pool.cpp by switching to single-writer") become distilled knowledge ("this project uses single-writer ownership for shared state").
3. **Associative recall** — asking "what do I know about auth?" retrieves relevant memories across sessions, files, and time periods.

---

## Three axes of memory

Memory is organized along three independent axes. Every piece of information has a position on each.

### Axis 1: Temporal (when) — Atkinson-Shiffrin inspired

```
Working Memory          Session Memory          Long-term Memory
(current context)       (this session)          (persistent)
    │                       │                       │
    │  attention +          │  consolidation         │
    │  tool outputs         │  at Work Item          │
    │                       │  completion            │
    ▼                       ▼                       ▼
Ephemeral               Ephemeral               Durable
(context window)        (session lifetime)      (survives restarts)
```

- **Working memory** is the LLM's context window — current conversation, active tool outputs, in-flight Work Items. It's not stored anywhere; it's the live computation.
- **Session memory** captures what happened during a session — commands run, decisions made, errors encountered, files touched. Stored as episodic entries in SQLite. Gets consolidated by the Narrator after each Work Item.
- **Long-term memory** persists across sessions. This is the curated knowledge base — semantic markdown files that agents read, plus the episodic SQLite store for queryable history.

### Axis 2: Episodic vs Semantic (what kind)

Borrowed from Tulving's memory taxonomy:

- **Episodic memory**: specific events with context. *"In session abc123, the user debugged a race condition in auth.cpp. They tried mutex first (failed), then switched to single-writer pattern (worked). Files touched: auth.cpp, worker_pool.h."*
- **Semantic memory**: distilled facts and patterns. *"This project uses single-writer ownership for shared state. The auth module is sensitive to concurrency issues."*

Episodes are raw material. Semantic knowledge is the refined product. The Narrator's primary job is turning episodes into semantic entries — and then pruning the episodes that have been fully absorbed.

### Axis 3: Scope (where) — associative, not just hierarchical

| Scope | What it captures | Storage |
|-------|-----------------|---------|
| **User** | Cross-project preferences, tool habits, communication style | `~/.codiv/memory/user.md` |
| **Project** | Per-repo conventions, architecture decisions, known gotchas | `~/.codiv/memory/projects/<fingerprint>/project.md` |
| **Topic** | Clusters within a project — "auth module," "build system," "API layer" | `~/.codiv/memory/projects/<fingerprint>/topics/<name>.md` |

Topics are the associative layer. They aren't directories or file paths — they're semantic clusters that the Narrator creates, merges, splits, and retires as the project evolves. Thinking about "auth" might pull knowledge from topics spanning multiple directories and sessions.

---

## Storage model: SQLite + Markdown hybrid

Two storage systems, each playing to its strengths:

**SQLite** (episodic store) — queryable, structured, supports associative retrieval by tags, recency, scope, and event type. Agents query this on demand via the `memory_search` tool.

**Markdown** (semantic store) — human-readable, agent-readable, directly injectable into context. `user.md` and `project.md` are always loaded at session start. Topic files are loaded on demand via the `memory_read` tool.

The Narrator reads from SQLite and writes to markdown. This is a one-way refinement pipeline: episodes → semantic knowledge.

---

## Consolidation: the Narrator's job

The Narrator is triggered after every completed Work Item. It follows a structured algorithm:

1. **Extract** — read new episodic entries from the current Work Item
2. **Match** — identify which existing topics (or user/project scope) the new information relates to
3. **Update** — add, revise, or strengthen semantic entries in the matching markdown files
4. **Create** — if new information doesn't fit existing topics, create a new topic file
5. **Merge/Split** — periodically check if topics should be merged (high overlap) or split (too large)
6. **Prune** — mark episodic entries as consolidated; they become candidates for age-based cleanup

The Narrator is not a single instance — multiple Narrators can run concurrently for different scopes or topics, as long as each markdown file has a single writer at a time (consistent with the single-writer ownership pattern used throughout Codiv Agent).

---

## Retrieval: memory as a tool

Memory retrieval follows the same progressive loading pattern as the tool system:

**Always in context (Tier 0):**
- `user.md` — injected into Orchestrator's context at session start
- `project.md` — injected when project is detected (via cwd, git remote, repo fingerprint)

**On demand (agent queries):**
- `memory_search(query, scope?, recency?)` — searches episodic SQLite store by tags, summaries, event types
- `memory_read(topic)` — loads a specific topic markdown file into context

This means the agent actively decides when it needs more memory — just like a human reaching for a specific recollection. The base context stays lean.

---

## Size management

**Semantic layer (markdown):** bounded per scope. These files are what cost tokens when loaded into context, so caps are enforced:
- `user.md` — capped (configurable, default 64KB)
- `project.md` — capped (configurable, default 128KB)
- Topic files — capped per file (configurable, default 32KB)
- Narrator is responsible for staying within bounds during consolidation

**Episodic layer (SQLite):** age-based retention. Disk is cheap; the episodic store is only queried on demand, not loaded into context. Retention policy prunes entries older than a configurable threshold (e.g., 90 days) unless they're marked as high-value by the Narrator.

---

## How it all fits together

```
User input
    │
    ▼
Orchestrator (has user.md + project.md in context)
    │
    ├── needs more context? ──► memory_search / memory_read
    │
    ▼
TeamLead decomposes task → Work Items
    │
    ▼
Engineer executes Work Item
    │
    ▼
Work Item completes → episodic entry written to SQLite
    │
    ▼
Narrator triggered
    ├── reads new episodes
    ├── updates semantic markdown (project.md, topics/)
    └── prunes consolidated episodes
```

The memory system is invisible to the user but continuously learning. After enough sessions, the agent knows the project's conventions, the user's preferences, and the recurring patterns — without ever being explicitly told to remember them.
