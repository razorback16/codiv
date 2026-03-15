# Rust Daemon + aisdk.rs Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Rewrite codivd from C++ to Rust, integrate aisdk.rs for LLM access, and implement a single-agent AI loop with Tier 0 builtin tools.

**Architecture:** Cargo workspace with three crates — `codiv-common` (shared IPC types), `codivd` (Tokio-based daemon with aisdk.rs), and `codiv` (existing TUI client, updated to use serde+bincode IPC). The daemon runs a custom agent loop that calls aisdk.rs for LLM inference, executes tools, and streams results to the client over Unix socket IPC.

**Tech Stack:** Rust, Tokio, aisdk.rs, serde + bincode, ratatui (client unchanged), nix, tracing

**Design Doc:** `docs/plans/2026-02-26-rust-daemon-aisdk-design.md`

---

## Task 1: Cargo Workspace Setup

**Files:**
- Create: `Cargo.toml` (workspace root)
- Move: `codiv/` → `crates/codiv/`
- Create: `crates/codiv-common/Cargo.toml`
- Create: `crates/codiv-common/src/lib.rs`
- Create: `crates/codivd/Cargo.toml`
- Create: `crates/codivd/src/main.rs`

**Step 1: Create workspace root Cargo.toml**

```toml
[workspace]
members = ["crates/codiv", "crates/codiv-common", "crates/codivd"]
resolver = "2"
```

