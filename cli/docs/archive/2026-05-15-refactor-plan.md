# hmanlab refactor plan

> **Historical record (archived 2026-05-16).** This plan was executed on
> 2026-05-15; the file layout it describes (`src/app.rs`, `src/tools.rs`,
> `src/ui.rs` as flat files) no longer matches `main` — see `src/app/`,
> `src/tools/`, and `src/ui/` for the post-refactor structure. Kept for
> context on the design decisions that produced today's module layout;
> not a roadmap.

The TUI source has grown to ~4 600 lines across 8 files. Three of those files are doing too much; the other five are already focused and can stay. This plan splits the bloated files into intent-aligned submodules without changing behavior.

Edit this file as you go: tick boxes (`- [x]`) as sub-tasks land. Phase is done when every box is ticked AND `cargo build --release` is clean AND the TUI launches + a smoke chat works end-to-end.

---

## Starting state (2026-05-15)

```
src/
├── api.rs              321 lines    hmanlab-api client                   FINE
├── app.rs             1725 lines    state, events, stream, agent loop   ❌ BLOATED
├── config.rs           143 lines    on-disk config                       FINE
├── main.rs             161 lines    boot, runtime setup                  FINE
├── ollama.rs           273 lines    Ollama client + streaming            FINE
├── openai_compat.rs    340 lines    z.ai / OpenAI-compatible client      FINE
├── tools.rs            856 lines    tool definitions + 8 tool impls     ❌ BLOATED
└── ui.rs               743 lines    all rendering                       ❌ BLOATED
                       ─────
                       4 562 lines
```

The three bloated files are the ones that grew organically as we added features (tool persistence, BYOK, sessions, edit/write tools, diff preview, agent loop). Each is now doing four or five jobs that deserve their own neighbourhood.

---

## Decisions (locked — don't relitigate mid-refactor)

- **Behavior-preserving only.** This is purely a file/folder reorganisation. No bug fixes, no new features, no API changes, no dependency bumps. If you spot a bug during the refactor, **write it down and fix it after**.
- **`cargo build --release` must succeed at the end of every sub-task**, not just every phase. We do one move per commit so bisecting is trivial if something breaks.
- **No new abstractions.** Don't introduce a `Tool` trait, a `Renderer` trait, or an `LlmBackend` trait *unless* the split would be ugly without one. Premature abstractions hide more than they reveal.
- **Modules, not new files for the sake of files.** A 40-line module is fine when its purpose is genuinely separate; a 40-line module that just hosts one utility called from one place isn't earning its directory entry.
- **Keep public surface area the same.** Anything that's `pub` today stays `pub`. Tightening visibility (`pub` → `pub(crate)` → private) is its own clean-up pass; do it AFTER this plan completes.

---

## Target shape

```
src/
├── api.rs              (unchanged)
├── config.rs           (unchanged)
├── main.rs             (unchanged)
├── ollama.rs           (unchanged)
├── openai_compat.rs    (unchanged)
├── agent.rs            NEW: agent_loop extracted from app.rs
├── app/
│   ├── mod.rs              ~300 lines: App struct, enums, new(), top-level dispatch
│   ├── event.rs            ~600 lines: handle_event + the per-Mode key handlers
│   ├── stream.rs           ~350 lines: handle_stream (StreamMsg cases)
│   └── backend.rs          ~150 lines: LlmBackend, make_backend, current_host, extra_provider
├── tools/
│   ├── mod.rs              ~120 lines: ConfirmRequest, DiffLine, ToolContext, execute_tool dispatch
│   ├── definitions.rs      ~250 lines: tool_definitions() + system_prompt() — the "prompt surface" the model sees
│   ├── workspace.rs        ~80 lines:  resolve_in_workspace, truncate_utf8, MAX_*_BYTES constants
│   ├── read.rs             ~180 lines: read_file, list_dir, find_files, IGNORED_DIRS
│   ├── git.rs              ~100 lines: git_diff, git_log, git_show + run_git helpers
│   ├── write.rs            ~180 lines: edit_file, write_file, confirm()
│   ├── diff.rs             ~120 lines: diff_edit, diff_write, truncate_diff
│   └── shell.rs            ~50 lines:  run_command
└── ui/
    ├── mod.rs              ~80 lines:  render entry, Mode dispatch, mask_host, format_tokens
    ├── chat.rs             ~200 lines: render_chat, render_input
    ├── popups.rs           ~280 lines: render_picker, render_session_picker, render_add_model, render_confirm, centered_rect
    └── markdown.rs         ~150 lines: parse_inline_md, wrap_styled_segments
```

Module discovery is via `mod.rs` + `pub mod foo;` declarations. Nothing else changes.

---

## Pre-flight

- [ ] Capture baseline metrics so the refactor's effect is measurable:
  ```bash
  cd ~/projects/hmanlab
  wc -l src/**/*.rs | tail -1            # 4562 total
  cargo build --release                  # 0 warnings expected
  cargo test --release                   # all green (currently 0 tests, that's fine)
  ```
  Save the output of `wc -l` somewhere — you'll compare against it at the end.
