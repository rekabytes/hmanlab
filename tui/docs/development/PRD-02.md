# PRD-02 — hmanlab TUI: Session Persistence + Multi-Provider BYOK

| | |
|---|---|
| **Status** | Draft |
| **Target** | `tui/` phase 2 — sessions + multi-provider |
| **Depends on** | [PRD v0](./PRD.md) (shipped — walking skeleton with Ollama Cloud chat) |
| **Reference impl** | [`cli/`](../../cli/) (Rust + ratatui, production) |

---

## 1. Vision

PRD v0 proved the architecture: type a message, stream a response from
Ollama Cloud, render it as markdown. It's a working chat client — but
only for one provider, and conversations vanish on exit.

Phase 2 makes the TUI a **persistent, multi-provider chat client**:

- **Session persistence.** Conversations are saved as JSONL files on
  disk, one per session. Users can quit, come back days later, and pick
  up where they left off. `/sessions` lists past conversations;
  `/load <prefix>` restores one.
- **Multi-provider BYOK.** The `Provider` interface (already in v0)
  gets real implementations: z.ai, OpenRouter, OpenCode Go, hmanlab,
  MiniMax, and local Ollama — all behind one config file. The connect
  modal expands from "Ollama Cloud only" to a provider picker.

No agent loop, no tools, no tool calling. Those land in PRD-03. This
phase is about making the chat client solid enough that adding tools
on top is a clean layering, not a rescue mission.

[`cli/`](../../cli/) remains the reference implementation. The JSONL
session format and provider constants are mirrored byte-for-byte so
sessions and config files are cross-compatible between the two clients.

---

## 2. Goals for phase 2

**Two features, each complete end-to-end.**

### Session persistence
- Every conversation is auto-saved to
  `~/.config/hmanlab/sessions/<cwd-hash>/<uuid>.jsonl`.
- `/sessions` lists past sessions for the current working directory
  (title, model, timestamp).
- `/load <prefix>` restores a session into the active chat — full
  history replays, ready to continue.
- Sessions launched from the same cwd are grouped; different cwds are
  isolated (matches CLI behavior).
- JSONL format is identical to the CLI's (`cli/src/session.rs`) —
  cross-compatible, same files readable by both clients.

### Multi-provider BYOK
- 7 BYOK providers + local Ollama, each with its own API key and
  model catalog:
  - z.ai (subscription), z.ai (usage-based)
  - Ollama Cloud
  - OpenCode Go
  - OpenRouter
  - hmanlab (prox relay)
  - MiniMax
- Two wire protocols:
  - **Ollama native** (`/api/chat`, newline-delimited JSON) — v0 already
    has this for Ollama Cloud; local Ollama uses the same protocol
    without Bearer auth.
  - **OpenAI-compatible** (`/v1/chat/completions`, SSE `data:` lines,
    `[DONE]` sentinel) — new client, covers all other providers.
- `/model` picker shows models from all configured providers, grouped
  by provider. Models from unconfigured providers show as
  `+ Add <provider> key` rows.
- `/provider` command to switch active provider without diving through
  the model picker.
- First-run connect modal expands from single-provider to a provider
  picker — user picks one (or more), pastes keys, drops into chat.
- `last_model` + `last_provider` persisted so relaunch boots with the
  same selection.

---

## 3. Non-goals for phase 2

Explicitly **deferred to PRD-03+**:

- Agent loop (model calls tools, sees results, continues)
- Tool execution pipeline (read / write / edit / git / shell)
- Multi-tool grouping
- Specialist agents (`/ask`, `consult_specialist`)
- MCP web-search providers
- Sidebar / inspector multi-pane layout
- Compaction
- Memory store
- Telegram bridge
- Image attachments / clipboard paste
- OpenRouter live model catalog refresh (use static seed for now;
  live `/v1/models` fetch deferred)
- Workspace trust gate (no destructive tools yet, so no gate needed)

---

## 4. Reference contract

