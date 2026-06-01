# Plan — Free TUI, no account, hmanlab-prox as a provider

**Status:** proposed (not yet implemented)
**Date:** 2026-05-31

## Goal

Make the hmanlab TUI free to use with **no signup and no sign-in**. A user
downloads the binary, connects a provider (local Ollama, z.ai, OpenCode Go,
Ollama Cloud, OpenRouter, or **hmanlab-prox**), and chats. No `bai_` key, no
account wall, no required backend.

`hmanlab-prox` becomes one more BYOK provider in the picker — the user pastes
the `sk-` key they bought on the prox web app and selects a prox-served model.
prox is where accounts, subscriptions, billing, and quota live; the TUI never
logs in.

## What this changes about the architecture

Today the TUI hard-depends on **hmanlab-stack** for two things, both gated
behind a mandatory `bai_` API key:

1. **Session persistence** — `src/api.rs` (`Client`, `run_writer`) writes every
   turn to stack's `/v1/sessions` + `/messages`.
2. **The "hmanlab (free)" hosted provider** — `HMANLAB_HOSTED_PROVIDER` in
   `src/config.rs`, routed in `src/app/backend.rs:71` through stack's
   `/v1/chat/completions` proxy using the `bai_` key.

We are dropping the hmanlab-stack dependency from the TUI entirely. That
removes the only reason the TUI needs an account. Sessions and the hosted
proxy both need a decision (see below).

## The boot blocker to remove

`src/main.rs:67-74` forces the account:

```rust
let api_key = match cli.api_key.or(saved.api_key.clone()) {
    Some(k) => k,
    None => {
        let cfg = config::run_setup_wizard(&api_url, ...).await?;
        cfg.api_key.expect("wizard guarantees api_key")  // ← mandatory bai_
    }
};
```

`run_setup_wizard` (`src/config.rs:419`) loops until it gets a valid `bai_`
key validated against `/v1/auth/me`. **This is the account wall.** It must
become optional — the wizard should offer provider setup without ever
demanding a backend key.

## Work items

### 1. Remove the mandatory key at boot — `src/main.rs`
- Drop the `api_key` resolution that calls the wizard (`:67-74`).
- Drop the `api::Client::new` + `check_auth` + `run_writer` spawn (`:128-136`).
- Remove `--api-key` / `--api-url` CLI flags and their `HMANLAB_API_*` envs
  (`:45-51`) — or keep `--api-url` only if we choose local-session export later.
- Boot straight into the TUI; the only first-run prompt is workspace trust
  (already non-blocking) and the new optional provider wizard.

### 2. Rework the first-run wizard — `src/config.rs`
- `run_setup_wizard` becomes **provider selection only**. No "paste your
  hmanlab key" step, no `check_auth`.
- Menu offers: local Ollama URL, z.ai (sub/usage), Ollama Cloud, OpenCode Go,
  OpenRouter, **hmanlab-prox**. All optional; "skip everything" is valid and
  lands the user in the TUI with whatever Ollama is running (or nothing).
- Drop `api_url` / `api_key` from `Config` (`:307-308`) once nothing reads them.

### 3. Add hmanlab-prox as a BYOK provider — `src/config.rs`
Mirror the OpenCode/OpenRouter pattern:
```rust
pub const PROX_PROVIDER: &str = "hmanlab-prox";
pub const PROX_BASE: &str = "https://ai.hmanlab.pro/v1";   // OpenAI-compat relay
pub const PROX_MODELS: &[&str] = &[ /* seed list, see note */ ];
pub const PROX_DEFAULT_MODEL: &str = "...";
```
- Add `PROX_PROVIDER` to `BYOK_PROVIDERS` (`:166`).
- Add a `provider_label` arm (`:198`) → `"hmanlab (prox)"`.
- Add a `prox_api_key: Option<String>` field to `Config` + fan it out in
  `persist_config` (`src/app/backend.rs:352`).

