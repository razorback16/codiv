# Requirements: Codiv Auth

**Defined:** 2026-03-31
**Core Value:** Users can authenticate with any supported AI provider through a single `codiv login` command

## v1 Requirements

### CLI & Wizard

- [ ] **CLI-01**: User can run `codiv login` to start interactive provider wizard
- [ ] **CLI-02**: User can run `codiv login <provider>` to skip provider selection
- [ ] **CLI-03**: Wizard shows numbered provider list with arrow key selection (dialoguer)
- [ ] **CLI-04**: Wizard shows available auth methods for selected provider

### Authentication

- [ ] **AUTH-01**: User can authenticate via API key (paste, validate, store)
- [ ] **AUTH-02**: User can authenticate via OAuth code flow (browser redirect, code exchange)
- [x] **AUTH-03**: OAuth tokens include refresh tokens with expiration tracking

### Providers

- [ ] **PROV-01**: Anthropic provider with API key auth
- [ ] **PROV-02**: OpenAI provider with API key auth
- [ ] **PROV-03**: Claude Code provider with OAuth code flow
- [ ] **PROV-04**: Codex provider with OAuth code flow
- [x] **PROV-05**: Provider registry with auth method metadata (ported from forgecode)

### Storage & Refresh

- [x] **STOR-01**: Credentials stored in `~/.codiv/config.toml` (merged with existing config)
- [ ] **STOR-02**: codivd daemon refreshes OAuth tokens automatically before expiry
- [ ] **STOR-03**: Migration utility to move env var credentials into config.toml

## v2 Requirements

### Additional Providers

- **PROV-06**: GitHub Copilot with OAuth device flow
- **PROV-07**: Google Vertex AI with ADC
- **PROV-08**: Azure OpenAI with API key
- **PROV-09**: Full forgecode provider catalog

### Enhanced UX

- **UX-01**: TUI-inline login flow (login within the TUI interface)
- **UX-02**: `codiv logout <provider>` command
- **UX-03**: `codiv providers` to list configured providers and status

## Out of Scope

| Feature | Reason |
|---------|--------|
| OAuth device flow | Not needed for v1 providers (Anthropic, OpenAI, Claude Code, Codex) |
| Google ADC | Defer until Vertex AI demand exists |
| TUI-inline login | CLI subcommand sufficient for v1 |
| fzf-based selection | Using dialoguer for interactive prompts instead |
| Shared auth crate with forgecode | Port and adapt approach chosen over shared dependency |
| Credential encryption | Existing config.toml is plaintext — match current pattern |

## Traceability

| Requirement | Phase | Status |
|-------------|-------|--------|
| CLI-01 | Phase 3 | Pending |
| CLI-02 | Phase 3 | Pending |
| CLI-03 | Phase 3 | Pending |
| CLI-04 | Phase 3 | Pending |
| AUTH-01 | Phase 2 | Pending |
| AUTH-02 | Phase 2 | Pending |
| AUTH-03 | Phase 1 | Complete |
| PROV-01 | Phase 2 | Pending |
| PROV-02 | Phase 2 | Pending |
| PROV-03 | Phase 2 | Pending |
| PROV-04 | Phase 2 | Pending |
| PROV-05 | Phase 1 | Complete |
| STOR-01 | Phase 2 | Complete |
| STOR-02 | Phase 3 | Pending |
| STOR-03 | Phase 3 | Pending |

**Coverage:**
- v1 requirements: 15 total
- Mapped to phases: 15
- Unmapped: 0

---
*Requirements defined: 2026-03-31*
*Last updated: 2026-03-31 after roadmap creation*