Mirror the CLI's artifacts exactly. The JSONL session files and the
config must be cross-compatible — a session saved by the Go TUI loads
in the Rust CLI, and vice versa.

| Concern | Source of truth | Notes |
|---|---|---|
| Session JSONL format | [`cli/src/session.rs`](../../cli/src/session.rs) | Record types (`meta`/`user`/`assistant`/`tool`), field names, UUID shape, ISO-8601 timestamps. Must be byte-compatible. |
| Session directory layout | `cli/src/session.rs:68-87` | `~/.config/hmanlab/sessions/<cwd-hash>/<uuid>.jsonl`. CWD hash via `DefaultHasher` (Rust) — Go must produce the same hash. |
| Provider constants | [`cli/src/config.rs`](../../cli/src/config.rs) | All `*_PROVIDER`, `*_BASE`, `*_MODELS`, `*_DEFAULT_MODEL` constants. Keep in lockstep. |
| BYOK key field names | `cli/src/config.rs:321-420` | `zai_api_key`, `zai_usage_api_key`, `ollama_cloud_api_key`, `opencode_api_key`, `openrouter_api_key`, `hmanlab_api_key`, `minimax_api_key`, `ollama_host`. |
| OpenAI-compat wire format | [`cli/src/openai_compat.rs`](../../cli/src/openai_compat.rs) | SSE `data:` lines, `[DONE]` sentinel, `stream_options.include_usage`, retry on 429/502/503/504. |
| Provider label display | `cli/src/config.rs:208-219` | `provider_label()` — human-readable names for picker / status bar. |
| `extra_models` schema | `cli/src/config.rs:223-229` | `[{provider, name}]` — user-added models per provider. |

### CWD hash compatibility note

The CLI uses Rust's `std::collections::hash_map::DefaultHasher` (SipHash
1-3 with a fixed seed of 0) to hash the CWD path into a 16-hex-char
directory name. Go's `hash/fnv` or `crypto/sha256` will produce a
**different** hash. Options:

1. **Port SipHash** — implement SipHash 1-3 in Go (small, ~100 lines).
   Guarantees the same hash → same session directory. Sessions saved by
   CLI in a given cwd appear in the TUI and vice versa.
2. **Use a different hash and accept isolation** — Go TUI sessions
   live in a different subdirectory than CLI sessions. Cross-client
   session sharing requires manual lookup.

**Recommendation:** Option 1 (port SipHash). The whole point of
cross-compatibility is that users don't think about which client they
launched last. ~100 lines of Go for a SipHash 1-3 implementation is
cheap insurance. See `cli/src/session.rs:76-82` for the exact call.

---

## 5. Architecture overview

### New file layout (additions to v0)

```
tui/
├── internal/
│   ├── config/
│   │   ├── config.go              # expanded: all BYOK key fields, provider constants
│   │   ├── config_test.go
│   │   └── providers.go           # NEW: provider catalog (constants, labels, model lists)
│   ├── llm/
│   │   ├── provider.go            # unchanged interface
│   │   ├── ollama.go              # expanded: supports local Ollama (no key) + Ollama Cloud (key)
│   │   ├── openai_compat.go       # NEW: OpenAI-compatible streaming client (SSE)
│   │   ├── openai_compat_test.go  # NEW: SSE parse tests
│   │   └── backend.go             # NEW: provider routing (provider ID → concrete client)
│   ├── session/
│   │   ├── session.go             # NEW: JSONL read/write/list (mirrors cli/src/session.rs)
│   │   ├── session_test.go        # NEW: round-trip, list, prefix-find tests
│   │   └── siphash.go             # NEW: SipHash 1-3 port (CWD hash compat with CLI)
│   └── ui/
│       ├── model.go               # expanded: session state, provider switching
│       ├── chat.go                # unchanged
│       ├── commands.go            # expanded: /sessions, /load, /provider, /disconnect
│       ├── connect.go             # expanded: multi-provider picker modal
│       ├── session_picker.go      # NEW: /sessions list overlay
│       └── model_picker.go        # NEW: /model picker with provider grouping
├── docs/development/
│   ├── PRD.md                     # v0 (reference)
│   └── PRD-02.md                  # this document
├── go.mod
└── README.md
```