> **Model list:** prox exposes `GET /v1/models` (`router.go:135`). Prefer a
> live refresh (like `refresh_openrouter_models` in `backend.rs:274`) over a
> hardcoded seed, since the admin configures channels/models server-side and a
> static list will drift. A small seed is fine as the offline fallback.

### 4. Route the prox provider — `src/app/backend.rs`
- Add a `PROX_PROVIDER` arm in `make_backend_for` (`:63`) that builds an
  `openai_compat::Client` with `PROX_BASE` + the user's `sk-` key. This is the
  same shape as the OpenCode arm (`:98`) — prox speaks OpenAI-compatible
  `/v1/chat/completions`, which `src/openai_compat.rs` already drives.
- Add `current_host` arm (`:30`) for the header display.
- Add `ensure_prox_models` following `ensure_opencode_models` (`:227`).

### 5. Remove the stack-dependent hosted provider — `src/config.rs` + `backend.rs`
- Delete `HMANLAB_HOSTED_PROVIDER` / `HMANLAB_HOSTED_MODELS` (`config.rs:185`).
- Delete the hosted-chat arm in `make_backend_for` (`backend.rs:71`) and
  `ensure_hosted_models` (`backend.rs:176`) — it routed through stack's `bai_`
  proxy, which no longer exists for the TUI.
- Keep a migration arm so old configs with `provider:"hmanlab-free"` get
  dropped from `extra_models` instead of leaving a dead picker row (extend
  `migrate_legacy_zai_provider`, `backend.rs:327`).

### 6. Decide what happens to sessions — **needs your call**
`/sessions`, `/load`, `/more`, and cross-device history all came from stack.
With stack gone there are three options:

- **(a) Drop server sessions.** History lives only in the running process;
  remove `src/api.rs`, the `ApiOp` writer, and the session slash-commands.
  Simplest; the TUI becomes a pure client. Matches "settle the TUI first."
- **(b) Local session storage.** Persist turns to
  `~/.config/hmanlab/sessions/` (SQLite or JSONL) so `/sessions` + `/load`
  keep working offline, no account. More work, keeps the feature.
- **(c) Defer.** Leave the session code dormant behind an unset URL so nothing
  calls it, revisit when training data is on the table.

Recommendation: **(a)** now, revisit **(b)** when we wire training data — the
training capture will likely live in prox's relay path, not the TUI.

### 7. Strip the account block from `/settings` — `src/app/commands/settings.rs`
- Remove the `fetch_me` / `fetch_hosted_usage` lookups (`:86-90`) and the
  "Account" + "api call x/200" block (`:91-123`). Those read from stack.
- `/settings` keeps showing: version, model, Ollama host, configured BYOK
  providers (presence only), workspace. `/whoami` no longer has an account to
  report — repoint it at "configured providers" or retire the alias.

### 8. Docs + onboarding copy — `README.md`
- Rewrite "First run" (`README.md:95`) — no `bai_` key, no
  `hmanlab.senireka.my` registration. New flow: download → optionally connect
  a provider → chat.
- Update the architecture diagram (`README.md:259`) — drop the
  `HTTPS → hmanlab-api` leg; add hmanlab-prox to the provider table (`:246`).
- Note that buying hosted model access happens on the prox web app, and the
  `sk-` key is pasted into the TUI like any other provider key.

## Out of scope (settle later)

- **Training-data capture.** Where opted-in conversations get stored for
  fine-tuning. Most likely prox's relay path, not the TUI. Tracked separately.
- **Local session persistence** — option (6b), if we want history back.
- **Retiring hmanlab-stack down to just the trainer** — a prox/stack concern,
  not a TUI change.

## Acceptance check

1. Fresh machine, no config: run `hmanlab` → lands in the TUI, no key prompt.
2. `/model` → add hmanlab-prox key → pick a prox model → streaming reply works.
3. Local Ollama still works with zero providers configured.
4. `/settings` shows config only, never errors trying to reach a backend.
5. No code path references `be-ai.senireka.my` or a `bai_` key.
