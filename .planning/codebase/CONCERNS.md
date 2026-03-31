# Codebase Concerns

**Analysis Date:** 2026-03-31

## Tech Debt

**Weak UUID generator in permissions.rs:**
- Issue: `uuid_v4()` in `crates/codivd/src/agent/permissions.rs` (lines 277–284) is a hand-rolled function using `SystemTime::now()` XOR'd with `process::id()`. It is not cryptographically random, produces non-standard hex format (two 16-char hex words, not the 8-4-4-4-12 UUID format), and could collide under high concurrency or within the same millisecond.
- Files: `crates/codivd/src/agent/permissions.rs`
- Impact: Confirmation `request_id` values may collide, causing confirmation responses to resolve the wrong pending request. The `uuid` crate is already available in the workspace (used in `daemon.rs` and `session.rs`).
- Fix approach: Replace `uuid_v4()` with `uuid::Uuid::new_v4().to_string()`, the same pattern used everywhere else in the codebase.

**`LoadSession` uses a linear scan to look up session name:**
- Issue: `ClientMessage::LoadSession` handler in `crates/codivd/src/daemon.rs` (lines 555–558) calls `store.list_sessions(100)` to find the name of a specific session, which fetches up to 100 sessions and iterates to find the target. This is a redundant full scan instead of a direct lookup.
- Files: `crates/codivd/src/daemon.rs`, `crates/codivd/src/store.rs`
- Impact: Minor inefficiency; silently fails to find the name if the session is older than the 100-session limit.
- Fix approach: Add a `SessionStore::get_session(id)` method that queries by primary key.

**`AppConfig::load()` called inside `wrap_with_permissions` on every tool call:**
- Issue: `crates/codivd/src/agent/permissions.rs` line 85 calls `crate::agent::config::AppConfig::load()` synchronously (reading and parsing `~/.codiv/config.toml` from disk) on every permission check, including the hot path for Low-risk `Allow` decisions.
- Files: `crates/codivd/src/agent/permissions.rs`
- Impact: Unnecessary disk I/O per tool call. The config is already loaded and watched via `Arc<RwLock<AppConfig>>` in the daemon; the permission wrapper does not have access to that shared reference and re-loads from disk instead.
- Fix approach: Thread the `Arc<RwLock<AppConfig>>` into `PermissionContext` and read from it instead of calling `AppConfig::load()`.

**`fork_session` and `append_events` are marked `#[allow(dead_code)]`:**
- Issue: `crates/codivd/src/store.rs` (lines 131, 195) marks `append_events` and `fork_session` as dead code. These were likely built for a planned branching feature that is not yet used.
- Files: `crates/codivd/src/store.rs`
- Impact: Accumulating unused code inflates the store module without contributing functionality.
- Fix approach: Remove if the feature is not planned imminently; otherwise add a tracking issue.

**Hardcoded context-window sizes in `model_alias()` / `context_window()`:**
- Issue: `crates/codivd/src/agent/config.rs` (lines 270–302) uses a `match` table mapping model name strings to hardcoded alias and context-window numbers. Any new model or name variant requires a code change.
- Files: `crates/codivd/src/agent/config.rs`
- Impact: Model additions/removals require code changes; the fallback `_ => 200_000` can silently over-report context window size for models with smaller limits (e.g., a 32K model).
- Fix approach: Move model metadata into the `[models.roles.*]` config, or into a separate `[model_info]` section in `config.toml`, so no rebuild is needed to support new models.

**`ShellBackend` is an enum with only one variant:**
- Issue: `crates/codivd/src/agent/shell_backend.rs` defines `ShellBackend` as an enum with only `ClientRelay`. The comment (`// not yet wired`) hints a `DaemonShell` variant was planned.
- Files: `crates/codivd/src/agent/shell_backend.rs`
- Impact: The enum adds indirection without the polymorphism benefit it was designed for.
- Fix approach: Either add the second variant or simplify to a plain struct.

**Agent history truncation silently discards oldest events:**
- Issue: `crates/codivd/src/agent/agent.rs` (lines 94–98) enforces a hard `MAX_HISTORY_EVENTS = 100` limit by draining the front of the `Vec`. This can silently drop tool results in the middle of multi-step tasks that exceed 100 events.
- Files: `crates/codivd/src/agent/agent.rs`
- Impact: Mid-task history pruning may cause the LLM to lose context about earlier steps; the compaction system was added to address this, but the raw limit can still fire before compaction triggers.
- Fix approach: Either increase the limit to something much larger (compaction is the intended mechanism) or ensure compaction always fires before the raw limit is hit.

**`dead_code` fields on UI block structs:**
- Issue: `crates/codiv/src/ui/blocks.rs` uses `#[allow(dead_code)]` on `PromptBlock.id`, `PromptBlock.text`, `CmdResponseBlock.id`, `CmdResponseBlock.command`, `CmdResponseBlock.exit_code`, `AiResponseBlock.id`, and `PendingToolCall.name`. These fields carry data that is stored but never read.
- Files: `crates/codiv/src/ui/blocks.rs`
- Impact: Wasted memory per block; indicates incomplete rendering features (e.g., block IDs for keyboard navigation were added but not used).
- Fix approach: Either wire the fields into the rendering/navigation code or remove them and the `#[allow(dead_code)]` annotations.

