# Rust Daemon + aisdk.rs Integration Design

**Date**: 2026-02-26 | **Author**: Subhagato | **Status**: Approved

---

## 1. Overview

Rewrite the `codivd` daemon from C++ to Rust and integrate `aisdk.rs` as the LLM primitive. The C++ daemon (~400 lines) has no AI logic yet — this is a clean rewrite, not a port. The Rust client (`codiv`) remains unchanged except for switching IPC serialization from FlatBuffers to serde+bincode.

### Key Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Architecture | Two-process (codiv + codivd) | Daemon outlives terminal sessions, serves multiple clients, runs background AI |
| IPC format | serde + bincode | Both sides are Rust — no need for cross-language FlatBuffers. Zero schema compiler. |
| Agent layer | Custom orchestration on aisdk.rs primitives | Full control over DAG, budget, role system. aisdk.rs for LLM calls only. |
| Tool system | aisdk.rs `Tool` type, "everything is a CLI tool" | Binary tools, prompt tools, builtins — all present the same interface. |
| Migration | Clean rewrite | C++ code is small, no AI logic. Idiomatic Rust from scratch. |
| Workspace | Cargo monorepo | `codiv/`, `codivd/`, `codiv-common/` crates. Shared IPC types compile-checked. |

---

## 2. Project Structure

```
codiv/
├── Cargo.toml                  # workspace root
├── crates/
│   ├── codiv/                  # TUI client binary (existing, mostly unchanged)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs
│   │       ├── app.rs
│   │       ├── shell/          # BashCoprocess, InteractiveSession, CommandIndex
│   │       ├── ui/             # terminal.rs, rendering
│   │       └── ipc/            # client-side IPC (serde+bincode)
│   │
│   ├── codivd/                 # daemon binary (NEW — replaces C++ codivd)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs         # daemon startup, PID file, signal handling
│   │       ├── daemon.rs       # Tokio event loop, session management
│   │       ├── worker.rs       # command execution (tokio::process)
│   │       ├── ipc/            # server-side IPC (tokio UnixListener)
│   │       ├── agent/          # orchestrator, agent loop, role system
│   │       ├── tools/          # tool registry, builtins, CLI tool discovery
│   │       └── memory/         # project memory, context management (future)
│   │
│   └── codiv-common/           # shared library crate
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── messages.rs     # IPC message types (serde Serialize/Deserialize)
│           ├── config.rs       # paths, constants, shared config
│           └── types.rs        # WorkItem, AgentRole, RiskLevel, etc.
```

---

## 3. IPC Protocol

### Transport

Same as before: Unix domain socket at `/tmp/codivd-{uid}.sock`, 4-byte big-endian length prefix framing. Payload changes from FlatBuffers to bincode.

### Message Types

```rust
// codiv-common/src/messages.rs

#[derive(Serialize, Deserialize)]
pub struct CommandRecord {
    pub command: String,
    pub output: String,
    pub exit_code: i32,
    pub timestamp: u64,
}

#[derive(Serialize, Deserialize)]
pub struct SessionContext {
    pub cwd: String,
    pub recent_commands: Vec<CommandRecord>,
    pub env_vars: Vec<(String, String)>,
}

#[derive(Serialize, Deserialize)]
pub enum ClientMessage {
    AgentRequest {
        prompt: String,
        request_id: String,
        context: SessionContext,
    },
    EnvSnapshot {
        env_vars: Vec<(String, String)>,
        path: String,
        cwd: String,
    },
    Confirmation {
        request_id: String,
        approved: bool,
    },
    Heartbeat {
        timestamp: u64,
    },
    Shutdown {
        reason: String,
    },
}

#[derive(Serialize, Deserialize)]
pub enum DaemonMessage {
    AgentStreamChunk {
        request_id: String,
        chunk: StreamChunk,
    },
    AgentComplete {
        request_id: String,
        summary: String,
    },
    ConfirmationRequest {
        request_id: String,
        description: String,
        risk: RiskLevel,
    },
    TaskTreeUpdate {
        request_id: String,
        tree: TaskTree,
    },
    Heartbeat {
        timestamp: u64,
    },
    Error {
        request_id: String,
        message: String,
    },
}

#[derive(Serialize, Deserialize)]
pub enum StreamChunk {
    Text(String),
    Reasoning(String),
    ToolCall { name: String, arguments: String },
    ToolResult { name: String, result: String },
}
```

