# Memory System — Design Document

**Date**: 2026-02-19
**Author**: Subhagato
**Status**: Draft
**Refines**: FR-007 (Memory System), Phase 5 (Memory + Project Context)

---

## 1. Problem

The PRD defines memory as a two-tier system (global 64KB + project 128KB) with Narrator curation. This is underspecified:

- No data model for what a "memory entry" actually is
- No storage format or retrieval mechanism
- No consolidation algorithm — just "Narrator curates aggressively"
- No concept of episodic vs semantic memory — everything is a flat text blob
- No associative retrieval — no way for the agent to ask "what do I know about X?"
- Topics within a project (auth module, build system, API layer) have no first-class representation

This design provides the concrete architecture for FR-007.

## 2. Design Philosophy

**Human memory as the model.** Three cognitive science principles adapted for a coding agent:

1. **Temporal staging** (Atkinson-Shiffrin): information flows from working memory → session memory → long-term memory, with consolidation at each transition
2. **Episodic/Semantic split** (Tulving): raw events are stored separately from distilled knowledge; episodes consolidate into semantic facts over time
3. **Associative recall**: memories are retrieved by semantic relevance, not just by file path or timestamp

**Two storage systems, each for its strength:**
- SQLite for structured episodic data (queryable, indexed, prunable)
- Markdown for semantic knowledge (human-readable, agent-readable, directly injectable into LLM context)

## 3. Storage Layout

```
~/.slate-agent/memory/
├── user.md                              # Semantic: cross-project user preferences, habits
├── memory.db                            # Episodic: SQLite database (all scopes)
└── projects/
    └── <fingerprint>/                   # Derived from git remote + repo root
        ├── project.md                   # Semantic: project conventions, architecture, decisions
        └── topics/
            ├── topics.toml              # Topic index: name → file, tags, last_updated
            ├── auth-module.md           # Semantic: topic-scoped knowledge
            ├── build-system.md
            └── api-layer.md
```

### Project fingerprint

Projects are identified by a fingerprint derived from:
1. Git remote URL (normalized: strip `.git`, lowercase) — primary
2. Repository root path — fallback when no remote exists
3. Manual override via `[project] id` in `.slate-agent/config.toml`

The fingerprint is a stable hash of the primary identifier. Example: `a1b2c3d4` from `github.com/user/repo`.

## 4. Episodic Store (SQLite)

### Schema

```sql
CREATE TABLE episodes (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id  TEXT NOT NULL,
    work_item_id TEXT,                    -- NULL for non-Work-Item events
    scope       TEXT NOT NULL,            -- 'user' | 'project:<fingerprint>'
    timestamp   TEXT NOT NULL,            -- ISO 8601
    event_type  TEXT NOT NULL,            -- see event types below
    summary     TEXT NOT NULL,            -- one-line human-readable summary
    detail      TEXT,                     -- longer description when needed
    tags        TEXT NOT NULL DEFAULT '', -- comma-separated, lowercase
    files       TEXT NOT NULL DEFAULT '', -- comma-separated file paths involved
    outcome     TEXT,                     -- 'success' | 'failure' | 'partial' | NULL
    consolidated BOOLEAN NOT NULL DEFAULT 0,  -- Narrator has processed this
    high_value  BOOLEAN NOT NULL DEFAULT 0,   -- resist age-based pruning
    raw_context TEXT                      -- optional JSON blob for structured data
);

CREATE INDEX idx_episodes_scope ON episodes(scope);
CREATE INDEX idx_episodes_timestamp ON episodes(timestamp);
CREATE INDEX idx_episodes_tags ON episodes(tags);
CREATE INDEX idx_episodes_consolidated ON episodes(consolidated);
CREATE INDEX idx_episodes_session ON episodes(session_id);
```

### Event types