---

## Security Considerations

**API keys are stored in plaintext config file:**
- Risk: `~/.codiv/config.toml` accepts `api_key` fields under `[providers.*]` and `[models.roles.*]`. If the file has loose permissions or is read by another process, keys are exposed.
- Files: `crates/codivd/src/agent/config.rs`
- Current mitigation: None. The `ProviderConfig::fmt` Debug impl redacts the key in log output, but the file itself is unprotected.
- Recommendations: Document recommended file permissions (chmod 600); consider supporting keychain/secret-store backends; add a startup warning if the file is world-readable.

**Permission evaluator LLM responses are parsed with `.contains("ALLOW")`:**
- Risk: `crates/codivd/src/agent/llm_evaluator.rs` (line 52) checks `response.contains("ALLOW")`. A model that produces verbose output mentioning "ALLOW" in a different context (e.g., "I cannot ALLOW this") would be misinterpreted as a grant.
- Files: `crates/codivd/src/agent/llm_evaluator.rs`
- Current mitigation: The system prompt says "Respond with EXACTLY one word: ALLOW or DENY", but this is not enforced structurally.
- Recommendations: Trim the response and compare exactly: `response.trim() == "ALLOW"`. If the model returns anything other than "ALLOW", default to `Prompt`.

**No authentication on the daemon Unix socket:**
- Risk: The IPC socket at `/tmp/codivd-{uid}.sock` is accessible to any process running as the same user. Any process can send arbitrary `ClientMessage` frames including `Shutdown`, `AgentRequest`, or `CommandExecutionResult`.
- Files: `crates/codivd/src/ipc/server.rs`, `crates/codiv-common/src/messages.rs`
- Current mitigation: Socket is per-UID (in `/tmp`). A process already running as the user has equivalent access anyway.
- Recommendations: Acceptable risk for a single-user local tool. Document the threat model; consider path in `$XDG_RUNTIME_DIR` on Linux for better isolation.

**`ClientMessage::Shutdown` calls `std::process::exit(0)` directly:**
- Risk: Any connected client can terminate the daemon immediately via `ClientMessage::Shutdown`, which calls `std::process::exit(0)` without cleanup (`crates/codivd/src/daemon.rs` line 432). Pending IPC sends, SQLite writes, and in-flight agent tasks are abandoned.
- Files: `crates/codivd/src/daemon.rs`
- Current mitigation: In practice only the official TUI client sends this message.
- Recommendations: Transition to a graceful shutdown path (set a flag, let the event loop exit cleanly) instead of calling `exit(0)`.

---

## Performance Bottlenecks

**`cleanup_stale_sessions` scans all sessions every 30 seconds:**
- Problem: `crates/codivd/src/daemon.rs` (line 95) fires a `cleanup_interval` every 30 seconds, which iterates all active sessions to check heartbeat staleness.
- Files: `crates/codivd/src/daemon.rs`
- Cause: O(N) scan; fine for small N but worth noting as session count scales.
- Improvement path: Track a min-heap of (expiry_time, client_id) to compute the earliest possible stale time and skip the scan otherwise.

**`block_in_place` + `block_on` inside every permission-checked tool call:**
- Problem: `crates/codivd/src/agent/permissions.rs` (lines 100–103, 160–163) uses `tokio::task::block_in_place` + `rt.block_on(...)` to bridge async confirmation into the synchronous tool closure. This blocks a tokio worker thread for the duration of user interaction (up to 60 seconds).
- Files: `crates/codivd/src/agent/permissions.rs`
- Cause: Tools are currently `Fn(Value) -> Result<String, String>` (sync), so async bridging is required.
- Improvement path: Migrate tool signatures to `async fn` or use a dedicated blocking thread pool (`tokio::task::spawn_blocking`) to avoid stalling tokio workers.

**LLM evaluator spawns a new single-threaded tokio runtime per call:**
- Problem: `crates/codivd/src/agent/permissions.rs` (lines 134–145) spawns a new `std::thread`, builds a `tokio::runtime::Builder::new_current_thread()` runtime, and blocks on it for each `LlmEvaluate` permission decision.
- Files: `crates/codivd/src/agent/permissions.rs`
- Cause: Workaround for `tokio::time::timeout` not firing inside `block_in_place + block_on`; creates a fresh runtime every call.
- Improvement path: Move the evaluator out of the sync closure path entirely; pre-resolve LLM evaluation asynchronously before the tool closure fires.

---

## Fragile Areas

**IPC message framing — any field rename breaks wire compatibility:**
- Files: `crates/codiv-common/src/messages.rs`, `crates/codiv-common/src/conversation.rs`
- Why fragile: `ClientMessage` and `DaemonMessage` are serialized with `bincode` (length-prefixed). Adding or renaming enum variants, or reordering fields, silently breaks a running daemon talking to a stale client binary (or vice versa).
- Safe modification: Always bump the version string in `Cargo.toml`; ensure `codiv` and `codivd` binaries are always deployed together. Consider adding a handshake version check on connect.
- Test coverage: One integration test (`crates/codivd/tests/ipc_roundtrip.rs`) covers basic framing but not schema evolution.

