# Agent System — Design Document

**Date**: 2026-02-26
**Author**: Subhagato
**Status**: Draft
**Refines**: FR-004 (Agent System), Phase 2 (Single-Agent AI Loop), Phase 4 (Multi-Agent Roles + Multi-Model)

---

## 1. Overview

This document specifies the architecture of Codiv Agent's AI agent system — how LLMs are integrated, how the tool calling loop operates, how system prompts are structured, how context is managed, how streaming output reaches the terminal, and how the system evolves from a single combined agent (Phase 2) to a multi-agent hierarchy with specialized roles (Phase 4).

The PRD defines agent roles (Orchestrator, TeamLead, Engineer, Reviewer, Narrator) and Work Items at a requirements level. This design provides the concrete implementation architecture.

## 2. Problem

The PRD (FR-004, Phases 2 and 4) establishes agent roles, the Orchestrator-Worker coordination pattern, and single-writer ownership semantics. It does not specify:

- How the LLM is called — the integration surface between the daemon and aisdk.rs
- How tool calling works in a loop — the full cycle from LLM generating a tool call to result injection and continuation
- How system prompts are structured — what information each role receives and in what format
- How context is managed — token budget allocation, compaction strategy, tool output truncation
- How streaming output reaches the terminal — the path from aisdk.rs streaming through IPC to rendered agent blocks in codiv
- How single-agent (Phase 2) evolves to multi-agent (Phase 4) — the concrete transition path

This design fills those gaps.

## 3. Phase 2: Single-Agent Architecture

Phase 2 delivers a combined Orchestrator+Engineer agent — a single LLM session that receives user input classified as `AiQuery` by the command fast-pass, reasons about it, calls tools, and produces output.

### 3.1 aisdk.rs Integration

The daemon (`codivd`) integrates with aisdk.rs for streaming LLM access with tool calling. The integration surface:

```
codivd
  └── AgentSession
        ├── aisdk::Client               // HTTP client to LLM provider
        ├── aisdk::LanguageModelRequest  // streaming completion with tool definitions
        ├── system_prompt: String        // assembled at session start
        ├── messages: Vec<Message>       // growing conversation history
        └── tool_registry: ToolRegistry  // available tools and their schemas
```

- **Provider abstraction**: aisdk.rs handles OpenAI, Anthropic, and Google API formats. For unsupported providers, the daemon falls back to direct HTTP via reqwest with a custom SSE parser.
- **Streaming**: aisdk.rs provides an async Stream interface — each token or tool call chunk is yielded as a stream item that the daemon processes.
- **Tool definitions**: Built-in tools (Bash, Read, Write, Edit, Glob, Grep) are registered as tool schemas (JSON Schema format) that aisdk.rs includes in the completion request.

### 3.2 Tool Calling Loop

The core agent loop is a synchronous cycle within a single LLM turn:

```
1. User input arrives at daemon (via IPC from codiv)
2. Daemon constructs messages array: [system_prompt, ...history, user_message]
3. Daemon calls aisdk.rs streaming completion with tool definitions
4. For each streamed chunk:
   a. If text token → forward to codiv via IPC (AgentStreamChunk message)
   b. If tool_call → pause streaming, execute the tool:
      i.   Parse tool name + arguments from the LLM's structured output
      ii.  Dispatch to tool implementation (built-in or external via CODIV_TOOLS_PATH)
      iii. Capture tool result (stdout, stderr, exit code)
      iv.  Append tool_call message and tool_result message to conversation history
      v.   Resume streaming completion with updated messages (LLM sees the result)
5. LLM continues generating — may produce more text or more tool calls
6. Loop terminates when LLM produces a final text response with no tool calls
7. Daemon sends AgentComplete to codiv
```

Key behaviors:

- **Multi-step tool use**: The LLM can chain multiple tool calls in a single turn. Each tool result is injected back, and the LLM decides whether to call another tool or produce a final response.
- **Tool call accumulation**: aisdk.rs streams tool call arguments incrementally (JSON tokens). The daemon accumulates until the tool call is complete before executing.
- **Parallel tool calls**: If the LLM generates multiple tool calls in a single response (supported by OpenAI and Anthropic APIs), the daemon executes them concurrently and returns all results together.