| Event Type | When Written | Example |
|-----------|-------------|---------|
| `command_executed` | After any tool/command execution | "Ran `make test` — 3 failures in auth_test.cpp" |
| `file_modified` | After Write/Edit tool use | "Modified auth.cpp: added JWT validation" |
| `decision_made` | When agent makes an architectural/design choice | "Chose single-writer over mutex for shared state" |
| `error_encountered` | On tool failure or unexpected result | "Compilation failed: missing <jwt.h> header" |
| `error_resolved` | When a previously logged error is fixed | "Fixed missing dependency by adding jwt crate to Cargo.toml" |
| `pattern_observed` | When agent notices a recurring pattern | "User always runs tests after editing auth module" |
| `user_preference` | Explicit user instruction about preferences | "User prefers pytest over unittest" |
| `task_completed` | Work Item finished | "Refactored auth module to use JWT — 5 files changed, all tests pass" |
| `task_failed` | Work Item failed | "Security audit found 2 vulnerabilities in auth flow" |

### Write path

Episodes are written by the agent system at well-defined points:
1. **Tool execution** — after each tool call completes, the daemon writes an episode with the tool name, arguments (sanitized), outcome, and files involved
2. **Work Item completion** — when a Work Item transitions to `completed` or `failed`
3. **Explicit memory** — when the agent decides something is worth remembering (via the `memory_write_episode` tool, available to all agent roles)

Episodes are **append-only** during a session. The Narrator never modifies episodes — it only reads them and marks them as `consolidated`.

## 5. Semantic Store (Markdown)

### Structure

Each semantic file follows a consistent format:

```markdown
# <Scope Title>

_Last updated: 2026-02-19 by Narrator_

## Conventions
- This project uses Cargo with Rust
- Test framework: cargo test
- Code style: rustfmt (default), clippy pedantic

## Architecture Decisions
- **Shared state coordination**: single-writer ownership, no reader-writer locks
  - Rationale: Cursor's lock-based approach failed at scale
  - Date: 2026-02-15

## Known Issues
- auth.cpp is sensitive to concurrency — always test with TSAN enabled

## Patterns
- User runs `make test` after every edit to auth/
- Prefer small, incremental commits over large batches
```

The exact sections are not fixed — the Narrator organizes content as it sees fit. But the file must:
1. Start with a title heading
2. Include a "Last updated" timestamp
3. Stay within its size cap

### Size caps

| File | Default Cap | Configurable Via |
|------|------------|-----------------|
| `user.md` | 64 KB | `[memory] user_max_size_kb` |
| `project.md` | 128 KB | `[memory] project_max_size_kb` |
| Per-topic file | 32 KB | `[memory] topic_max_size_kb` |

When a file approaches its cap (>90%), the Narrator must compress — merge related entries, remove low-value content, or split into subtopics.

## 6. Topic Management

Topics are semantic clusters within a project. They are fully managed by the Narrator — created, merged, split, and retired automatically.

### Topic index

Each project maintains a `topics.toml`:

```toml
# Auto-managed by Narrator. Do not edit manually.

[[topics]]
name = "auth-module"
file = "auth-module.md"
tags = ["auth", "jwt", "login", "session", "oauth"]
last_updated = "2026-02-19T14:30:00Z"
episode_count = 23        # episodes that contributed to this topic

[[topics]]
name = "build-system"
file = "build-system.md"
tags = ["cmake", "build", "compile", "link", "dependency"]
last_updated = "2026-02-18T09:15:00Z"
episode_count = 11
```

### Topic lifecycle

**Creation**: When the Narrator encounters episodes that don't fit any existing topic's tag set, it creates a new topic file and index entry.

**Merging**: When two topics have >60% tag overlap or the Narrator determines they cover substantially the same domain, it merges them — combines the markdown content, unions the tag sets, removes the old entry.

**Splitting**: When a topic file exceeds its size cap, the Narrator splits it into two or more subtopics based on content clustering.

**Retirement**: When a topic has had no new contributing episodes for a configurable period (default: 180 days) and its content is fully captured in `project.md`, the Narrator may retire it — archive the file and remove the index entry.