### Session persistence design

**Write path:**
- On first user message in a fresh session: call `session.WriteMeta()`
  to create the file and write the `meta` record (title derived from
  first message, current model).
- On every user message: `session.WriteUser()` appends a `user` record.
- On every completed assistant response: `session.WriteAssistant()`
  appends an `assistant` record.
- Writes are append-only, one JSON line per record. No locks needed
  (single-writer — the TUI process owns the file).

**Read path:**
- `/load <prefix>`: `session.FindByPrefix()` resolves the prefix to a
  file path, `session.LoadRecords()` reads all lines, then
  `session.RecordsToMessages()` converts to `[]llm.Message` for the
  chat history.
- `/sessions`: `session.ListSessions()` scans the cwd-hash directory,
  reads the first 20 lines of each file for title + model, sorts by
  mtime descending.

**Active session tracking:**
- `model.activeSession` holds the current session ID + file path.
- On launch with no session loaded: `activeSession = nil`. First user
  message triggers session creation.
- `/clear` closes the current session and starts fresh (new UUID on
  next message).

### Multi-provider design

**Backend routing** (`internal/llm/backend.go`):

```go
// BackendFor returns the streaming client for a given provider ID.
// Returns nil if the provider isn't configured (no key).
func BackendFor(cfg *config.Config, provider string) llm.Provider
```

Mirrors `cli/src/app/backend.rs:58-94`. The routing table:

| Provider ID | Client | Base URL | Auth |
|---|---|---|---|
| `ollama-cloud` | `ollama.Client` | `https://ollama.com` | Bearer key |
| `zai-subscription` | `openai_compat.Client` | `https://api.z.ai/api/coding/paas/v4` | Bearer key |
| `zai-usage` | `openai_compat.Client` | `https://api.z.ai/api/paas/v4` | Bearer key |
| `opencode` | `openai_compat.Client` | `https://opencode.ai/zen/go/v1` | Bearer key |
| `openrouter` | `openai_compat.Client` | `https://openrouter.ai/api/v1` | Bearer key |
| `hmanlab` | `openai_compat.Client` | `https://ai.hmanlab.pro/v1` | Bearer key |
| `minimax` | `openai_compat.Client` | `https://api.minimax.io/v1` | Bearer key |
| local Ollama | `ollama.Client` | from `ollama_host` config | none |

**OpenAI-compat client** (`internal/llm/openai_compat.go`):
- POST `{base}/chat/completions` with `{model, messages, stream: true}`.
- `Authorization: Bearer {key}` header.
- Response is SSE: lines starting with `data: `, terminated by
  `data: [DONE]`.
- Parse each `data:` payload as `{choices: [{delta: {content}}], usage: {...}}`.
- Emit `StreamEvent{Text: chunk}` per content delta,
  `StreamEvent{Done: true, PromptTokens: ..., CompletionTokens: ...}`
  on `[DONE]`.
- Retry on 429/502/503/504 with exponential backoff (800ms, 1.6s, 3.2s),
  max 4 attempts. Match `cli/src/openai_compat.rs:109-148`.

**Config expansion** (`internal/config/config.go`):
- Add typed fields for all BYOK keys + `ollama_host`.
- Preserve the `Raw` map round-trip pattern from v0 (don't drop
  unknown keys on save).
- Add `ExtraModels []ExtraModel` for the model catalog (seeded from
  provider constants on first key entry).

---

## 6. Feature spec

Concrete checklist — phase 2 ships when every box is checked.

### Session persistence

- [ ] `session.WriteMeta(path, sessionID, title, model)` — creates
      file, writes first `meta` line.
- [ ] `session.WriteUser(path, sessionID, content, model)` — appends
      `user` record.
- [ ] `session.WriteAssistant(path, sessionID, content, model)` —
      appends `assistant` record.