### 3.3 System Prompt Structure

The combined Orchestrator+Engineer system prompt for Phase 2:

```
[ROLE]
You are Codiv Agent, a terminal-native coding assistant. You operate inside a
terminal where the user types commands and natural language requests. You can
execute shell commands, read and modify files, and search codebases.

[AVAILABLE TOOLS]
{tier_0_tool_list}
// Tier 0: one-line descriptions of all available tools
// Example:
// - Bash: Execute shell commands
// - Read: Read file contents with line range support
// - Write: Create or overwrite files
// - Edit: Replace exact strings in files
// - Glob: Find files by pattern
// - Grep: Search file contents (ripgrep-style)

[USER CONTEXT]
Working directory: {cwd}
Platform: {os} {arch}
Shell: {shell}
Last exit code: {last_exit_code}

[PROJECT MEMORY]
{project_md_content}
// Injected from ~/.codiv/memory/projects/<fingerprint>/project.md
// Empty string if no project memory exists yet

[USER MEMORY]
{user_md_content}
// Injected from ~/.codiv/memory/user.md
// Empty string if no user memory exists yet

[CONSTRAINTS]
- Always verify before destructive operations (rm -rf, git push --force, etc.)
- Prefer reading files before editing them
- Keep responses concise — you are in a terminal, not a chat interface
- When a task is complete, state what was done and stop
```

The system prompt is assembled at session start and remains static for the session lifetime. Dynamic context (cwd changes, new tool discoveries) is injected as user messages, not by modifying the system prompt.

### 3.4 Context Management

The conversation history grows with each user message, assistant response, tool call, and tool result. Without management, this exceeds the model's context window.

**Token budget allocation** (for a 200K context window model):

| Segment | Budget | Notes |
|---------|--------|-------|
| System prompt | ~2,000 tokens | Role + tools + constraints |
| User context | ~1,000 tokens | cwd, env, platform info |
| Project memory | ~2,000 tokens | project.md content |
| User memory | ~1,000 tokens | user.md content |
| Conversation history | Remaining (~194,000) | Messages, tool calls, tool results |

**Compaction strategy**:

When the total token count exceeds 80% of the model's context window:

1. Identify the oldest N messages (excluding system prompt and the most recent 10 messages)
2. Send these messages to a fast, cheap model (e.g., Gemini 2.5 Flash) with the prompt: "Summarize the following conversation history, preserving: key decisions, file paths mentioned, errors encountered, and current task state."
3. Replace the N messages with a single `[CONVERSATION SUMMARY]` message containing the summary
4. Continue the session with the compacted history

Compaction preserves:
- The system prompt (never compacted)
- The 10 most recent messages (recent context is critical)
- All messages from the current tool calling chain (mid-chain compaction breaks the loop)

### 3.5 Streaming Output

The path from LLM to terminal:

```
aisdk.rs stream item
  → AgentSession::on_token(chunk: &str)
    → IPC: AgentStreamChunk { request_id, chunk: StreamChunk::Text(chunk) }
      → Unix socket write to codiv client
        → codiv receives AgentStreamChunk
          → Appends to agent output block widget
            → ratatui re-renders the block with new content
              → User sees streaming text in a color-bordered agent block
```

- **Chunking**: The daemon forwards tokens as they arrive from aisdk.rs. No batching — latency-sensitive path.
- **Tool call visibility**: When a tool call is detected, the daemon sends a status message to codiv: `[Calling: Bash("make test")]`. The tool's output streams separately. When the tool completes, the daemon sends `[Tool result: exit code 0]` and resumes LLM streaming.
- **Markdown rendering**: codiv uses comrak + syntect to render the agent's markdown output with syntax highlighting in code blocks. For streaming, codiv maintains a growing buffer and re-parses on significant updates (every ~500 characters or on newline).

### 3.6 Error Handling