- [ ] Commit the current state if uncommitted. Each phase below ends with its own commit so `git bisect` works if a regression slips through.
- [ ] Confirm no in-flight branches/changes that the refactor would conflict with.

---

## Phase 1 — Split `tools.rs`

**Goal:** Each tool category lives in its own file. Adding a new tool requires touching `tools/mod.rs` (dispatch) + one new tool file.

**Estimate:** 90 min.

### Sub-tasks

- [ ] Create `src/tools/` and move `src/tools.rs` → `src/tools/mod.rs`. Build. Should still compile — only one file, just relocated.
- [ ] Extract `tool_definitions()` and `system_prompt()` into `src/tools/definitions.rs`. These are the two giant string-building functions (~250 lines combined) that are the model-facing surface. Re-export from `mod.rs` so call sites in `main.rs` / `app.rs` don't move:
  ```rust
  // tools/mod.rs
  mod definitions;
  pub use definitions::{tool_definitions, system_prompt};
  ```
- [ ] Extract `resolve_in_workspace`, `truncate_utf8`, the `MAX_*_BYTES` constants, and `IGNORED_DIRS` + `is_in_ignored_dir` into `src/tools/workspace.rs`. These are shared by every tool that touches the filesystem.
- [ ] Extract the read-side tools into `src/tools/read.rs`:
  - `tool_read_file`, `tool_list_dir`, `tool_find_files`
- [ ] Extract the git tools + helpers into `src/tools/git.rs`:
  - `tool_git_diff`, `run_git`, `run_git_owned`, `git_output_to_string`
  - The git tools inside `execute_tool`'s match arm (`git_status`, `git_log`, `git_show`) currently inline their calls — leave them in `tools/mod.rs::execute_tool`, just have them call into `git::run_git()`.
- [ ] Extract the diff machinery into `src/tools/diff.rs`:
  - `DiffLineKind` enum, `DiffLine` struct
  - `diff_edit`, `diff_write`, `truncate_diff`, `MAX_DIFF_LINES`
  Re-export `DiffLineKind` and `DiffLine` from `tools/mod.rs` since `ui.rs` imports them today.
- [ ] Extract the write tools into `src/tools/write.rs`:
  - `tool_edit_file`, `tool_write_file`, and the `confirm()` helper
  Notice `confirm()` is used by `write.rs` AND `shell.rs` — it stays in either `tools/mod.rs` or `tools/workspace.rs`. Pick whichever has fewer cross-imports; `mod.rs` is the safer default.
- [ ] Extract `tool_run_command` into `src/tools/shell.rs`.
- [ ] Slim `tools/mod.rs` down to: types (`ConfirmRequest`, `ToolContext`), `pub mod` declarations, `pub use` re-exports, and the `execute_tool` dispatch match.

### Checkpoint

- [ ] `cargo build --release` succeeds with zero warnings.
- [ ] `wc -l src/tools/*.rs` — no single file > 300 lines.
- [ ] Launch the TUI, make it call `read_file`, `list_dir`, `edit_file`, and `run_command` in one session. All four work, including the confirm popup with diff.

---

## Phase 2 — Split `ui.rs`

**Goal:** Each rendered surface (chat, popups, markdown) is its own module. The top-level `mod.rs` is just dispatch + common utilities.

**Estimate:** 45 min.

### Sub-tasks

- [ ] Create `src/ui/` and move `src/ui.rs` → `src/ui/mod.rs`.
- [ ] Extract `parse_inline_md` and `wrap_styled_segments` into `src/ui/markdown.rs`. These are pure functions used in multiple places (chat rendering AND confirm rendering for diff lines).
- [ ] Extract the chat surface into `src/ui/chat.rs`:
  - `render_chat`, `render_input`
- [ ] Extract the popup surfaces into `src/ui/popups.rs`:
  - `render_picker`, `render_session_picker`, `render_add_model`, `render_confirm`, `centered_rect`
- [ ] Leave in `ui/mod.rs`:
  - The top-level render entry function and Mode dispatch
  - `render_header`, `render_status` (header + statusbar are always-on chrome — not popups, not chat)
  - `mask_host`, `format_tokens` (small utilities used by header)

### Checkpoint

- [ ] `cargo build --release` clean.
- [ ] All five Modes render correctly: Chat, ModelPicker, AddModel, SessionPicker, Confirm. (Cycle through them in the TUI.)
- [ ] A long `edit_file` diff still pins `[y] allow / [n] deny` to the bottom of the popup.

---

## Phase 3 — Extract `agent.rs` from `app.rs`

**Goal:** The agent loop is a top-level async task, not "app state". Move it out so `app.rs` is just the state machine.

**Estimate:** 30 min.

### Sub-tasks