---

## 4. Agent System

### Architecture

Custom orchestration layer using aisdk.rs as the LLM primitive. No aisdk.rs agent loop — we control the loop.

```
User prompt → Orchestrator → TeamLead agent (aisdk.rs call)
                                ↓
                          Work Item DAG
                           ↓      ↓
                     Engineer   Reviewer  (parallel aisdk.rs calls)
                        ↓
                   Tool execution
                        ↓
                   Stream results → IPC → codiv client
```

### Agent Struct

```rust
pub struct Agent {
    pub role: AgentRole,
    pub system_prompt: String,
    pub model: ModelConfig,
    pub tools: Vec<Tool>,         // aisdk.rs Tool objects
    pub context: AgentContext,    // conversation history, budget tracking
}

impl Agent {
    /// Run one turn: send context to LLM, get response, execute any tool calls.
    pub async fn step(&mut self) -> Result<AgentStep> {
        let request = LanguageModelRequest::builder()
            .model(self.model.to_aisdk_model())
            .system(&self.system_prompt)
            .messages(self.context.to_messages())
            .tools(&self.tools)
            .build();

        let response = request.generate_text().await?;

        if let Some(tool_calls) = response.tool_calls() {
            for call in tool_calls {
                let result = self.execute_tool(call).await?;
                self.context.push_tool_result(result);
            }
            return Ok(AgentStep::ToolsExecuted);
        }

        Ok(AgentStep::Complete(response.text().unwrap_or_default()))
    }

    /// Streaming variant — forwards chunks over IPC
    pub async fn step_streaming(&mut self, tx: &IpcSender) -> Result<AgentStep> {
        let response = request.stream_text().await?;
        let mut stream = response.stream;

        while let Some(chunk) = stream.next().await {
            match chunk {
                LanguageModelStreamChunkType::Text(text) => {
                    tx.send(DaemonMessage::AgentStreamChunk {
                        request_id: self.request_id.clone(),
                        chunk: StreamChunk::Text(text),
                    }).await?;
                }
                LanguageModelStreamChunkType::End(msg) => { /* check for tool calls */ }
                _ => {}
            }
        }
        // ...
    }
}
```

### Model Configuration (Multi-Model)

Models configured per-role in TOML, using aisdk.rs `DynamicModel` for runtime selection:

```toml
# ~/.codiv/models.toml
default_provider = "anthropic"

[roles.team_lead]
provider = "anthropic"
model = "claude-sonnet-4-5-20250514"
max_tokens = 8192

[roles.engineer]
provider = "anthropic"
model = "claude-sonnet-4-5-20250514"
max_tokens = 16384

[roles.reviewer]
provider = "openai"
model = "gpt-4o"
max_tokens = 4096
```

### Orchestrator

```rust
pub struct Orchestrator {
    dag: WorkItemDag,
    agents: HashMap<WorkItemId, Agent>,
    model_catalog: ModelCatalog,
    budget: BudgetTracker,
}

impl Orchestrator {
    pub async fn run(&mut self, prompt: &str, tx: &IpcSender) -> Result<()> {
        // 1. TeamLead decomposes prompt into work items
        let team_lead = Agent::new(AgentRole::TeamLead, &self.model_catalog);
        let plan = team_lead.plan(prompt).await?;
        self.dag = WorkItemDag::from_plan(plan)?;

        // 2. Execute ready work items in parallel (up to 4)
        loop {
            let ready = self.dag.ready_items();
            if ready.is_empty() && self.dag.all_complete() { break; }

            let futures: Vec<_> = ready.iter().map(|wi| {
                let agent = self.spawn_agent(wi);
                self.run_agent(agent, wi, tx)
            }).collect();

            let results = futures::future::join_all(futures).await;
            for result in results {
                self.dag.mark_complete(result.work_item_id, result.outcome)?;
            }
        }
        Ok(())
    }
}
```