- [ ] `session.LoadRecords(path)` — reads all lines, returns
      `[]Record`.
- [ ] `session.RecordsToMessages(records)` — converts to
      `[]llm.Message` for chat history replay.
- [ ] `session.ListSessions(cwd)` — scans cwd-hash dir, returns
      `[]SessionSummary` sorted by mtime descending.
- [ ] `session.FindByPrefix(summaries, prefix)` — resolves ≥4 hex char
      prefix to a unique session. Error on 0 or 2+ matches.
- [ ] CWD hash uses SipHash 1-3 (port from Rust) — same hash as CLI
      for the same path.
- [ ] Active session auto-created on first user message (meta record
      with title from first line of message, truncated ≤60 chars).
- [ ] Every user message + completed assistant response appended to
      the active session file.
- [ ] `/clear` closes active session (next message starts a new one).
- [ ] Ctrl+C mid-stream: partial assistant response saved with `[...]`
      trailing marker (matching v0 cancel behavior).

### Session UI

- [ ] `/sessions` — overlay list of sessions for current cwd, showing
      title + relative time + model. Arrow keys to navigate, Enter to
      load, Esc to cancel.
- [ ] `/load <prefix>` — load session by UUID prefix (≥4 hex chars).
      History replays into chat view. Active session ID updates.
- [ ] `/load` with no arg — same as `/sessions` (show picker).
- [ ] Loaded session continues in the chat — next user message
      appends to the same JSONL file.
- [ ] Status bar shows active session ID (short prefix, first 8
      chars) when a session is loaded.

### Multi-provider config

- [ ] Config struct expanded with all BYOK key fields:
      `zai_api_key`, `zai_usage_api_key`, `ollama_cloud_api_key`,
      `opencode_api_key`, `openrouter_api_key`, `hmanlab_api_key`,
      `minimax_api_key`, `ollama_host`.
- [ ] `config.ConfiguredProviders()` — returns provider IDs that have
      a non-empty key.
- [ ] `config.ByokKey(provider)` — returns the key for a provider ID.
- [ ] Provider constants in `providers.go` — all `*_PROVIDER`,
      `*_BASE`, `*_MODELS`, `*_DEFAULT_MODEL` values matching CLI
      exactly.
- [ ] `config.ProviderLabel(provider)` — human-readable display name.
- [ ] Saving config preserves all existing keys (Raw map round-trip).

### OpenAI-compat streaming client

- [ ] `openai_compat.Client{Base, APIKey, HTTP}` — HTTP client with
      120s timeout.
- [ ] `StreamChat(ctx, model, messages) <-chan StreamEvent` —
      satisfies `llm.Provider`.
- [ ] POST `{base}/chat/completions` with `stream: true`,
      `max_tokens: 32000`, `stream_options: {include_usage: true}`.
- [ ] SSE parsing: skip blank lines + non-`data:` lines, parse
      `data: {json}`, detect `data: [DONE]`.
- [ ] Content delta extraction: `choices[0].delta.content`.
- [ ] Usage extraction: `usage.prompt_tokens` / `usage.completion_tokens`
      (may arrive on last data chunk or on `[DONE]`).
- [ ] Error handling: non-2xx → surface response body (first 1000
      chars) as error message.
- [ ] Retry on 429/502/503/504 with exponential backoff (800ms /
      1600ms / 3200ms), max 4 attempts. Client errors (other 4xx) →
      immediate failure, no retry.
- [ ] Context cancellation respected — goroutine returns promptly on
      `ctx.Done()`.

### Local Ollama support

- [ ] `ollama.Client` expanded to work without an API key (local mode).
- [ ] Reads `ollama_host` from config (default
      `http://localhost:11434`).
- [ ] `Models()` calls `GET /api/tags` to list locally installed
      models (live, not static).
- [ ] No `Authorization` header in local mode.

### Backend routing

- [ ] `llm.BackendFor(cfg, provider)` — returns the correct streaming
      client for a provider ID, or nil if not configured.