**`LoadSession` loads all events into agent history without applying `enforce_limit`:**
- Files: `crates/codivd/src/daemon.rs` (lines 562–573)
- Why fragile: `agent.add_tool_events(&events)` bypasses `enforce_limit()` (the call path for `add_tool_events` does call `enforce_limit`, but a large session with >100 events will immediately trigger the truncation, potentially losing recent context from the loaded session).
- Safe modification: Ensure compaction is applied before loading into a new agent, or load only the last N events.
- Test coverage: No automated test for loading a session with >100 events.

**PTY global lock in `pty_io.rs`:**
- Files: `crates/codiv-common/src/shell/pty_io.rs` (lines 167, 180, 192, 205)
- Why fragile: A `static PTY_LOCK: Mutex<()>` serializes all PTY read/write operations across threads. Any panic while holding the lock will poison it, crashing subsequent PTY operations with a `PoisonError`.
- Safe modification: Use `lock().unwrap_or_else(|p| p.into_inner())` (poison recovery) or replace with an atomics-based approach.
- Test coverage: No tests for concurrent PTY access or lock poisoning.

**`sentinel` protocol assumes no command echoes the sentinel string:**
- Files: `crates/codiv-common/src/shell/mod.rs`
- Why fragile: `generate_sentinel()` uses `rand::thread_rng()` with a 64-bit random suffix, making collisions astronomically unlikely but not impossible. More practically, a command that produces extremely large output before the sentinel can cause `read_until_sentinel` to buffer unbounded data.
- Safe modification: Add a hard buffer cap in `read_until_sentinel` implementations.
- Test coverage: `mod.rs` has unit tests for sentinel detection and ANSI stripping but no stress tests with large outputs.

---

## Scaling Limits

**In-memory `HashMap<ClientId, ClientSession>` for all sessions:**
- Current capacity: All sessions live in a single `HashMap` in the `Daemon` struct.
- Limit: Not a practical concern for a single-user local tool, but the map is never pruned except by 120-second heartbeat staleness. A client that disconnects ungracefully (no Shutdown message) keeps its session alive for 2 minutes.
- Scaling path: The 120-second timeout is already in place; acceptable for current use.

**SQLite sessions.db — no event pruning:**
- Current capacity: All conversation events for all sessions are stored indefinitely.
- Limit: For heavy users the database will grow without bound.
- Scaling path: Add a `sessions.db` retention policy (e.g., delete sessions older than N days, or keep only the last M sessions).

---

## Dependencies at Risk

**`aisdk` and `streamdown-rs` are forked git dependencies:**
- Risk: Both are pinned to the author's forks (`razorback16/aisdk`, `razorback16/streamdown-rs`) without a published crate version. Any breaking change upstream requires a manual fork update; no semver guarantees.
- Impact: Multi-model streaming, tool-call handling, and markdown rendering are tightly coupled to these forks. A stale fork may miss security patches or API compatibility fixes.
- Migration plan: Publish the fork to crates.io or lock to a specific git revision (not just branch HEAD) to prevent silent upstream changes from breaking the build.

---

## Test Coverage Gaps

**No integration tests for the full agent tool-call cycle:**
- What's not tested: The end-to-end path from `ClientMessage::AgentRequest` → LLM stream → tool execution → `ClientMessage::CommandExecutionResult` → `DaemonMessage::AgentComplete` is not covered by any test.
- Files: `crates/codivd/src/daemon.rs`, `crates/codivd/src/agent/agent.rs`, `crates/codivd/src/agent/tools.rs`
- Risk: Regressions in the agentic loop (e.g., tool events not persisted, compaction triggered at wrong time) are only caught manually.
- Priority: High

**No tests for permission system under concurrent tool calls:**
- What's not tested: Multiple simultaneous confirmation prompts, LLM evaluator timeout/panic paths, and allowlist/denylist persistence are not exercised by tests.
- Files: `crates/codivd/src/agent/permissions.rs`, `crates/codivd/src/agent/llm_evaluator.rs`
- Risk: Confirmation channel errors silently deny tool calls; allowlist persistence writes may race with config watcher reloads.
- Priority: High

**No tests for `LoadSession` with large histories or mismatched event counts:**
- What's not tested: Loading a session with >100 events (triggering `enforce_limit` truncation), loading a compacted session (with a `Summary` event), and loading a session while an agent task is running.
- Files: `crates/codivd/src/daemon.rs`, `crates/codivd/src/store.rs`
- Risk: Silent context loss on session reload.
- Priority: Medium

**E2E tests are screenshot-based and not run in CI:**
- What's not tested: `crates/codiv/tests/e2e_paste.rs` compares against PNG screenshots stored in `crates/codiv/tests/screenshots/`. There is no indication these run in an automated CI pipeline; screenshot drift can cause false failures or mask real regressions.
- Files: `crates/codiv/tests/e2e_paste.rs`
- Risk: UI regressions go undetected between screenshot updates.
- Priority: Medium

---

*Concerns audit: 2026-03-31*