| Error | Detection | Action |
|-------|-----------|--------|
| Tool execution fails (non-zero exit) | Exit code from subprocess | Inject error result into context; LLM sees the failure and can retry or report |
| Tool execution times out | Configurable timeout (default: 120s) | Kill the subprocess; inject timeout error into context |
| LLM API error (rate limit, auth, server) | HTTP status code from aisdk.rs | Retry with exponential backoff (3 attempts: 2s, 4s, 8s); on final failure, report to user |
| LLM produces invalid tool call | JSON parse failure on tool arguments | Inject parse error into context; LLM typically self-corrects |
| Context window exceeded | Token count check before API call | Trigger compaction; if still over, truncate oldest tool results |

### 3.7 Stop Conditions

The agent loop terminates when:

1. **Agent decides task is complete** — LLM produces a final text response with no tool calls
2. **User interrupts** — Ctrl+C forwarded from codiv to daemon; daemon cancels the in-flight API call and any running tool subprocess
3. **Budget exceeded** — token or cost budget for the Work Item is exhausted (see work-item-dag-design.md)
4. **Maximum turns reached** — configurable safety limit (default: 50 tool calling turns per request) to prevent infinite loops
5. **Unrecoverable error** — LLM API fails after all retries; daemon reports failure to codiv

## 4. Phase 4: Multi-Agent Architecture

Phase 4 separates the combined Orchestrator+Engineer into distinct roles, each with its own LLM session and system prompt.

### 4.1 Agent Role Separation

| Role | LLM Session | Lifecycle | Primary Responsibility |
|------|------------|-----------|----------------------|
| Orchestrator | Persistent (session-scoped) | Created at session start, destroyed at session end | Intent classification, delegation, project context |
| TeamLead | On-demand | Created per complex task, destroyed after DAG construction | Task decomposition, Work Item DAG, model selection |
| Engineer | On-demand | Created per Work Item, destroyed after completion | Tool execution, code generation, following acceptance criteria |
| Reviewer | On-demand | Created per review Work Item, destroyed after review | Validation against acceptance criteria, creating fix Work Items |
| Narrator | On-demand | Created after Work Item completion, destroyed after consolidation | Memory compression, episode consolidation (see memory-system-design.md) |

Each role gets its own `AgentSession` instance with:
- A role-specific system prompt
- Its own conversation history
- Access to a subset of tools appropriate to its role
- A model assigned from the catalog (fixed for Orchestrator/TeamLead/Narrator, dynamic for Engineer/Reviewer)

### 4.2 Orchestrator

The Orchestrator is the session-level agent. It persists for the entire user session and handles every user input that is classified as `AiQuery`.

**Responsibilities**:
- Classify user intent: simple question (answer directly), single-file task (handle directly or delegate to Engineer), complex task (delegate to TeamLead)
- Maintain project context: detect project changes on `cd`, load/unload project memory
- Present results to the user: aggregate Work Item outputs into a coherent response

**Model assignment**: Frontier model (e.g., Claude Sonnet 4.5) — the Orchestrator sees every user message and must make high-quality routing decisions.

**Tools available**: Read, Grep, Glob, memory_search, memory_read, memory_list_topics (read-only tools — the Orchestrator does not write files directly).

### 4.3 TeamLead

The TeamLead is created when the Orchestrator determines that a task requires decomposition.

**Responsibilities**:
- Receive the task description and context from the Orchestrator
- Produce a Work Item DAG as structured output (see work-item-dag-design.md for DAG construction details)
- Select models from the catalog for each Work Item based on complexity, cost tier, and capability requirements
- Optionally hire specialist roles (Security, Perf, Researcher) when task characteristics warrant them

**Model assignment**: Strong planning model (e.g., Claude Sonnet 4.5 or o3) — planning quality directly determines execution quality.

**Tools available**: Read, Grep, Glob (for codebase analysis during planning), memory_search, memory_read (for project context).

**Structured output**: The TeamLead produces a JSON plan that the daemon parses into Work Items:

```json
{
  "work_items": [
    {
      "goal": "Add JWT validation to auth.cpp",
      "acceptance_criteria": ["JWT tokens are validated on every API endpoint", "Invalid tokens return 401"],
      "assigned_role": "Engineer",
      "model_id": "claude-sonnet-4-5",
      "risk_level": "medium",
      "dependencies": [],
      "context": "The current auth uses session cookies. See auth.cpp and middleware.cpp."
    },
    {
      "goal": "Write tests for JWT validation",
      "acceptance_criteria": ["Unit tests cover valid, expired, and malformed tokens", "All tests pass"],
      "assigned_role": "Engineer",
      "model_id": "gemini-2.5-flash",
      "risk_level": "low",
      "dependencies": ["<id of first work item>"]
    },
    {
      "goal": "Review JWT implementation for security issues",
      "acceptance_criteria": ["No hardcoded secrets", "Token expiry is enforced", "Algorithm is not 'none'"],
      "assigned_role": "Reviewer",
      "model_id": "claude-opus-4",
      "risk_level": "high",
      "dependencies": ["<id of first work item>"]
    }
  ]
}
```

### 4.4 Engineer

The Engineer is the workhorse — created per Work Item, given a goal and acceptance criteria, and expected to accomplish it using tools.

**Model assignment**: Dynamic — selected by the TeamLead per Work Item from the model catalog. Simple tasks get cheap/fast models (Gemini 2.5 Flash), complex tasks get frontier models (Claude Sonnet 4.5).

**Tools available**: Bash, Read, Write, Edit, Glob, Grep, WebFetch, memory_search, memory_read, memory_write_episode.

**Execution**: The Engineer runs the standard tool calling loop (Section 3.2) within its Work Item's scope. It has access to its own worker bash session (spawned by codivd, initialized from env snapshot). On completion, it produces artifacts (diffs, files, transcripts) stored in Shared Project State.

### 4.5 Reviewer

The Reviewer validates Work Item outputs against acceptance criteria.

**Model assignment**: Capable model — review quality matters. Typically the same tier as the Engineer or one tier higher for high-risk Work Items.

**Tools available**: Read, Grep, Glob, Bash (for running tests), memory_search.

**Behavior**:
- Receives the Work Item's acceptance criteria and the artifacts produced by the Engineer
- Reads the modified files, runs tests if applicable, checks each acceptance criterion
- Produces a verdict: `approve` or `reject` with specific feedback
- On rejection, the Reviewer creates a fix Work Item with the feedback as context, which is added to the DAG (see work-item-dag-design.md for dynamic DAG modification)

### 4.6 Narrator

The Narrator is a specialized agent that runs after Work Item completion to consolidate episodic memory into semantic memory. Its architecture is fully specified in the [Memory System Design](memory-system-design.md), Section 7. Key points:

- Triggered after each completed Work Item
- Reads unconsolidated episodes from SQLite, classifies them, updates semantic markdown files
- Manages topic lifecycle (create, merge, split, retire)
- Uses a fast/cheap model (e.g., Gemini 2.5 Flash) — consolidation does not require frontier intelligence

### 4.7 Inter-Agent Communication

Agents do not communicate directly. All coordination happens through Shared Project State (FR-008):

```
                    Shared Project State
                   ┌─────────────────────┐
                   │  Artifacts           │
                   │  Decisions           │
                   │  Task State (DAG)    │
                   │  Work Item Outputs   │
                   └─────────┬───────────┘
                             │
        ┌────────────────────┼────────────────────┐
        │ read               │ read/write          │ read
        ▼                    ▼                      ▼
   ┌──────────┐      ┌──────────────┐       ┌──────────┐
   │ TeamLead │      │   Engineer   │       │ Reviewer │
   └──────────┘      └──────────────┘       └──────────┘
```

- **Engineer writes**: artifacts (diffs, files, logs), decisions, task state updates
- **Reviewer reads**: Engineer's artifacts, acceptance criteria from the Work Item
- **TeamLead reads**: Reviewer verdicts, task state — to decide on re-planning
- **Single-writer ownership**: At any time, only one agent writes to a given resource. The daemon enforces this — when a Work Item is assigned to an agent, that agent holds write ownership of its artifact directory and the files it operates on.

### 4.8 Agent Lifecycle

```
User input → Orchestrator classifies intent
  │
  ├── Simple question → Orchestrator answers directly
  │
  ├── Single-file task → Orchestrator delegates to Engineer (no TeamLead)
  │
  └── Complex task → Orchestrator delegates to TeamLead
                        │
                        └── TeamLead produces DAG → Scheduler fires Work Items
                              │
                              ├── Engineer Work Item → Agent created → tools → artifacts → Agent destroyed
                              │
                              ├── Reviewer Work Item → Agent created → review → verdict → Agent destroyed
                              │
                              └── Narrator fires (post-completion) → consolidate memory → Agent destroyed
```