---

## 5. Tool System — Everything is a CLI Tool

### Core Concept

The shell is the protocol. CLI tools already have a universal interface: stdin, stdout, exit codes, flags, `--help`. Tools are CLI programs that the daemon discovers and invokes. The daemon wraps them as aisdk.rs `Tool` objects dynamically.

### Two Tool Types, One Interface

| Type | Implementation | Discovery |
|------|---------------|-----------|
| **Binary tool** | Executable in `bin/` | `CODIV_TOOLS_PATH` + PATH |
| **Prompt tool** | No binary — daemon is runtime | `tool.toml` + `guide.md` |
| **Builtin** | Compiled into daemon | Always available |

### Progressive Loading (Tier System)

| Tier                      | When Loaded                      | Tools                                                                                         |
| ------------------------- | -------------------------------- | --------------------------------------------------------------------------------------------- |
| **Tier 0** (always)       | Every LLM call                   | Read, Edit, Write, Bash, Glob, Grep, AskUser, Todo, WebFetch, WebSearch, ToolSearch, ToolLoad |
| **Tier 1** (project)      | Auto-loaded from `.codiv/tools/` | Project-specific tools                                                                        |
| **Tier 2** (on-demand)    | After `ToolSearch` + `ToolLoad`  | Global tools from `~/.codiv/tools/`                                                     |
| **Tier 3** (deep context) | When agent calls `--agent-guide` | Reference docs, detailed guides                                                               |

### Tier 0 Built-in Tools

| Tool | Available To | Category |
|------|-------------|----------|
| Read | All roles | File ops |
| Write | Engineer only | File ops |
| Edit | Engineer only | File ops |
| Glob | All roles | Search |
| Grep | All roles | Search |
| Bash | Engineer, Reviewer | Execution |
| AskUser | All roles | User interaction |
| Todo | All roles | Task management |
| WebFetch | All roles | Web |
| WebSearch | All roles | Web |
| ToolSearch | All roles | Tool discovery |
| ToolLoad | All roles | Tool discovery |
| Delegate | TeamLead only | Agent coordination |

### Tool Categories

**Daemon-local** (execute entirely in codivd): Read, Write, Edit, Glob, Grep, Bash, WebFetch, WebSearch, ToolSearch, ToolLoad

**Client-interactive** (require IPC round-trip to codiv): AskUser, Confirm, Todo updates

### Tool Registry

```rust
pub struct ToolRegistry {
    tools: Vec<CliTool>,
}

pub struct CliTool {
    pub name: String,
    pub description: String,
    pub agent_guide: Option<String>,
    pub parameters: JsonSchema,
    pub kind: ToolKind,
}

pub enum ToolKind {
    Binary { path: PathBuf },
    Prompt { toml: ToolToml, guide: String, scripts: PathBuf },
    Builtin(BuiltinFn),
}

impl CliTool {
    pub fn to_aisdk_tool(&self, ctx: ToolContext) -> Tool {
        Tool::builder()
            .name(&self.name)
            .description(&self.description)
            .input_schema(self.parameters.clone())
            .execute(/* dispatch based on ToolKind */)
            .build()
            .unwrap()
    }
}
```

### Skills = Tool + Expertise Prompt

A skill is a tool enhancement — metadata + prompt that makes the agent smarter about using a tool:
- Raw tool: agent knows `git commit` exists
- Skill: agent knows to check status first, write good messages, stage selectively

The `guide.md` content is injected into the agent's system prompt when the tool is active.

### Hooks = Event-Triggered Tools

```toml
[[hooks]]
event = "pre-commit"
tool = "lint-staged"
args = ["--config", ".lintstagedrc"]
```

---

## 6. Daemon Event Loop

### Tokio-based Async Architecture

