# Roadmap: Codiv Auth

## Overview

Three phases take the project from a ported auth domain layer to a working `codiv login` command. Phase 1 establishes the provider registry and auth type foundation ported from forgecode. Phase 2 implements API key and OAuth code flows with credential storage in config.toml. Phase 3 wires everything into the interactive CLI wizard, adds env var migration, and adds daemon-side token refresh.

## Phases

**Phase Numbering:**
- Integer phases (1, 2, 3): Planned milestone work
- Decimal phases (2.1, 2.2): Urgent insertions (marked with INSERTED)

Decimal phases appear between their surrounding integers in numeric order.

- [x] **Phase 1: Auth Foundation** - Provider registry, auth domain types, and OAuth token model ported from forgecode (completed 2026-03-31)
- [ ] **Phase 2: Auth Flows & Storage** - API key and OAuth code flow implementations with config.toml credential storage
- [ ] **Phase 3: CLI Wizard & Daemon Refresh** - Interactive `codiv login` wizard, env var migration, and daemon-side token refresh

## Phase Details

### Phase 1: Auth Foundation
**Goal**: Auth domain types and provider registry are in place so auth flows can be built against them
**Depends on**: Nothing (first phase)
**Requirements**: PROV-05, AUTH-03
**Success Criteria** (what must be TRUE):
  1. A provider registry exists with metadata for Anthropic, OpenAI, Claude Code, and Codex providers
  2. Each provider entry declares its supported auth methods (API key or OAuth code flow)
  3. An OAuth token struct exists with access token, refresh token, and expiry fields
**Plans**: 2 plans

Plans:
- [x] 01-01-PLAN.md — Auth domain types (OAuthTokens, OAuthConfig, AuthMethod, newtypes) in codiv-common
- [x] 01-02-PLAN.md — Provider registry (Anthropic, OpenAI, Claude Code, Codex) in codiv-common

### Phase 2: Auth Flows & Storage
**Goal**: Users can authenticate with any supported provider and have credentials persisted to config.toml
**Depends on**: Phase 1
**Requirements**: AUTH-01, AUTH-02, PROV-01, PROV-02, PROV-03, PROV-04, STOR-01
**Success Criteria** (what must be TRUE):
  1. User can paste an API key for Anthropic or OpenAI and it is validated and saved to `~/.codiv/config.toml`
  2. User can complete an OAuth code flow for Claude Code or Codex (browser redirect, code exchange) and receive a stored token
  3. Credentials written to config.toml are merged with existing config without overwriting unrelated settings
  4. codiv reads the stored credentials and uses them for API calls without requiring env vars
**Plans**: TBD

### Phase 3: CLI Wizard & Daemon Refresh
**Goal**: Users can run `codiv login` from the terminal and interact with a polished provider selection wizard; daemon refreshes tokens without user action
**Depends on**: Phase 2
**Requirements**: CLI-01, CLI-02, CLI-03, CLI-04, STOR-02, STOR-03
**Success Criteria** (what must be TRUE):
  1. Running `codiv login` presents a numbered provider list with arrow key navigation
  2. Running `codiv login anthropic` skips provider selection and goes directly to auth method prompt
  3. Wizard shows only the auth methods relevant to the selected provider
  4. codivd automatically refreshes an expiring OAuth token before it causes a failed API call
  5. Running the migration utility moves existing env var credentials into config.toml
**Plans**: TBD
**UI hint**: yes

## Progress

**Execution Order:**
Phases execute in numeric order: 1 → 2 → 3

| Phase | Plans Complete | Status | Completed |
|-------|----------------|--------|-----------|
| 1. Auth Foundation | 2/2 | Complete   | 2026-03-31 |
| 2. Auth Flows & Storage | 0/TBD | Not started | - |
| 3. CLI Wizard & Daemon Refresh | 0/TBD | Not started | - |