Agents are ephemeral except for the Orchestrator. This keeps memory usage bounded and avoids stale context accumulation.

### 4.9 Concurrent Agents

Multiple Engineers can run simultaneously on independent Work Items (independent branches of the DAG). Each concurrent agent gets:

- Its own LLM session (separate aisdk.rs completion stream)
- Its own worker bash process (spawned by codivd from env snapshot)
- Its own artifact directory (`~/.codiv/state/<session_id>/artifacts/<work_item_id>/`)
- No shared mutable state with other concurrent agents

Maximum concurrent agents is configurable (default: 4, matching the default Tokio concurrent task limit).

## 5. Context Management

### 5.1 Token Budget Allocation

Context window sizes vary by model. The daemon reads the model's context window size from the catalog and allocates budgets proportionally:

| Segment | Allocation | 200K Model | 128K Model | 32K Model |
|---------|-----------|------------|------------|-----------|
| System prompt | ~1% | 2,000 | 1,280 | 320 |
| User context | ~0.5% | 1,000 | 640 | 160 |
| Project memory | ~1% | 2,000 | 1,280 | 320 |
| User memory | ~0.5% | 1,000 | 640 | 160 |
| Conversation history | ~97% | 194,000 | 124,160 | 31,040 |

For small context windows (<32K), project memory and user memory are truncated to fit. The daemon prioritizes project memory over user memory (project-specific context is more immediately relevant).

### 5.2 Compaction

Compaction triggers when conversation history reaches 80% of its allocated budget:

1. **Select messages for compaction**: All messages older than the most recent 10, excluding:
   - Messages in the current tool calling chain (active chain must stay intact)
   - Messages containing architectural decisions (detected by keyword heuristics: "decided", "chose", "because")
2. **Summarize**: Send selected messages to a fast model with a summarization prompt
3. **Replace**: Swap the selected messages with a single summary message prefixed with `[CONTEXT SUMMARY]`
4. **Log**: Record the compaction event in metrics (pre/post token count, compression ratio)

If compaction is insufficient (still above 80% after summarization), the daemon performs aggressive truncation — removing the oldest tool results first (they are the largest and least likely to be relevant).

### 5.3 Tool Output Truncation

Tool outputs can be large (e.g., `grep` across a large codebase). The daemon truncates before injecting into context:

| Output Size | Action |
|-------------|--------|
| < 10,000 chars | Inject as-is |
| 10,000 - 50,000 chars | Truncate to first 5,000 + last 2,000 chars with `[... truncated {N} chars, use Read tool for full content ...]` |
| > 50,000 chars | Truncate to first 3,000 + last 1,000 chars with truncation notice |

The full output is always stored in the Work Item's artifact directory and available via the Read tool.

### 5.4 File Content Caching

Recently read files are cached in the daemon's memory (per session, LRU cache with configurable max entries, default: 50). When the agent calls Read on a cached file:

- If the file's mtime has not changed, the daemon returns the cached content without re-reading from disk
- The cached content is included in the tool result message — it is not re-injected into the system prompt

This reduces disk I/O but does not save context tokens (the tool result still appears in the conversation history).

## 6. Streaming Architecture

### 6.1 Token Streaming Path

```
LLM Provider (HTTP/SSE)
  │
  │ SSE event: data: {"choices":[{"delta":{"content":"Hello"}}]}
  ▼
aisdk.rs
  │
  │ stream item: StreamChunk::Text("Hello")
  ▼
AgentSession::on_token("Hello")
  │
  │ Accumulate in response_buffer
  │ Build IPC message: AgentStreamChunk { request_id, chunk: StreamChunk::Text("Hello") }
  ▼
Unix socket write (framed: 4-byte BE length + bincode payload)
  │
  ▼
codiv IPC client receives AgentStreamChunk
  │
  │ Route to correct agent output block by work_item_id
  │ Append text to block's content buffer
  ▼
ratatui re-render
  │
  │ Agent output block widget re-draws with new content
  │ Markdown rendering (comrak + syntect) applied on re-draw
  ▼
Terminal output (user sees streaming text in color-bordered block)
```

