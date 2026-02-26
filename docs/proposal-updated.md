# Slate Agent --- Technical Proposal

## 1. Purpose

This document defines the complete technical architecture, system
components, execution model, memory system, and multi-model strategy for
Slate Agent.

The one-pager provides a high-level overview.\
This document provides implementation-level clarity.

------------------------------------------------------------------------

## 2. System Architecture

### 2.1 slate Binary (Per Terminal)

Responsibilities: - FTXUI three-zone layout (status bar, main canvas,
input bar) - Persistent bash co-process (`bash --noediting -i` via
pty) - Command fast-pass index (PATH, builtins, aliases) - Env snapshot
capture & sync - Signal forwarding - Rendering agent output blocks -
FlatBuffers IPC client

### 2.2 slated Daemon (Singleton)

Responsibilities: - Agent system lifecycle - Work Item DAG management -
Scheduler + worker thread pool - Spawn-on-demand worker bash sessions -
Memory system management - Shared Project State persistence - Tool
execution routing

Communication:\
FlatBuffers (4-byte length prefix + payload) over Unix domain socket.

------------------------------------------------------------------------

## 3. Execution Model

### 3.1 Command Routing

-   User commands → slate persistent bash
-   Agent tool calls → isolated worker bash sessions in slated

Worker bash sessions: - Fresh process per Work Item - Initialized from
env snapshot - Killed on completion - Enable parallel execution

------------------------------------------------------------------------

## 4. Work Item System

Each Work Item contains:

-   Goal
-   Acceptance criteria
-   Dependencies
-   Risk level
-   Tool allowlist
-   Assigned role
-   Model ID
-   Outputs (artifacts)

Scheduler executes ready nodes concurrently while respecting
dependencies.

------------------------------------------------------------------------

## 5. Agent System

### 5.1 Orchestrator

-   Owns session context
-   Project auto-switching
-   Delegates to TeamLead

### 5.2 TeamLead

-   Decomposes task into Work Item DAG
-   Assigns roles and models
-   Sets acceptance criteria

### 5.3 Engineer

-   Executes implementation Work Items

### 5.4 Reviewer

-   Gates outputs
-   Creates fix Work Items if needed

### 5.5 Optional Roles

-   Security
-   Perf
-   Researcher

### 5.6 Narrator

-   Compresses memory
-   Produces bounded summaries

------------------------------------------------------------------------

## 6. Memory Model

Two bounded layers:

-   Global memory (user habits)
-   Project memory (per repo)

Narrator aggressively curates memory to enforce strict size caps.

------------------------------------------------------------------------

## 7. Shared Project State

All coordination occurs through:

-   Artifacts (diffs, logs, outputs)
-   Decisions (short rationales)
-   Task state (status + pointers)

No agent-to-agent conversational state.

------------------------------------------------------------------------

## 8. Multi-Model Strategy

### 8.1 Fixed Roles

-   Orchestrator → frontier model
-   TeamLead → strong planning model
-   Narrator → fast summarization model

### 8.2 Dynamic Worker Model Selection

TeamLead selects model per Work Item using a configurable model catalog
based on:

-   Task complexity
-   Risk level
-   Latency requirements
-   Cost sensitivity

Scheduler respects assigned model_id when spawning workers.

------------------------------------------------------------------------

## 9. Safety & Audit

-   Tool allowlists
-   Risk classification
-   Destructive command confirmation
-   Full audit trail in Shared Project State

------------------------------------------------------------------------

## 10. Configuration

Configurable via `~/.config/slate-agent/config.toml`:

-   Role model assignments
-   Model catalog
-   Tool limits
-   Memory limits
-   Privacy settings

------------------------------------------------------------------------

## 11. MVP Definition

Included: - slate + slated separation - Work Item DAG - Worker pool -
Fast-pass command index - File edit + git diff tools - Bounded memory

Excluded (Deferred): - Multi-model forked execution - Circuit breakers -
Capability tokens - Voice integration - MCP server support

------------------------------------------------------------------------

## 12. Implementation Requirements

-   C++20
-   ClickHouse ai-sdk-cpp
-   FTXUI
-   FlatBuffers
-   Bazel or CMake
-   LLM provider credentials