### Topic matching algorithm

When the Narrator needs to assign new information to a topic:

1. Extract tags from the episode (file paths → module names, tool names, error types, etc.)
2. Score each existing topic by tag overlap: `score = |episode_tags ∩ topic_tags| / |episode_tags|`
3. If best score > 0.3 → assign to that topic
4. If no match → create new topic from the episode's tags
5. If multiple topics match with similar scores → assign to the most recently updated one

## 7. Consolidation Algorithm

The Narrator runs after each completed Work Item. The algorithm is structured and deterministic:

### Input
- New unconsolidated episodes from the completed Work Item
- Current semantic files (user.md, project.md, relevant topics)
- Topic index

### Steps

```
1. EXTRACT
   - Read all episodes where consolidated = 0 AND work_item_id = <current>
   - Group by scope (user vs project)

2. CLASSIFY
   For each episode:
   - Is this a user preference/habit? → target: user.md
   - Is this project-specific? → target: project.md or topic
   - Is this topic-specific? → run topic matching algorithm

3. UPDATE
   For each target file:
   - Read current content
   - Determine what new information the episodes add:
     a. New fact not yet captured → append to relevant section
     b. Reinforcement of existing fact → strengthen (e.g., add "confirmed" or increase confidence)
     c. Contradiction of existing fact → update with new information, note the change
     d. Redundant → skip
   - Ensure file stays within size cap

4. TOPIC MANAGEMENT
   - Check for new topic creation (unmatched episodes)
   - Check for merge candidates (tag overlap > 60%)
   - Check for split candidates (file size > 90% of cap)
   - Update topics.toml

5. MARK CONSOLIDATED
   - SET consolidated = 1 for all processed episodes
   - SET high_value = 1 for episodes containing:
     architectural decisions, unresolved errors, user-stated preferences
```

### Narrator prompt structure

The Narrator is an LLM agent with a specific system prompt and tool access. It is not a hand-coded algorithm — the steps above are instructions in its system prompt, and it uses tools to read/write files and query SQLite. The "structured" aspect is that the prompt defines clear steps and priorities, not that the code is procedural.

Tools available to the Narrator:
- `Read` — read semantic markdown files
- `Write` — write semantic markdown files
- `memory_search` — query episodic SQLite store
- `memory_mark_consolidated` — mark episodes as processed

## 8. Retrieval: Memory Tools

Memory is exposed to all agent roles as tools, following the unified tool model.

### Always-loaded context

At session start, the daemon injects into the Orchestrator's context:
- Full content of `user.md`
- Full content of `project.md` for the detected project

This is automatic — no tool call needed. It mirrors how `user.md` and `project.md` work in context: the agent always has baseline knowledge.

### On-demand tools

**`memory_search`** — query the episodic store

```
Arguments:
  --query <text>       Natural language or keyword query (matched against summary + tags)
  --scope <scope>      'user' | 'project' (default: current project)
  --type <event_type>  Filter by event type
  --recency <days>     Only return episodes from last N days
  --limit <n>          Max results (default: 10)

Output: Formatted list of matching episodes with timestamps, summaries, and tags
```

Implementation: the daemon translates the query into SQL. Keywords are matched against the `summary` and `tags` columns using `LIKE` with tokenization. No vector search — keyword matching is sufficient for the structured, tag-rich episodic format.

**`memory_read`** — load a topic file

```
Arguments:
  --topic <name>       Topic name (from topics.toml)
  --scope <scope>      'user' | 'project' (default: current project)

Output: Full content of the topic markdown file
```

**`memory_list_topics`** — see available topics

```
Arguments:
  --scope <scope>      'user' | 'project' (default: current project)

Output: Table from topics.toml — name, tags, last_updated, episode_count
```

**`memory_write_episode`** — explicitly record something worth remembering