```rust
impl Daemon {
    pub async fn run(&self) -> Result<()> {
        let listener = UnixListener::bind(&socket_path())?;

        loop {
            tokio::select! {
                Ok((stream, _)) = listener.accept() => {
                    self.handle_connect(stream).await;
                }
                Some(msg) = self.ipc_server.next_message() => {
                    self.dispatch(msg).await;
                }
                _ = tokio::time::sleep(Duration::from_secs(30)) => {
                    self.cleanup_stale_sessions().await;
                }
            }
        }
    }
}
```

### Concurrency Model

- **IPC handling**: Single Tokio task accepts connections and dispatches messages
- **Agent execution**: Each Orchestrator runs as a separate Tokio task
- **Sub-agents**: Each delegated Work Item runs as a spawned Tokio task (up to 4 concurrent)
- **Worker processes**: `tokio::process::Command` for shell execution with async stdout/stderr streaming
- **LLM calls**: aisdk.rs is async/await on Tokio — fits naturally

### Worker Process (Used Internally by Agent's Bash Tool)

The worker module is used internally by the agent's Bash tool — it is NOT triggered by IPC messages from the client. The client never sends commands for the daemon to execute. Instead, the agent invokes shell commands as part of its tool execution loop, and results flow back to the client as `AgentStreamChunk` messages (ToolCall/ToolResult variants).

```rust
pub async fn execute_command(
    command: &str,
    cwd: &str,
    env: &[(String, String)],
    tx: &IpcSender,
    request_id: &str,
) -> Result<i32> {
    let mut child = Command::new("bash")
        .args(["-c", command])
        .current_dir(cwd)
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .spawn()?;

    // Stream stdout/stderr captured by the agent's Bash tool
    // Results sent to client as AgentStreamChunk::ToolResult
    // ...

    let status = child.wait().await?;
    Ok(status.code().unwrap_or(1))
}
```

---

## 7. Key Dependencies

### codivd (daemon)

```toml
[dependencies]
codiv-common = { path = "../codiv-common" }
aisdk = { features = ["openai", "anthropic", "google"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
bincode = "1"
nix = { version = "0.29", features = ["signal", "process"] }
toml = "0.8"
schemars = "0.8"
tracing = "0.1"
tracing-subscriber = "0.3"
```

### codiv-common (shared)

```toml
[dependencies]
serde = { version = "1", features = ["derive"] }
bincode = "1"
```

### codiv (client — updated deps)

Remove `flatbuffers`. Add `codiv-common`, `bincode`, `serde`.

---

## 8. Phased Migration

| Phase | Name | Status | What Changes |
|-------|------|--------|-------------|
| **1** | Terminal Foundation | COMPLETE | No change — codiv client stays as-is |
| **2** | Rust Daemon + Single-Agent AI Loop | NEXT | Rewrite codivd in Rust, aisdk.rs, single-agent tool loop |
| **3** | Work Item DAG + Scheduler | Planned | Tokio tasks, DAG execution engine |
| **4** | Multi-Agent Roles + Multi-Model | Planned | Orchestrator, TeamLead, Delegate, per-role model config |
| **5** | Memory + Project Context | Planned | SQLite via rusqlite |
| **6** | Tool System (Full) | Planned | CLI tool discovery, ToolSearch/ToolLoad, prompt tools, hooks |
| **7** | Advanced Safety & Audit | Planned | Sandboxing, audit log, risk classification |

### Phase 2 Deliverables

1. Cargo workspace setup (root Cargo.toml, 3 crates)
2. `codiv-common` crate (IPC messages, config, paths)
3. `codivd` Rust daemon (Tokio event loop, Unix socket, sessions, workers)
4. IPC migration in codiv client (FlatBuffers → serde+bincode)
5. aisdk.rs integration (provider config, first LLM call)
6. Single-agent loop (Agent struct, Tier 0 builtins)
7. Streaming to client (AgentStreamChunk IPC messages)
8. Basic safety gate (risk classification, user confirmation)
9. Delete C++ codivd source
