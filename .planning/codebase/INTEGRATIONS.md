# External Integrations

**Analysis Date:** 2026-03-31

## APIs & External Services

**LLM Providers (all via `aisdk` in `crates/codivd/`):**

- **Anthropic** — Claude models (default provider; claude-sonnet-4-5, claude-opus-4, claude-haiku-3-5)
  - SDK/Client: `aisdk` feature `anthropic` → `aisdk::providers::Anthropic`
  - Auth: `ANTHROPIC_API_KEY` env var or `[providers.anthropic] api_key` in `~/.codiv/config.toml`
  - Base URL: configurable via `[providers.anthropic] base_url` (for proxy/custom endpoints)
  - Context windows: 200,000 tokens for all listed Claude models

- **OpenAI** — GPT models (gpt-4o, gpt-4o-mini)
  - SDK/Client: `aisdk` feature `openai` → `aisdk::providers::OpenAI`
  - Auth: `OPENAI_API_KEY` env var or `[providers.openai] api_key` in config
  - Schema sanitization applied for OpenAI strict mode compatibility (see `sanitize_tool_schema_for_openai` in `crates/codivd/src/agent/config.rs`)
  - Context window: 128,000 tokens

- **Google** — Gemini models (gemini-2.0-flash, gemini-2.5-pro)
  - SDK/Client: `aisdk` feature `google` → `aisdk::providers::Google`
  - Auth: `GOOGLE_API_KEY` env var or `[providers.google] api_key` in config
  - Context window: 1,048,576 tokens

- **vLLM** — Local/self-hosted inference server
  - SDK/Client: `aisdk` feature `vllm` → `aisdk::providers::Vllm`
  - Auth: Optional `api_key`; requires `base_url` in config
  - Uses streaming to avoid blocking single-request concurrency slots

- **OpenAI-Compatible providers** — Any provider with an OpenAI-style API (e.g. Deepseek, Groq, Ollama, LM Studio)
  - SDK/Client: `aisdk` feature `openaicompatible` → `aisdk::providers::OpenAICompatible`
  - Auth: `api_key` and `base_url` both required in `[providers.<name>]` config block
  - Examples in `crates/codivd/config.default.toml`: Deepseek, Groq, Ollama

All LLM communication uses Server-Sent Events (SSE) streaming via `reqwest` 0.12 and `reqwest-eventsource` inside `aisdk`. Retry logic with exponential backoff (up to 3 retries, 1s initial backoff) is implemented in `crates/codivd/src/agent/config.rs` (`stream_from_config`).

## Data Storage

**Databases:**
- **SQLite** (via `rusqlite` 0.31, bundled feature)
  - Location: `~/.codiv/sessions.db`
  - Client: `rusqlite::Connection` (synchronous, single-threaded)
  - Schema: Two tables — `sessions` (UUID pk, name, parent_id, fork_at_seq, cwd) and `events` (session_id, seq, event_type, payload JSON)
  - WAL journal mode enabled; synchronous=NORMAL
  - Implementation: `crates/codivd/src/store.rs`

**File Storage:**
- Local filesystem only
- Config dir: `~/.codiv/`
- No cloud/object storage

**Caching:**
- None (no Redis, Memcached, or in-process cache layer)
- Anthropic prompt caching tracked via `cached_tokens` in usage metrics

## Authentication & Identity

**Auth Provider:**
- No user-facing authentication system
- API key authentication to LLM providers only
- Keys sourced from environment variables or `~/.codiv/config.toml`
- Key resolution order (per provider):
  1. Per-role `api_key` in `[roles.<name>]`
  2. Per-provider `api_key` in `[providers.<name>]`
  3. Environment variable (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GOOGLE_API_KEY`)
- Key values are redacted in debug output (`ProviderConfig` custom `Debug` impl in `crates/codivd/src/agent/config.rs`)

## Monitoring & Observability

**Error Tracking:**
- No external error tracking service (no Sentry, Datadog, etc.)

**Logs:**
- Daemon: `tracing` + `tracing-subscriber` writing to `~/.codiv/codivd.log`
  - Log level controlled via `RUST_LOG` env var
  - Default level: `codivd=info`
- Client TUI: `log` + `env_logger` writing to `/tmp/codiv-debug.log` (only in `--debug` mode)
- Token usage (input, output, cache_read) logged per LLM call in daemon

## CI/CD & Deployment

**Hosting:**
- GitHub Releases at `github.com/razorback16/codiv`
- Pre-built binaries distributed (not published to crates.io)

**Install:**
- `install.sh` script downloads release binary and installs to `~/.local/bin`
- macOS: LaunchAgent plist (`~/Library/LaunchAgents/ai.codiv.daemon.plist`) for auto-start
- Linux: systemd user unit (`~/.config/systemd/user/codivd.service`) for auto-start

**CI Pipeline:**
- No CI configuration detected in repository root (no `.github/workflows/`, no `.circleci/`)

## IPC Transport

**Client ↔ Daemon:**
- Unix domain socket: `/tmp/codivd-{uid}.sock`
- Protocol: length-prefixed binary frames (4-byte header, `FRAME_HEADER_SIZE = 4`)
- Serialization: `serde` + `bincode`
- Max message size: 16 MiB (`MAX_MESSAGE_SIZE`)
- Message types defined in `crates/codiv-common/src/messages.rs`
- IPC server implementation: `crates/codivd/src/ipc/server.rs`

## Webhooks & Callbacks

**Incoming:**
- None — no HTTP server, no webhook endpoints

**Outgoing:**
- None — all external communication is outbound-only LLM API calls

## Environment Configuration

**Required for operation (at least one LLM provider key):**
- `ANTHROPIC_API_KEY` — Anthropic API key (used by default config)
- `OPENAI_API_KEY` — OpenAI API key (if using OpenAI provider)
- `GOOGLE_API_KEY` — Google Gemini API key (if using Google provider)

**Optional:**
- `RUST_LOG` — Log level filter for daemon (e.g., `codivd=debug`)
- `HOME` — Home directory resolution (falls back to `/tmp` if unset)

**Secrets location:**
- Environment variables or `~/.codiv/config.toml` (not committed to source)
- Config default template embedded in daemon binary at compile time: `crates/codivd/config.default.toml` (contains no secrets)

---

*Integration audit: 2026-03-31*