```
Arguments:
  --summary <text>     One-line summary (required)
  --tags <tags>        Comma-separated tags
  --scope <scope>      'user' | 'project' (default: current project)
  --type <event_type>  Event type (default: 'pattern_observed')
  --high-value         Mark as high-value (resist pruning)

Output: Confirmation with episode ID
```

Available to all agent roles. Used when an agent wants to explicitly flag something for long-term retention without waiting for the Narrator's consolidation pass.

## 9. Episodic Retention Policy

The episodic SQLite store has a separate retention policy from the semantic markdown caps:

| Condition | Action |
|-----------|--------|
| `consolidated = 1` AND age > 90 days AND `high_value = 0` | Delete |
| `consolidated = 1` AND age > 365 days AND `high_value = 1` | Delete |
| `consolidated = 0` AND age > 30 days | Force consolidation, then apply above rules |
| Database size > 50 MB | Aggressive pruning: reduce retention thresholds by 50% |

Retention thresholds are configurable:

```toml
[memory.retention]
consolidated_days = 90
high_value_days = 365
unconsolidated_days = 30
max_db_size_mb = 50
```

## 10. Project Auto-Switching

The Orchestrator detects project context and switches memory accordingly:

### Detection signals (in priority order)

1. **Explicit config**: `.slate-agent/config.toml` in cwd with `[project] id`
2. **Git remote**: `git remote get-url origin` → normalize → hash
3. **Git root**: `git rev-parse --show-toplevel` → hash
4. **Working directory**: cwd path → hash (fallback for non-git projects)

### Switch behavior

When the detected project changes:
1. Unload previous `project.md` from Orchestrator context
2. Load new `project.md` into Orchestrator context
3. Update `SLATE_PROJECT_FINGERPRINT` environment variable for tools
4. Any in-flight Work Items continue with their original project context

Switching is **transparent** — no user confirmation needed. The agent announces the switch: *"Switched to project X."*

## 11. Configuration

All memory settings live in the global config:

```toml
# ~/.slate-agent/config.toml

[memory]
storage_path = "~/.slate-agent/memory"   # override storage location
user_max_size_kb = 64
project_max_size_kb = 128
topic_max_size_kb = 32

[memory.retention]
consolidated_days = 90
high_value_days = 365
unconsolidated_days = 30
max_db_size_mb = 50

[memory.topics]
merge_overlap_threshold = 0.6            # tag overlap ratio to trigger merge
retirement_days = 180                    # days without new episodes before retirement
```

## 12. Impact on PRD

This design refines FR-007 and Phase 5. Specific changes needed:

- **FR-007**: Replace current description with reference to this design doc. Key updates: three-axis model (temporal, episodic/semantic, scoped), SQLite + markdown hybrid, memory tools, topic system, structured consolidation.
- **Phase 5 deliverables**: Update to reflect:
  - SQLite episodic store with schema and retention policy
  - Semantic markdown files (user.md, project.md, topics/)
  - Narrator consolidation triggered per Work Item
  - Memory tools (memory_search, memory_read, memory_list_topics, memory_write_episode)
  - Project auto-switching with fingerprint detection
  - Topic management (create/merge/split/retire)
- **FR-006 (Built-in Tools)**: Add memory tools to the built-in tool list
- **Goals table**: The KPI ">90% relevant recall" can be measured by querying the episodic store + semantic files for known facts from previous sessions

## 13. Open Questions

1. **Topic granularity**: Should there be a minimum episode count before a topic is created (to avoid topic explosion from one-off tasks)?
2. **Cross-project knowledge**: Should semantic knowledge ever flow from one project to another (e.g., "this user always prefers X pattern")? Currently only `user.md` is cross-project.
3. **Memory export/import**: Should users be able to export a project's memory for sharing with teammates, or import memory from a shared baseline?
4. **Narrator model selection**: The PRD assigns Narrator to Gemini 2.5 Flash. Is this sufficient for the consolidation algorithm, or does topic management need a more capable model?