- [ ] Ollama native: `ollama-cloud` + local Ollama.
- [ ] OpenAI-compat: z.ai (both plans), OpenCode Go, OpenRouter,
      hmanlab, MiniMax.
- [ ] Provider switching at runtime updates `model.provider` +
      re-resolves the backend.

### Model picker

- [ ] `/model` (no arg) — shows picker overlay. Models grouped by
      provider. Configured providers first (models listed), then
      unconfigured providers as `+ Add <provider> key` rows.
- [ ] `/model <name>` — switch to a model. Validates against
      configured providers' model lists. If ambiguous (same model
      name on multiple providers), requires `/model <provider>/<name>`.
- [ ] Selecting an `+ Add key` row opens a key input prompt inline.
- [ ] Model selection persists `last_model` + `last_provider` to
      config on next launch.

### Provider commands

- [ ] `/provider` — show current provider.
- [ ] `/provider <name>` — switch active provider (must be configured).
- [ ] `/disconnect <provider>` — remove a provider's key from config.
      Confirms before wiping.
- [ ] `/settings` — show current config summary (providers, model,
      session).

### First-run modal expansion

- [ ] Provider picker: list of all 7 BYOK providers + local Ollama.
- [ ] Selecting a BYOK provider shows key input + "Get a key from
      <url>" hint.
- [ ] Selecting local Ollama shows host input (default
      `http://localhost:11434`).
- [ ] User can configure multiple providers before entering chat.
- [ ] On submit: writes all keys to config.toml, drops into chat with
      the first configured provider's default model.

### Error handling

- [ ] Network errors surface inline in chat as a styled error line.
- [ ] Auth errors (401/403) show inline: "API key for `<provider>`
      seems invalid. Use `/disconnect <provider>` or update the key."
- [ ] Provider-not-configured: "No key configured for `<provider>`.
      Use the connect modal or `/model` picker to add one."
- [ ] Session file corruption: skip malformed lines (matching CLI's
      `load_records` behavior), log a warning.
- [ ] App never crashes — every error path is recoverable.

---

## 7. Session format (detailed)

Each session is one JSONL file. Every line is a JSON object with this
shape (must match `cli/src/session.rs:23-47`):

```json
{
  "type": "user",
  "session_id": "a1b2c3d4-5678-4ef0-a123-b4567890abcd",
  "uuid": "e5f6a7b8-9012-4abc-d345-e67890f1a2b3",
  "timestamp": "2026-07-08T14:30:00Z",
  "content": "Hello, how are you?",
  "model": "glm-4.7"
}
```

### Record types

| `type` | When written | Key fields |
|---|---|---|
| `meta` | First line, on session creation | `title`, `model` |
| `user` | Each user message | `content`, `model` |
| `assistant` | Each completed assistant response | `content`, `model` |
| `tool` | *(deferred — no tools in phase 2)* | `name`, `content` |

### Fields

| Field | Type | Notes |
|---|---|---|
| `type` | string | `"meta"` / `"user"` / `"assistant"` / `"tool"` |
| `session_id` | string | UUID v4 shape. Constant for all records in one file. |
| `uuid` | string | UUID v4 shape. Unique per record. |
| `timestamp` | string | ISO-8601 UTC (`YYYY-MM-DDTHH:MM:SSZ`). |
| `content` | string | Message text. Empty for `meta`. |
| `name` | string? | Tool name. Only set for `tool` records. |
| `model` | string? | Model in use. Set on `meta`, `user`, `assistant`. |
| `title` | string? | Session title. Only set on `meta`. |

### UUID generation

The CLI generates pseudo-UUIDs from time + pid + counter
(`cli/src/session.rs:98-119`), not cryptographically random. The shape
is `xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx` (UUID v4 format). The Go TUI
should use `crypto/rand` or `google/uuid` for real UUIDs — the format
is the same, and real randomness is strictly better. Cross-compatibility
isn't affected (the CLI reads any UUID-shaped string).

---