### 6.2 Tool Call Detection in Stream

Tool calls are detected during streaming:

1. aisdk.rs detects a `tool_calls` field in the streamed response (OpenAI format) or a `tool_use` content block (Anthropic format)
2. aisdk.rs yields a ToolCall stream item with name and arguments_json
3. The daemon:
   a. Sends a status message to codiv: `[Calling: {tool_name}({brief_args})]`
   b. Executes the tool (may take seconds — Bash commands, file operations)
   c. Sends tool output to codiv as it streams (for Bash, stdout/stderr stream in real time)
   d. Sends tool completion status: `[Tool complete: {tool_name} → {exit_code}]`
   e. Injects the tool result into the conversation and makes another completion call
4. The next completion call may produce more text, more tool calls, or a final response

### 6.3 Output Block Management

codiv manages agent output blocks in its ratatui widget tree:

- Each Work Item gets a dedicated output block identified by `work_item_id`
- Blocks are color-bordered by agent role (Cyan for Engineer, Yellow for Reviewer, etc.)
- While a Work Item is running, its block auto-scrolls with new content
- When a Work Item completes, its block auto-collapses to a single-line summary
- User can expand collapsed blocks with Enter, or focus a block with Ctrl+F

For concurrent Work Items, multiple blocks are visible simultaneously, each streaming independently.

## 7. System Prompt Templates

### 7.1 Orchestrator

```
You are the Orchestrator for Codiv Agent, a terminal-native coding assistant.

ROLE: You manage the user's session. You interpret user input, decide how to
handle it, and present results.

DECISION FRAMEWORK:
- Simple factual question → answer directly from your knowledge
- Single-file task or quick edit → delegate to an Engineer Work Item
- Multi-file task, refactoring, or complex work → delegate to TeamLead for
  decomposition into a Work Item DAG

AVAILABLE TOOLS:
{tool_list}

USER CONTEXT:
Working directory: {cwd}
Platform: {platform}

PROJECT CONTEXT:
{project_memory}

USER PREFERENCES:
{user_memory}

CONSTRAINTS:
- Do not modify files directly — delegate to Engineer
- Keep responses concise — terminal environment
- When delegating, provide clear task description and relevant context
- Announce project context switches when detected
```

### 7.2 TeamLead

```
You are a TeamLead for Codiv Agent. You decompose complex tasks into executable
Work Items.

ROLE: Analyze the task, examine relevant code, and produce a plan as a
structured Work Item DAG.

OUTPUT FORMAT: Respond with a JSON object containing a "work_items" array.
Each work item must have: goal, acceptance_criteria, assigned_role, model_id,
risk_level, dependencies, and context.

MODEL CATALOG:
{model_catalog_summary}

AVAILABLE ROLES:
- Engineer: code changes, file operations, command execution
- Reviewer: validates outputs against acceptance criteria
- Security: audits for security issues (hire when task involves auth, secrets,
  infrastructure, or permissions)
- Perf: performance analysis (hire when task involves optimization or benchmarks)
- Researcher: domain investigation (hire when task involves unfamiliar APIs or
  libraries)

AVAILABLE TOOLS:
{tool_list}

CONSTRAINTS:
- Prefer fewer, larger Work Items over many small ones (reduce overhead)
- Set realistic token budgets — simple tasks: 10K tokens, medium: 50K, complex: 200K
- Use cheaper models for simple tasks (tests, formatting, docs)
- Use frontier models for complex tasks (architecture, security, core logic)
- Always include a Reviewer Work Item for high-risk changes
- Follow ADAPT: only decompose when a single agent cannot handle the task directly
```

### 7.3 Engineer

