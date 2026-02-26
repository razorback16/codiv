# Slate Agent --- One Pager

## Overview

Slate Agent is a **terminal-native AI coding assistant written in C++**
that integrates directly into the developer's shell workflow.\
It separates **terminal UX (slate)** from **AI orchestration (slated
daemon)** to deliver near-zero latency command execution and structured
AI task execution.

------------------------------------------------------------------------

## Problem

Developers constantly switch between: - Terminal - Editor - AI tools

This context switching slows down execution, fragments workflow, and
reduces productivity.

------------------------------------------------------------------------

## Solution

Slate Agent behaves like a normal shell but adds AI task execution when
needed.

**Two execution modes:** 1. **Command Mode (Fast-Pass)** -- Recognized
shell commands execute instantly via a persistent bash process. 2.
**Agent Mode** -- Natural language tasks are decomposed into structured
Work Items and executed with full visibility.

------------------------------------------------------------------------

## Core Architecture

-   **slate (binary)**
    -   Owns terminal UI (FTXUI)
    -   Maintains persistent bash session
    -   Classifies command vs task
    -   Renders task DAG and agent output
-   **slated (daemon)**
    -   Manages agents, memory, scheduling
    -   Executes Work Items via isolated worker bash sessions
    -   Maintains shared project state

Communication: FlatBuffers over Unix domain socket.

------------------------------------------------------------------------

## Core Abstraction: Work Item

All AI tasks are represented as Work Items containing: - Goal +
acceptance criteria - Dependencies (DAG structure) - Assigned role
(Engineer, Reviewer, etc.) - Outputs (diffs, logs, summaries)

------------------------------------------------------------------------

## Agent Roles

1.  **Orchestrator** -- Owns session & context
2.  **Executor Roles** -- TeamLead, Engineer, Reviewer (+ optional
    Security, Perf, Researcher)
3.  **Narrator** -- Curates bounded memory

------------------------------------------------------------------------

## Key Principles

-   Separate thinking (agents) from execution (worker pool)
-   Tree of agents, graph of tasks
-   Shared explicit state (no agent chat)
-   Strictly bounded memory
-   Near-zero latency for real commands

------------------------------------------------------------------------

## MVP Scope

-   slate + slated IPC
-   Work Item DAG + scheduler
-   Fast-pass command cache
-   File edits, git diffs, test execution
-   Bounded memory model

------------------------------------------------------------------------

## Impact

-   Fully terminal-native AI
-   Parallel task execution
-   Strong auditability
-   Model-agnostic architecture