## 8. Provider catalog

Mirrors `cli/src/config.rs` constants. All values must match exactly.

| Provider ID | Label | Base URL | Default Model | Key Field |
|---|---|---|---|---|
| `zai-subscription` | z.ai (subscription) | `https://api.z.ai/api/coding/paas/v4` | `glm-4.7` | `zai_api_key` |
| `zai-usage` | z.ai (usage-based) | `https://api.z.ai/api/paas/v4` | `glm-4.7` | `zai_usage_api_key` |
| `ollama-cloud` | Ollama Cloud | `https://ollama.com` | `glm-4.7` | `ollama_cloud_api_key` |
| `opencode` | OpenCode Go | `https://opencode.ai/zen/go/v1` | `glm-5.1` | `opencode_api_key` |
| `openrouter` | OpenRouter | `https://openrouter.ai/api/v1` | `anthropic/claude-sonnet-4.6` | `openrouter_api_key` |
| `hmanlab` | hmanlab | `https://ai.hmanlab.pro/v1` | `claude-sonnet-4-6` | `hmanlab_api_key` |
| `minimax` | MiniMax | `https://api.minimax.io/v1` | `MiniMax-M3` | `minimax_api_key` |
| *(local)* | local Ollama | from `ollama_host` | *(live `/api/tags`)* | *(none)* |

Model catalogs are the static seed lists from `cli/src/config.rs`
(`ZAI_MODELS`, `OLLAMA_CLOUD_MODELS`, `OPENCODE_MODELS`,
`OPENROUTER_MODELS`, `HMANLAB_MODELS`, `MINIMAX_MODELS`). These are
starting points — the user can add models manually by editing config.

Live catalog refresh (OpenRouter `/v1/models`, local Ollama `/api/tags`)
is deferred except for local Ollama, which is cheap and always live.

---

## 9. Risks & unknowns

| Risk | Severity | Mitigation |
|---|---|---|
| CWD hash mismatch between Go and Rust → sessions not shared across clients | **High** | Port SipHash 1-3. Test against known Rust outputs. If skipped, document the isolation clearly. |
| OpenAI-compat SSE parsing quirks — each provider has slight variations in chunk shape, usage placement, error format | **High** | Port the CLI's test fixtures. Test with real keys against z.ai + OpenRouter early. |
| `bubbles/list` component may not exist or may be insufficient for the session/model picker overlays | Medium | Evaluate early (M1). Fallback: hand-roll a simple list renderer with arrow-key navigation. |
| Config round-trip: TOML encoder may reorder keys or change formatting, causing diff noise for CLI users | Medium | Test round-trip: load → save → compare. Accept key reordering if values are preserved. |
| Retry logic in OpenAI-compat client — getting backoff timing wrong causes thundering herd or stale UI | Medium | Match CLI's constants exactly (800ms / 1600ms / 3200ms, max 4 attempts). Surface retry state in status bar. |
| Session write on every message could block UI on slow filesystems | Low | Writes are append-only, one line — sub-millisecond. If profiling shows issues, move to a buffered channel + background writer. |
| Provider key validation (`/api/tags` for Ollama, `/v1/models` for OpenAI-compat) adds latency to first-run modal | Low | Validate only the selected provider(s), not all. Show a spinner during validation. |

---

## 10. Success criteria

Phase 2 ships when **all** of these are true:

### Sessions
- [ ] Send 5 messages, quit, relaunch — `/sessions` shows the session,
      `/load <prefix>` restores it, and continuing the conversation
      appends to the same file.
- [ ] Session file opens in the Rust CLI and loads correctly
      (cross-compatible).
- [ ] `/clear` starts a fresh session; old session remains on disk.
- [ ] `/sessions` picker navigates with arrow keys, loads on Enter,
      cancels on Esc.

### Multi-provider
- [ ] Configure two providers (e.g. Ollama Cloud + z.ai). Switch
      between them with `/provider`. Chat works on both.
- [ ] `/model` picker shows models from all configured providers,
      grouped by provider.