```
You are an Engineer for Codiv Agent. You execute a specific Work Item by using
tools to accomplish the goal.

GOAL:
{work_item_goal}

ACCEPTANCE CRITERIA:
{acceptance_criteria}

CONTEXT:
{work_item_context}

AVAILABLE TOOLS:
{tool_list}

CONSTRAINTS:
- Focus solely on the goal — do not make changes outside your scope
- Read files before editing them
- Run tests after making changes when applicable
- If you encounter an obstacle you cannot resolve, report it clearly and stop
- Do not ask the user questions — work with the information provided
- Verify each acceptance criterion before reporting completion
```

### 7.4 Reviewer

```
You are a Reviewer for Codiv Agent. You validate Work Item outputs against
acceptance criteria.

WORK ITEM UNDER REVIEW:
Goal: {work_item_goal}
Acceptance Criteria:
{acceptance_criteria}

ARTIFACTS TO REVIEW:
{artifact_list}

AVAILABLE TOOLS:
{tool_list}

INSTRUCTIONS:
1. Read each modified file
2. Check each acceptance criterion — is it satisfied? Be specific.
3. Run tests if applicable (use Bash tool)
4. Produce a verdict:
   - APPROVE: all criteria met, no issues found
   - REJECT: list each failing criterion with specific feedback

OUTPUT FORMAT:
{
  "verdict": "approve" | "reject",
  "criteria_results": [
    {"criterion": "...", "met": true/false, "notes": "..."}
  ],
  "issues": ["..."],
  "fix_suggestions": ["..."]
}
```

### 7.5 Narrator

```
You are the Narrator for Codiv Agent. You consolidate episodic memory into
semantic knowledge files.

ROLE: After a Work Item completes, you process new episodes and update the
project's semantic memory files.

STEPS:
1. EXTRACT: Read unconsolidated episodes for the completed Work Item
2. CLASSIFY: Determine target — user.md (user preferences), project.md
   (project knowledge), or a topic file (domain-specific)
3. UPDATE: Add new facts, reinforce existing ones, resolve contradictions,
   skip redundant information
4. MANAGE TOPICS: Create new topics for unmatched episodes, merge topics
   with >60% tag overlap, split topics exceeding size cap
5. MARK: Set consolidated = 1 for all processed episodes

AVAILABLE TOOLS:
- Read: read semantic markdown files
- Write: write semantic markdown files
- memory_search: query episodic SQLite store
- memory_mark_consolidated: mark episodes as processed

SIZE CAPS:
- user.md: {user_max_size_kb} KB
- project.md: {project_max_size_kb} KB
- Topic files: {topic_max_size_kb} KB each

CONSTRAINTS:
- Never delete information about unresolved errors or user-stated preferences
- Prefer compression over deletion
- Maintain consistent markdown formatting
- Include "Last updated" timestamp on every file modification
```

## 8. Open Questions

1. **Long-running tool calls**: How should the system handle tool calls that take >30 seconds? Options: (a) show a progress spinner in the agent output block, (b) impose a hard timeout with configurable override, (c) allow the LLM to set an expected duration hint. Current default timeout is 120 seconds.

2. **Orchestrator fast path**: Should the Orchestrator have a "fast path" for simple tasks that skips the TeamLead entirely? The current design allows this (Orchestrator delegates directly to Engineer for single-file tasks), but the threshold for "simple" vs "complex" is heuristic. Should the user be able to configure this threshold?

3. **Model-specific tool calling formats**: OpenAI and Anthropic use different tool calling formats. aisdk.rs abstracts this, but edge cases exist (e.g., Anthropic's tool_use blocks vs OpenAI's function_call). How should the daemon handle models that don't support native tool calling (e.g., some local models via llama.cpp)?

4. **Dynamic context budget allocation**: Context window sizes vary by model (32K to 200K+). The current design uses percentage-based allocation, but this may under-allocate for small-context models where 1% is only 320 tokens. Should there be minimum absolute allocations that override percentages?

5. **Agent warm-up cost**: Each agent session starts cold (no conversation history). For frequently created roles like Engineer, the system prompt + context injection takes ~3-5K tokens before any useful work begins. Is this overhead acceptable, or should there be a mechanism for lightweight agent sessions with minimal prompts?

6. **Reviewer rejection loops**: If the Reviewer rejects a Work Item and the fix Work Item also fails review, the system could enter a loop. Should there be a maximum retry count (e.g., 3 attempts) before escalating to the user?