- [ ] Create `src/agent.rs`. Move the entire `async fn agent_loop(...)` from `app.rs:1603` into it.
- [ ] In `app.rs` (or wherever spawns it — likely `app.rs::new` or `main.rs`), update the call site: `crate::agent::agent_loop(...)` instead of the inline function.
- [ ] Move any types that are ONLY used inside `agent_loop` along with it. Types shared between `app.rs` and `agent.rs` (e.g. `StreamMsg`, `ApiOp`) stay in `app.rs` for now — they cross the channel between the two and that's their natural home.

### Checkpoint

- [ ] `cargo build --release` clean.
- [ ] A chat round-trip works: send a user message → agent loop calls a tool → tool result lands → final assistant reply renders.

---

## Phase 4 — Split `app.rs`

**Goal:** App state, event handling, stream handling, and backend selection each get their own file.

**Estimate:** 90 min. This is the biggest refactor of the four — do it last.

### Sub-tasks

- [ ] Create `src/app/` and move `src/app.rs` → `src/app/mod.rs`.
- [ ] Extract `LlmBackend`, `make_backend`, `current_host`, `extra_provider`, `ensure_zai_models_pub` into `src/app/backend.rs`. Backend selection is its own concern — Ollama vs z.ai vs OpenAI-compat, picking URLs, mapping model names to providers.
- [ ] Extract `handle_stream` (the `StreamMsg` match block at `app.rs:1387`) into `src/app/stream.rs` as `impl App { ... }`. Each `StreamMsg` arm is independent; the function is ~350 lines and lives well on its own.
- [ ] Extract `handle_event` and its per-Mode children (`handle_chat`, `handle_picker`, `handle_session_picker`, `handle_add_model`, `handle_confirm`) into `src/app/event.rs` as `impl App { ... }`. This is the chunkiest extraction — `handle_chat` alone is dense.
  Also move:
  - `parse_command` + `Command` enum (slash command parser) — only used inside `handle_chat`
  - `fresh_textarea()` — utility for textarea reset
- [ ] What stays in `app/mod.rs`:
  - The `App` struct definition + `new()`
  - The state enums (`Mode`, `AddModelStep`, `PickerEntry`, `AppAction`, `StreamMsg`)
  - Small helpers that are widely used: `push_info`, `persist_assistant_if_any`, etc. (these are scattered through `impl App`; keep them in `mod.rs` for now)
- [ ] After the split, `app/mod.rs` should be ~300–400 lines. `event.rs` will be the chunkiest at ~600 lines — that's a *floor*, not a problem; the per-Mode handlers are intrinsically intertwined.

### Checkpoint

- [ ] `cargo build --release` clean.
- [ ] Full TUI smoke test: launch, switch model, open session picker, send a message that uses tools, edit a file via the AI, deny one tool call, allow another. Everything works as before.

---

## Phase 5 (optional) — Visibility tightening

**Goal:** Drop unnecessary `pub` markers introduced by the split. Some items had to be `pub` while moving them out of the old file — they can go back to `pub(crate)` or private once the dust settles.

**Estimate:** 20 min. Only do this if Phases 1–4 leave a lot of `pub` on items that are only used from one module.

### Sub-tasks

- [ ] `cargo +nightly clippy -- -W private_interfaces -W unreachable_pub` to surface candidates. Address only the obvious ones.
- [ ] For each `pub` item: if used only inside its own module tree, drop to private. If used cross-module within the crate, drop to `pub(crate)`. Leave `pub` only on items that need to be visible from `main.rs`.

### Checkpoint

- [ ] `cargo build --release` still clean.
- [ ] No new clippy warnings.

---

## Out of scope (separate plans)

These keep coming up but DON'T belong in this refactor — fight the urge:

- **Unifying `ollama.rs` and `openai_compat.rs`** behind a trait. They are 273 + 340 lines respectively and ~30% structurally similar. A trait would help when adding a third backend (Anthropic, Gemini, etc.) — not before. Wait until there's a real third client to motivate the abstraction.
- **Async runtime cleanup.** There are a few places where channels could be replaced by `tokio::sync::watch` or where a `select!` could simplify a polling loop. None of it is broken; it's just stylistic.
- **Testing infrastructure.** The TUI has zero tests today. Adding tests is its own multi-phase plan — pick the seams to test (`parse_command`, `qualifyTurn`-equivalent for tools, `parse_inline_md`) and write a separate `docs/test-plan.md` when ready.
- **Bug fixes spotted during the refactor.** Write them down in a separate `BUGS.md` or new tasks. Do NOT fix them mid-move — it makes the diff impossible to review and breaks the "behavior-preserving only" rule.

---

## Quick-status (the only line you need to skim)

> **Current state:** plan written 2026-05-15, not yet started. `src/` is 8 flat files totalling 4 562 lines; three of them (`app.rs` 1725, `tools.rs` 856, `ui.rs` 743) carry most of the weight. Target: ~17 focused files across 3 module trees + `agent.rs`. Phases 1 → 4 in order; Phase 5 optional. Total estimate ~4 hours of careful, mechanical work. Behavior-preserving — write down any bug you spot, fix it AFTER.