- [ ] OpenRouter with a real key streams a response from
      `anthropic/claude-sonnet-4.6`.
- [ ] Local Ollama (no key) streams from a locally installed model.
- [ ] Relaunching boots with the last-used provider + model.
- [ ] Auth error (bad key) surfaces inline with an actionable message.

### Quality
- [ ] `go test ./...` passes with tests for:
      - Session JSONL round-trip (write → read → compare)
      - Session list + prefix find
      - CWD hash (SipHash) matches known Rust output
      - OpenAI-compat SSE parsing (mocked SSE stream)
      - Config round-trip (load → save → load → compare)
      - Provider routing (provider ID → correct backend)
- [ ] `go vet` clean.
- [ ] No goroutine leaks (session writes complete, streams cancel
      cleanly).

---

## 11. Milestones

Suggested build order. Each milestone is independently testable.

- [ ] **M1 — Provider catalog + config expansion** (~2 days)
      `providers.go` with all constants. Config struct expanded with
      all BYOK key fields. Config round-trip tests passing. No UI
      changes yet.

- [ ] **M2 — OpenAI-compat streaming client** (~3 days)
      `openai_compat.go` with SSE parsing, retry logic, usage
      extraction. Tested with mocked SSE. Then validate with a real
      z.ai key end-to-end (chat streams through the existing UI).

- [ ] **M3 — Backend routing + provider switching** (~2 days)
      `backend.go` routing table. Wire into the model so
      `model.provider` selects the backend. `/provider` command works.
      Local Ollama support (no-key mode). Status bar shows current
      provider.

- [ ] **M4 — Model picker + connect modal expansion** (~3 days)
      `/model` picker overlay with provider grouping. Connect modal
      expands to provider picker. `+ Add key` flow inline. `last_model`
      + `last_provider` persistence.

- [ ] **M5 — Session JSONL core** (~3 days)
      `session.go` with WriteMeta / WriteUser / WriteAssistant /
      LoadRecords / RecordsToMessages. SipHash port for CWD hashing.
      Session auto-created on first message. Auto-append on every
      message + response. No UI yet — test via `/sessions` debug
      output.

- [ ] **M6 — Session UI** (~2 days)
      `/sessions` picker overlay (list, navigate, load, cancel).
      `/load <prefix>` command. Active session ID in status bar.
      Loaded session continues writing to the same file.

- [ ] **M7 — Polish + cross-compat validation** (~2 days)
      Test sessions saved by Go load in CLI (and vice versa). Error
      states (corrupt JSONL, missing session dir, bad provider key
      mid-session). `go vet` clean. README updated.

**Total estimate: ~17 working days** for one developer.

---

## 12. Open questions

- **Model picker component.** Does `bubbles/list` work for our needs
  (grouped, scrollable, custom rendering per row), or should we
  hand-roll? Decision in M4.
- **Session picker scope.** Should `/sessions` show sessions from all
  cwds, or only the current cwd? CLI shows current-cwd only. Match
  CLI or expand?
- **Config format: TOML or JSON?** v0 uses TOML (`config.toml`); CLI
  uses JSON (`config.json`). The v0 PRD said "mirrors cli config.rs
  schema" but chose TOML. For phase 2 we need to decide: stay TOML
  (Go TUI only), or switch to JSON for cross-compat with CLI? If we
  want config cross-compat, JSON is required. If config is per-client,
  TOML is fine.
- **OpenRouter live catalog.** The static seed (`OPENROUTER_MODELS`)
  is ~13 models. The live `/v1/models` endpoint returns 400+. CLI
  fetches live + filters by vendor whitelist. Defer to phase 2 or
  ship with static seed only?
- **Provider key validation.** Should the connect modal validate keys
  before entering chat (extra latency, better UX), or trust-and-let-
  errors-surface (faster, rougher)? CLI validates via `/api/tags`
  for Ollama. OpenAI-compat has no cheap validation endpoint — a
  `/v1/models` GET works but isn't universal.