**Step 2: Move codiv into crates/**

```bash
mkdir -p crates
git mv codiv crates/codiv
```

**Step 3: Create codiv-common crate**

```bash
mkdir -p crates/codiv-common/src
```

`crates/codiv-common/Cargo.toml`:
```toml
[package]
name = "codiv-common"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1", features = ["derive"] }
bincode = "1"
```

`crates/codiv-common/src/lib.rs`:
```rust
pub mod config;
pub mod messages;
pub mod types;
```

**Step 4: Create codivd crate skeleton**

```bash
mkdir -p crates/codivd/src
```

`crates/codivd/Cargo.toml`:
```toml
[package]
name = "codivd"
version = "0.1.0"
edition = "2021"

[dependencies]
codiv-common = { path = "../codiv-common" }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
bincode = "1"
nix = { version = "0.29", features = ["signal", "process"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

`crates/codivd/src/main.rs`:
```rust
fn main() {
    println!("codivd daemon placeholder");
}
```

**Step 5: Verify workspace builds**

```bash
cargo build --workspace
```

Expected: All three crates compile successfully.

**Step 6: Commit**

```bash
git add -A
git commit -m "Set up Cargo workspace with codiv, codiv-common, and codivd crates"
```

---

## Task 2: Shared IPC Types in codiv-common

**Files:**
- Create: `crates/codiv-common/src/config.rs`
- Create: `crates/codiv-common/src/messages.rs`
- Create: `crates/codiv-common/src/types.rs`

**Step 1: Write config.rs — shared paths and constants**

`crates/codiv-common/src/config.rs`:
```rust
use std::path::PathBuf;

pub const VERSION: &str = "0.1.0";
pub const FRAME_HEADER_SIZE: usize = 4;
pub const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024; // 16 MiB

pub fn socket_path() -> String {
    format!("/tmp/codivd-{}.sock", unsafe { libc::getuid() })
}

pub fn pid_file_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".codiv/codivd.pid")
}

pub fn log_file_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".codiv/codivd.log")
}

pub fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".codiv")
}
```

Add `libc = "0.2"` to `crates/codiv-common/Cargo.toml` dependencies.

**Step 2: Write messages.rs — IPC message enums**

`crates/codiv-common/src/messages.rs`:
```rust
use serde::{Deserialize, Serialize};

/// A record of a recently executed command, provided by the client as context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandRecord {
    pub command: String,
    pub output: String,
    pub exit_code: i32,
    pub timestamp: u64,
}

/// Session context sent by the client with each AgentRequest.
/// The client manages command execution locally and provides
/// recent history to the daemon for agent context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionContext {
    pub cwd: String,
    pub recent_commands: Vec<CommandRecord>,
    pub env_vars: Vec<(String, String)>,
}

/// Messages sent from the codiv client to the codivd daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Messages sent from the codivd daemon to the codiv client.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    Heartbeat {
        timestamp: u64,
    },
    Error {
        request_id: String,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamChunk {
    Text(String),
    Reasoning(String),
    ToolCall { name: String, arguments: String },
    ToolResult { name: String, result: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

/// Frame a message for IPC transport: 4-byte BE length prefix + bincode payload.
pub fn frame_message<T: Serialize>(msg: &T) -> Result<Vec<u8>, bincode::Error> {
    let payload = bincode::serialize(msg)?;
    let len = payload.len() as u32;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Parse the 4-byte big-endian frame header into a payload length.
pub fn parse_frame_header(header: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*header)
}
```

**Step 3: Write types.rs — shared domain types**

`crates/codiv-common/src/types.rs`:
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentRole {
    TeamLead,
    Engineer,
    Reviewer,
    Researcher,
    Security,
}
```

**Step 4: Verify it compiles**

```bash
cargo build -p codiv-common
```

Expected: PASS

**Step 5: Commit**

```bash
git add crates/codiv-common/
git commit -m "Add codiv-common crate with IPC messages, config, and shared types"
```

---

## Task 3: Rust Daemon — Core Skeleton

**Files:**
- Modify: `crates/codivd/Cargo.toml`
- Create: `crates/codivd/src/main.rs` (full daemon entry point)
- Create: `crates/codivd/src/daemon.rs`
- Create: `crates/codivd/src/session.rs`
- Create: `crates/codivd/src/ipc/mod.rs`
- Create: `crates/codivd/src/ipc/server.rs`

**Step 1: Write daemon main.rs — startup, PID file, daemonize, signal handling**

`crates/codivd/src/main.rs`:
```rust
mod daemon;
mod ipc;
mod session;
mod worker;

use codiv_common::config;
use std::fs;
use std::process;
use tracing::{error, info};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--version") {
        println!("codivd {}", config::VERSION);
        return;
    }

    let foreground = args.iter().any(|a| a == "--foreground" || a == "-f");

    // Set up logging
    let log_path = config::log_file_path();
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).ok();
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "codivd=info".into()),
        )
        .with_writer(move || {
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .unwrap_or_else(|_| fs::File::create("/dev/null").unwrap())
        })
        .init();

    if !foreground {
        daemonize();
    }

    // Check for existing daemon
    let pid_path = config::pid_file_path();
    if let Ok(contents) = fs::read_to_string(&pid_path) {
        if let Ok(pid) = contents.trim().parse::<i32>() {
            if pid > 0 && nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok() {
                error!("daemon already running (pid {})", pid);
                process::exit(1);
            }
        }
        fs::remove_file(&pid_path).ok();
    }

    // Write PID file
    if let Some(parent) = pid_path.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(&pid_path, format!("{}", process::id())).expect("cannot write PID file");

    info!("codivd started (pid {})", process::id());

    // Run daemon
    match daemon::Daemon::new().await {
        Ok(daemon) => {
            daemon.run().await;
        }
        Err(e) => {
            error!("failed to start daemon: {}", e);
            process::exit(1);
        }
    }

    // Cleanup
    fs::remove_file(&pid_path).ok();
    info!("codivd shutdown complete");
}

fn daemonize() {
    use nix::unistd::{fork, setsid, ForkResult};

    match unsafe { fork() } {
        Ok(ForkResult::Parent { .. }) => process::exit(0),
        Ok(ForkResult::Child) => {
            setsid().expect("setsid failed");
        }
        Err(e) => {
            eprintln!("fork failed: {}", e);
            process::exit(1);
        }
    }
}
```

**Step 2: Write session.rs — client session tracking**

`crates/codivd/src/session.rs`:
```rust
use std::time::Instant;

pub struct ClientSession {
    pub env_vars: Vec<(String, String)>,
    pub path: String,
    pub cwd: String,
    pub last_heartbeat: Instant,
}

impl ClientSession {
    pub fn new() -> Self {
        Self {
            env_vars: Vec::new(),
            path: String::new(),
            cwd: String::new(),
            last_heartbeat: Instant::now(),
        }
    }

    pub fn is_stale(&self, timeout_secs: u64) -> bool {
        self.last_heartbeat.elapsed().as_secs() > timeout_secs
    }
}
```

**Step 3: Write IPC server — async Unix socket with length-prefixed framing**

`crates/codivd/src/ipc/mod.rs`:
```rust
pub mod server;
```

`crates/codivd/src/ipc/server.rs`:
```rust
use codiv_common::config;
use codiv_common::messages::{self, ClientMessage, DaemonMessage};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

pub type ClientId = u64;

pub struct IpcMessage {
    pub client_id: ClientId,
    pub message: ClientMessage,
}

pub struct IpcServer {
    listener: UnixListener,
    clients: HashMap<ClientId, mpsc::Sender<Vec<u8>>>,
    next_id: ClientId,
    rx: mpsc::Receiver<IpcMessage>,
    tx: mpsc::Sender<IpcMessage>,
}

impl IpcServer {
    pub async fn new() -> std::io::Result<Self> {
        let path = config::socket_path();
        // Remove stale socket
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        info!("listening on {}", path);

        let (tx, rx) = mpsc::channel(256);
        Ok(Self {
            listener,
            clients: HashMap::new(),
            next_id: 1,
            rx,
            tx,
        })
    }

    pub async fn accept(&mut self) -> Option<ClientId> {
        let (stream, _) = self.listener.accept().await.ok()?;
        let client_id = self.next_id;
        self.next_id += 1;

        let (send_tx, send_rx) = mpsc::channel::<Vec<u8>>(64);
        self.clients.insert(client_id, send_tx);

        let msg_tx = self.tx.clone();
        tokio::spawn(client_connection(client_id, stream, msg_tx, send_rx));

        info!("client {} connected", client_id);
        Some(client_id)
    }

    pub async fn recv(&mut self) -> Option<IpcMessage> {
        self.rx.recv().await
    }

    pub async fn send(&self, client_id: ClientId, msg: &DaemonMessage) {
        if let Some(tx) = self.clients.get(&client_id) {
            match messages::frame_message(msg) {
                Ok(frame) => {
                    if tx.send(frame).await.is_err() {
                        warn!("client {} send channel closed", client_id);
                    }
                }
                Err(e) => error!("failed to serialize message: {}", e),
            }
        }
    }

    pub fn disconnect(&mut self, client_id: ClientId) {
        self.clients.remove(&client_id);
        info!("client {} disconnected", client_id);
    }
}

async fn client_connection(
    client_id: ClientId,
    mut stream: UnixStream,
    msg_tx: mpsc::Sender<IpcMessage>,
    mut send_rx: mpsc::Receiver<Vec<u8>>,
) {
    let (mut reader, mut writer) = stream.split();

    // Spawn writer task
    let writer_handle = tokio::spawn(async move {
        while let Some(frame) = send_rx.recv().await {
            if writer.write_all(&frame).await.is_err() {
                break;
            }
        }
    });

    // Reader loop
    let mut header_buf = [0u8; 4];
    loop {
        if reader.read_exact(&mut header_buf).await.is_err() {
            break;
        }

        let payload_len = messages::parse_frame_header(&header_buf) as usize;
        if payload_len > config::MAX_MESSAGE_SIZE {
            error!("client {}: message too large ({})", client_id, payload_len);
            break;
        }

        let mut payload = vec![0u8; payload_len];
        if reader.read_exact(&mut payload).await.is_err() {
            break;
        }

        match bincode::deserialize::<ClientMessage>(&payload) {
            Ok(message) => {
                if msg_tx
                    .send(IpcMessage { client_id, message })
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Err(e) => {
                error!("client {}: failed to deserialize: {}", client_id, e);
            }
        }
    }

    writer_handle.abort();
}
```

**Step 4: Write daemon.rs — event loop and message dispatch**

`crates/codivd/src/daemon.rs`:
```rust
use crate::ipc::server::{ClientId, IpcServer};
use crate::session::ClientSession;
use crate::worker;
use codiv_common::messages::{ClientMessage, DaemonMessage};
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tracing::{error, info};

pub struct Daemon {
    ipc: IpcServer,
    sessions: HashMap<ClientId, ClientSession>,
}

impl Daemon {
    pub async fn new() -> std::io::Result<Self> {
        let ipc = IpcServer::new().await?;
        Ok(Self {
            ipc,
            sessions: HashMap::new(),
        })
    }

    pub async fn run(mut self) {
        let mut cleanup_interval = tokio::time::interval(Duration::from_secs(30));

        loop {
            tokio::select! {
                Some(client_id) = self.ipc.accept() => {
                    self.sessions.insert(client_id, ClientSession::new());
                }
                Some(msg) = self.ipc.recv() => {
                    self.dispatch(msg.client_id, msg.message).await;
                }
                _ = cleanup_interval.tick() => {
                    self.cleanup_stale_sessions().await;
                }
            }
        }
    }

    async fn dispatch(&mut self, client_id: ClientId, msg: ClientMessage) {
        match msg {
            // Note: The client never sends commands for the daemon to execute.
            // The worker module (worker.rs) is used internally by the agent's
            // Bash tool, not triggered by IPC messages.

            ClientMessage::AgentRequest {
                prompt,
                request_id,
                context,
            } => {
                // Phase 2 step 2: agent integration (Task 5+)
                // The SessionContext provides cwd, recent command history,
                // and environment variables from the client.
                self.ipc
                    .send(
                        client_id,
                        &DaemonMessage::Error {
                            request_id,
                            message: "agent not yet implemented".to_string(),
                        },
                    )
                    .await;
            }

            ClientMessage::EnvSnapshot {
                env_vars,
                path,
                cwd,
            } => {
                if let Some(session) = self.sessions.get_mut(&client_id) {
                    session.env_vars = env_vars;
                    session.path = path;
                    session.cwd = cwd;
                    info!("received env snapshot from client {}", client_id);
                }
            }

            ClientMessage::Heartbeat { timestamp: _ } => {
                if let Some(session) = self.sessions.get_mut(&client_id) {
                    session.last_heartbeat = std::time::Instant::now();
                }
                let ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                self.ipc
                    .send(client_id, &DaemonMessage::Heartbeat { timestamp: ts })
                    .await;
            }

            ClientMessage::Shutdown { reason } => {
                info!("shutdown requested: {}", reason);
                std::process::exit(0);
            }

            ClientMessage::Confirmation { .. } => {
                // Phase 2 step 2: safety confirmation (Task 7+)
            }
        }
    }

    async fn cleanup_stale_sessions(&mut self) {
        let stale: Vec<ClientId> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.is_stale(120))
            .map(|(id, _)| *id)
            .collect();

        for id in stale {
            info!("cleaning stale session {}", id);
            self.sessions.remove(&id);
            self.ipc.disconnect(id);
        }
    }
}
```

**Step 5: Write worker.rs — async command execution (used internally by agent tools)**

The worker module is used internally by the agent's Bash tool for executing shell commands as part of the agent's tool loop. It is NOT triggered by IPC messages from the client. Command results flow back to the client as `AgentStreamChunk` messages (ToolCall/ToolResult variants).

`crates/codivd/src/worker.rs`:
```rust
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;
use tracing::error;

/// Execute a shell command internally for the agent's Bash tool.
/// Streams stdout/stderr through the channel. Each message is (data, is_stderr).
pub async fn execute_command(
    command: &str,
    cwd: &str,
    env: &[(String, String)],
    tx: mpsc::Sender<(Vec<u8>, bool)>,
) -> i32 {
    let mut child = match Command::new("bash")
        .args(["-c", command])
        .current_dir(cwd)
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            error!("failed to spawn worker: {}", e);
            return 127;
        }
    };

    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();

    let tx_err = tx.clone();
    let stderr_handle = tokio::spawn(async move {
        let mut buf = [0u8; 8192];
        loop {
            match stderr.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if tx_err.send((buf[..n].to_vec(), true)).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let mut buf = [0u8; 8192];
    loop {
        match stdout.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                if tx.send((buf[..n].to_vec(), false)).await.is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    let _ = stderr_handle.await;

    match child.wait().await {
        Ok(status) => status.code().unwrap_or(1),
        Err(_) => 1,
    }
}
```

**Step 6: Verify it compiles**

```bash
cargo build -p codivd
```

Expected: PASS

**Step 7: Commit**

```bash
git add crates/codivd/
git commit -m "Add Rust codivd daemon with Tokio event loop, IPC server, and worker execution"
```

---

## Task 4: Migrate codiv Client to serde+bincode IPC

**Files:**
- Modify: `crates/codiv/Cargo.toml` — remove flatbuffers, add codiv-common + bincode + serde
- Rewrite: `crates/codiv/src/ipc/messages.rs` — use codiv-common message types
- Modify: `crates/codiv/src/ipc/client.rs` — use bincode deserialization
- Modify: `crates/codiv/src/app.rs` — use codiv-common config
- Delete: `crates/codiv/build.rs` (FlatBuffers codegen)
- Modify: `crates/codiv/src/ipc/daemon_launcher.rs` — use codiv-common paths

**Step 1: Update codiv/Cargo.toml**

Remove `flatbuffers = "24.12.23"`. Add:
```toml
codiv-common = { path = "../codiv-common" }
serde = { version = "1", features = ["derive"] }
bincode = "1"
```

**Step 2: Delete build.rs**

The FlatBuffers codegen build script is no longer needed.

```bash
rm crates/codiv/build.rs
```

**Step 3: Rewrite ipc/messages.rs — use codiv-common**

Replace the entire file with a thin wrapper that re-exports codiv-common types and provides the same API the rest of codiv expects:

`crates/codiv/src/ipc/messages.rs`:
```rust
pub use codiv_common::config::{FRAME_HEADER_SIZE, MAX_MESSAGE_SIZE};
pub use codiv_common::messages::{
    frame_message, parse_frame_header, ClientMessage, DaemonMessage, SessionContext,
    CommandRecord, StreamChunk,
};

use std::time::{SystemTime, UNIX_EPOCH};

/// Build a framed EnvSnapshot message from session data.
pub fn build_env_snapshot(
    env_vars: &[(String, String)],
    path: &str,
    cwd: &str,
) -> Option<Vec<u8>> {
    let msg = ClientMessage::EnvSnapshot {
        env_vars: env_vars.to_vec(),
        path: path.to_string(),
        cwd: cwd.to_string(),
    };
    frame_message(&msg).ok()
}

/// Build a framed Heartbeat message.
pub fn build_heartbeat() -> Option<Vec<u8>> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let msg = ClientMessage::Heartbeat { timestamp: ts };
    frame_message(&msg).ok()
}

/// Build a framed AgentRequest message with session context.
/// The client attaches recent command history, cwd, and environment
/// so the agent has full context without needing to execute commands itself.
pub fn build_agent_request(
    prompt: &str,
    request_id: &str,
    context: SessionContext,
) -> Option<Vec<u8>> {
    let msg = ClientMessage::AgentRequest {
        prompt: prompt.to_string(),
        request_id: request_id.to_string(),
        context,
    };
    frame_message(&msg).ok()
}

/// Build a framed Shutdown message.
pub fn build_shutdown(reason: &str) -> Option<Vec<u8>> {
    let msg = ClientMessage::Shutdown {
        reason: reason.to_string(),
    };
    frame_message(&msg).ok()
}
```

**Step 4: Update ipc/client.rs — bincode deserialization**

In `crates/codiv/src/ipc/client.rs`, the `reader_loop` function currently uses FlatBuffers to parse incoming messages. Replace the parsing logic:

Find the FlatBuffers parsing in reader_loop (around line 66-100) and replace with:
```rust
// In the reader loop, after reading the payload bytes:
match bincode::deserialize::<DaemonMessage>(&payload_buf) {
    Ok(msg) => {
        if tx.send(msg).is_err() {
            break;
        }
    }
    Err(e) => {
        log::error!("failed to deserialize daemon message: {}", e);
    }
}
```

The `DaemonMessage` enum has variants for agent streaming (AgentStreamChunk, AgentComplete), confirmations, heartbeats, and errors.

**Step 5: Update daemon_launcher.rs — use codiv-common paths**

Replace the hardcoded `socket_path()` and `pid_file_path()` functions with imports from `codiv_common::config`:

```rust
use codiv_common::config::{socket_path, pid_file_path};
```

Remove the local `socket_path()` and `pid_file_path()` functions.

**Step 6: Update app.rs — use codiv-common config**

Replace the IPC import to use codiv-common paths. The `connect_to_daemon` call uses the socket path from config.

**Step 7: Verify it compiles and tests pass**

```bash
cargo build -p codiv
cargo test -p codiv
```

Expected: PASS (existing tests should still work since the PTY/sentinel logic is untouched)

**Step 8: Commit**

```bash
git add crates/codiv/
git commit -m "Migrate codiv client IPC from FlatBuffers to serde+bincode via codiv-common"
```

---

## Task 5: Integration Test — Client ↔ Daemon Round-Trip

**Files:**
- Create: `crates/codivd/tests/ipc_roundtrip.rs`

**Step 1: Write the integration test**

This test starts the daemon, connects the IPC client, sends messages, and verifies responses:

`crates/codivd/tests/ipc_roundtrip.rs`:
```rust
use codiv_common::messages::{ClientMessage, DaemonMessage, frame_message, parse_frame_header};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

#[tokio::test]
async fn test_heartbeat_roundtrip() {
    // Use a unique socket path for this test
    let socket_path = format!("/tmp/codivd-test-{}.sock", std::process::id());
    let _ = std::fs::remove_file(&socket_path);

    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

    // Simulate daemon accepting and responding to heartbeat
    let socket_path_clone = socket_path.clone();
    let server_handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut header = [0u8; 4];
        stream.read_exact(&mut header).await.unwrap();
        let len = parse_frame_header(&header) as usize;
        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload).await.unwrap();

        let msg: ClientMessage = bincode::deserialize(&payload).unwrap();
        assert!(matches!(msg, ClientMessage::Heartbeat { .. }));

        // Send heartbeat response
        let response = DaemonMessage::Heartbeat { timestamp: 12345 };
        let frame = frame_message(&response).unwrap();
        stream.write_all(&frame).await.unwrap();
    });

    // Connect client
    tokio::time::sleep(Duration::from_millis(50)).await;
    let mut stream = UnixStream::connect(&socket_path).await.unwrap();

    // Send heartbeat
    let hb = ClientMessage::Heartbeat { timestamp: 1000 };
    let frame = frame_message(&hb).unwrap();
    stream.write_all(&frame).await.unwrap();

    // Read response
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).await.unwrap();
    let len = parse_frame_header(&header) as usize;
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).await.unwrap();

    let response: DaemonMessage = bincode::deserialize(&payload).unwrap();
    assert!(matches!(response, DaemonMessage::Heartbeat { timestamp: 12345 }));

    server_handle.await.unwrap();
    let _ = std::fs::remove_file(&socket_path);
}
```

**Step 2: Run the test**

```bash
cargo test -p codivd --test ipc_roundtrip
```

Expected: PASS

**Step 3: Commit**

```bash
git add crates/codivd/tests/
git commit -m "Add IPC round-trip integration test for heartbeat"
```

---

## Task 6: aisdk.rs Integration — First LLM Call

**Files:**
- Modify: `crates/codivd/Cargo.toml` — add aisdk dependency
- Create: `crates/codivd/src/agent/mod.rs`
- Create: `crates/codivd/src/agent/config.rs`
- Create: `crates/codivd/src/agent/agent.rs`
- Modify: `crates/codivd/src/main.rs` — add agent module

Note: The `aisdk` crate must be available. Check its actual crate name and registry availability. If it's not on crates.io, it may need to be added as a git dependency from the lazy-hq/aisdk repo.

**Step 1: Add aisdk dependency**

Add to `crates/codivd/Cargo.toml`:
```toml
# Check actual crate name — may be 'aisdk' or need git source
aisdk = { git = "https://github.com/lazy-hq/aisdk.git", features = ["openai", "anthropic", "google"] }
futures = "0.3"
```

**Step 2: Write agent config — model catalog loading**

`crates/codivd/src/agent/mod.rs`:
```rust
pub mod agent;
pub mod config;
```

`crates/codivd/src/agent/config.rs`:
```rust
use serde::Deserialize;
use codiv_common::types::AgentRole;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct ModelCatalog {
    pub default_provider: String,
    pub roles: HashMap<String, ModelAssignment>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelAssignment {
    pub provider: String,
    pub model: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<u8>,
}

impl ModelCatalog {
    pub fn load() -> Self {
        let config_path = codiv_common::config::config_dir().join("models.toml");
        if let Ok(contents) = std::fs::read_to_string(&config_path) {
            if let Ok(catalog) = toml::from_str(&contents) {
                return catalog;
            }
        }

        // Default: Anthropic Claude for everything
        Self {
            default_provider: "anthropic".to_string(),
            roles: HashMap::new(),
        }
    }

    pub fn assignment_for(&self, role: &AgentRole) -> ModelAssignment {
        let key = match role {
            AgentRole::TeamLead => "team_lead",
            AgentRole::Engineer => "engineer",
            AgentRole::Reviewer => "reviewer",
            AgentRole::Researcher => "researcher",
            AgentRole::Security => "security",
        };

        self.roles.get(key).cloned().unwrap_or(ModelAssignment {
            provider: self.default_provider.clone(),
            model: "claude-sonnet-4-5-20250514".to_string(),
            max_tokens: Some(8192),
            temperature: None,
        })
    }
}
```

**Step 3: Write agent.rs — single-agent loop skeleton**

`crates/codivd/src/agent/agent.rs`:
```rust
use crate::agent::config::ModelAssignment;
use codiv_common::messages::{DaemonMessage, StreamChunk};
use codiv_common::types::AgentRole;
use tracing::info;

pub enum AgentStep {
    ToolsExecuted,
    Complete(String),
    Error(String),
}

pub struct Agent {
    pub role: AgentRole,
    pub system_prompt: String,
    pub model_config: ModelAssignment,
    pub messages: Vec<AgentMessage>,
}

pub struct AgentMessage {
    pub role: MessageRole,
    pub content: String,
}

pub enum MessageRole {
    System,
    User,
    Assistant,
}

impl Agent {
    pub fn new(role: AgentRole, model_config: ModelAssignment, system_prompt: String) -> Self {
        Self {
            role,
            system_prompt,
            model_config,
            messages: Vec::new(),
        }
    }

    pub fn add_user_message(&mut self, content: &str) {
        self.messages.push(AgentMessage {
            role: MessageRole::User,
            content: content.to_string(),
        });
    }

    /// Run the agent loop: LLM call → tool execution → repeat until done.
    /// This is a placeholder that will be wired to aisdk.rs in the next step.
    pub async fn run(&mut self) -> Result<String, String> {
        info!(
            "agent {:?} running with model {}/{}",
            self.role, self.model_config.provider, self.model_config.model
        );

        // TODO: Wire to aisdk.rs LanguageModelRequest
        // For now, return a placeholder
        Ok(format!(
            "Agent {:?} would process with {}/{}",
            self.role, self.model_config.provider, self.model_config.model
        ))
    }
}
```

**Step 4: Add agent module to main.rs**

Add `mod agent;` to `crates/codivd/src/main.rs` module declarations.

**Step 5: Verify it compiles**

```bash
cargo build -p codivd
```

Expected: PASS (aisdk dependency resolves and compiles)

Note: If the aisdk crate is not available via git, this step will fail. In that case, check the reference docs in `references/aisdk.rs/` for the correct dependency source. The implementation may need to be adjusted based on the actual crate's API.

**Step 6: Commit**

```bash
git add crates/codivd/src/agent/
git commit -m "Add agent module with model catalog config and single-agent loop skeleton"
```

---

## Task 7: Wire Agent to Daemon — AgentRequest → AgentComplete Flow

**Files:**
- Modify: `crates/codivd/src/daemon.rs` — handle AgentRequest, spawn agent task
- Modify: `crates/codiv/src/ui/terminal.rs` — send AgentRequest on AI input, render AgentStreamChunk
- Modify: `crates/codiv/src/shell/command_index.rs` — ensure AiQuery classification triggers AgentRequest

**Step 1: Handle AgentRequest in daemon.rs**

In `daemon.rs`, replace the `ClientMessage::AgentRequest { .. }` placeholder:

```rust
ClientMessage::AgentRequest { prompt, request_id, context } => {
    let model_catalog = agent::config::ModelCatalog::load();
    let assignment = model_catalog.assignment_for(&codiv_common::types::AgentRole::Engineer);

    let ipc = /* clone sender for client_id */;
    let rid = request_id.clone();

    // The SessionContext provides the agent with cwd, recent command
    // history, and environment variables from the client session.
    tokio::spawn(async move {
        let mut agent = agent::agent::Agent::new(
            codiv_common::types::AgentRole::Engineer,
            assignment,
            "You are a helpful coding assistant. Answer concisely.".to_string(),
        );
        agent.set_session_context(&context);
        agent.add_user_message(&prompt);

        match agent.run().await {
            Ok(response) => {
                // Stream the response text
                ipc.send(client_id, &DaemonMessage::AgentStreamChunk {
                    request_id: rid.clone(),
                    chunk: StreamChunk::Text(response.clone()),
                }).await;

                // Agent tool output (e.g. Bash tool results) streams via
                // AgentStreamChunk with ToolCall/ToolResult variants, NOT
                // through separate CommandOutput messages.

                ipc.send(client_id, &DaemonMessage::AgentComplete {
                    request_id: rid,
                    summary: response,
                }).await;
            }
            Err(e) => {
                ipc.send(client_id, &DaemonMessage::Error {
                    request_id: rid,
                    message: e,
                }).await;
            }
        }
    });
}
```

**Step 2: Update codiv client to send AgentRequest**

In `crates/codiv/src/ui/terminal.rs`, find the `InputAction::AiQuery` handler (the `"?"` prefix classification) and send an AgentRequest over IPC instead of displaying a placeholder:

```rust
InputAction::AiQuery => {
    if let Some(ref mut client) = daemon_client {
        let request_id = format!("agent-{}", rand::random::<u64>());
        // Build SessionContext with recent command history, cwd, and env
        let context = SessionContext {
            cwd: current_cwd.clone(),
            recent_commands: recent_command_history.clone(),
            env_vars: current_env_vars.clone(),
        };
        let msg = ClientMessage::AgentRequest {
            prompt: raw_input.clone(),
            request_id,
            context,
        };
        if let Ok(frame) = frame_message(&msg) {
            client.send(&frame);
        }
    }
}
```

**Step 3: Handle AgentStreamChunk in client's message processing**

In the daemon message handler in terminal.rs, add cases for the new message types:

```rust
DaemonMessage::AgentStreamChunk { request_id, chunk } => {
    match chunk {
        StreamChunk::Text(text) => {
            // Render text to the terminal output area
            parser_push_styled(&mut vt_parser, &text);
        }
        _ => {}
    }
}
DaemonMessage::AgentComplete { request_id, summary } => {
    // Agent is done — show completion indicator
    parser_push_styled(&mut vt_parser, "\n[agent complete]\n");
}
```

**Step 4: Verify it compiles**

```bash
cargo build --workspace
```

Expected: PASS

**Step 5: Commit**

```bash
git add crates/codivd/src/daemon.rs crates/codiv/src/ui/terminal.rs crates/codiv/src/shell/command_index.rs
git commit -m "Wire AgentRequest flow from client through daemon to agent and back"
```

---

## Task 8: Delete C++ Daemon and FlatBuffers Schema

**Files:**
- Delete: `codivd/` (entire C++ daemon directory)
- Delete: `schemas/ipc.fbs`
- Modify: `Makefile` — remove codivd build targets if any

**Step 1: Remove C++ daemon source**

```bash
git rm -r codivd/src/ codivd/CMakeLists.txt
```

Keep `codivd/build/` in .gitignore if not already tracked.

**Step 2: Remove FlatBuffers schema**

```bash
git rm schemas/ipc.fbs
rmdir schemas 2>/dev/null || true
```

**Step 3: Update Makefile if needed**

Check if the root Makefile has targets for the C++ build. Remove or update them to use `cargo build --workspace`.

**Step 4: Verify workspace still builds**

```bash
cargo build --workspace
cargo test --workspace
```

Expected: PASS

**Step 5: Commit**

```bash
git add -A
git commit -m "Remove C++ codivd daemon and FlatBuffers schema — fully replaced by Rust"
```

---

## Task 9: End-to-End Smoke Test

**Files:**
- No new files — manual verification

**Step 1: Build the full workspace**

```bash
cargo build --workspace --release
```

**Step 2: Start the daemon in foreground**

```bash
./target/release/codivd --foreground
```

Verify: PID file created at `~/.codiv/codivd.pid`, log output to `~/.codiv/codivd.log`.

**Step 3: Start the client**

In another terminal:
```bash
./target/release/codiv
```

Verify:
- Client connects to daemon (daemon logs "client 1 connected")
- Shell commands execute normally (via local PTY, not daemon)
- Env snapshot sent on startup
- Heartbeats exchanged

**Step 4: Test AI query (if aisdk.rs is wired)**

In the codiv client:
```
? what is 2+2
```

Verify: Agent placeholder response appears.

**Step 5: Kill daemon, verify client handles disconnect**

```bash
kill $(cat ~/.codiv/codivd.pid)
```

Verify: Client continues working (shell commands still execute via local PTY). Daemon disconnect logged.

**Step 6: Commit any fixes**

```bash
git add -A
git commit -m "Fix issues found during end-to-end smoke testing"
```

---

## Summary

| Task | Description | Key Deliverable |
|------|-------------|-----------------|
| 1 | Cargo workspace setup | 3-crate workspace builds |
| 2 | Shared IPC types | `codiv-common` with serde messages |
| 3 | Rust daemon skeleton | Tokio event loop, IPC server, worker execution |
| 4 | Client IPC migration | FlatBuffers → serde+bincode |
| 5 | Integration test | Heartbeat round-trip test |
| 6 | aisdk.rs integration | Agent module, model catalog |
| 7 | Agent request flow | End-to-end AgentRequest → AgentComplete |
| 8 | Delete C++ daemon | Remove codivd/ C++ and FlatBuffers |
| 9 | End-to-end smoke test | Manual verification of full system |
