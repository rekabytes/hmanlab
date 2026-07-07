# PRD — hmanlab TUI (Go + Bubble Tea), v0

| | |
|---|---|
| **Status** | Draft |
| **Target** | `tui/` v0 — walking skeleton |
| **Scope** | Single-pane chat with Ollama Cloud. No agent loop, no tools. |
| **Reference impl** | [`cli/`](../../cli/) (Rust + ratatui, production) |

---

## 1. Vision

The Rust + ratatui client in [`cli/`](../../cli/) is mature and shipping — 23k
LOC, polished rendering, multi-tool grouping, MCP, Telegram, three-pane
layout. Porting it directly to Go + Bubble Tea would be a 2-3 month rewrite
that ships with feature regressions (see
[`cli/src/ui/`](../../cli/src/ui/) for the buffer-mutation tricks that
don't translate to Bubble Tea's Elm-style View model).

This TUI is being built **greenfield** in Go with
[Bubble Tea](https://github.com/charmbracelet/bubbletea). v0 is the
**smallest possible proof that the architecture works end-to-end**:
type a message, stream a response from Ollama Cloud, render it as
markdown. No tools, no agent loop, no sessions. The point is to de-risk
the framework choice cheaply before investing in the harder layers
(rendering parity, tool surface, multi-pane layout).

[`cli/`](../../cli/) stays the production client and the **reference
implementation**. Anyone working on the Go side reads the Rust source
first when in doubt about an edge case — it's the source of truth
until the Go client reaches parity.

---

## 2. Goals for v0

**Walking skeleton. One feature, end-to-end.**

- One thing works: type message → stream response from Ollama Cloud →
  see markdown-rendered reply.
- Reads existing `~/.config/hmanlab/config.toml` — zero re-config for
  existing cli users.
- Clean Elm-style architecture (Model-Update-View split, goroutine-based
  streaming) so layering tools on top in v0.1 doesn't require a rewrite.
- Terminal state always restored on exit / Ctrl+C — no leaked
  goroutines, no raw-mode leaks.
- First-run UX: if no config or no Ollama Cloud key, the TUI still
  launches and shows an inline connect-to-provider prompt.

## 3. Non-goals for v0

Explicitly **deferred to v0.1+**:

- Agent loop (model calls tools, sees results, continues)
- Tool execution pipeline (read / write / edit / git / shell / memory)
- Multi-tool grouping (`⚡⚡⚡ 3 reads …` batch tiles)
- Session persistence (JSONL read+write)
- Specialist agents (`/ask`, `consult_specialist`)
- MCP web-search providers
- Telegram bridge
- Multi-pane layout (sidebar, inspector)
- File viewer
- Hover/select overlays, drag-select, copy-on-drag
- Plan / todo-list tracking
- Compaction
- Clipboard paste + image attachments
- Multiple BYOK providers (z.ai, OpenRouter, Minimax, Hibiscus, etc.)
- Animations beyond the streaming caret
- Local Ollama support

Each of these is a real feature in [`cli/`](../../cli/) and will be
ported in priority order after v0 ships.

---

## 4. Reference contract

The cli's existing artifacts are the source of truth. Mirror them
byte-for-byte where the wire is observable; diverge only on internal
implementation.

| Concern | Source of truth | Notes |
|---|---|---|
| Config TOML schema | [`cli/src/config.rs`](../../cli/src/config.rs) | Field names + nesting must match so the same `config.toml` works in both clients. |
| Ollama wire format | [`cli/src/ollama.rs`](../../cli/src/ollama.rs) | `/api/chat` streaming SSE format, chunk delimiters, error shapes. |
| System prompt | [`cli/src/tools/definitions/prompt.rs`](../../cli/src/tools/definitions/prompt.rs) | Use verbatim — model behavior depends on it. |
| Markdown feature set | [`cli/src/ui/markdown.rs`](../../cli/src/ui/markdown.rs) | Target parity: code blocks, inline code, bold, italics, bulleted + numbered lists, links, blockquotes. |
| Provider constants | [`cli/src/config.rs`](../../cli/src/config.rs) | `OLLAMA_CLOUD_PROVIDER = "ollama-cloud"`, `OLLAMA_CLOUD_BASE = "https://ollama.com"`, `OLLAMA_CLOUD_DEFAULT_MODEL = "glm-4.7"`. |
| Theme palette | [`cli/src/ui/theme.rs`](../../cli/src/ui/theme.rs) | Catppuccin Mocha. RGB literals must match so the two clients look identical. |

---

## 5. Architecture overview

### File layout

```
tui/
├── cmd/hmanlab-tui/
│   └── main.go                    # entrypoint: load config, init tea.Program
├── internal/
│   ├── config/
│   │   └── config.go              # TOML loader (mirrors cli config.rs schema)
│   ├── llm/
│   │   ├── provider.go            # Provider interface (forward-compat for v0.1)
│   │   └── ollama.go              # Ollama Cloud streaming client
│   └── ui/
│       ├── model.go               # top-level Bubble Tea Model
│       ├── update.go              # Update(): tea.Msg dispatch
│       ├── view.go                # View(): single-pane render
│       ├── chat.go                # chat history viewport
│       ├── input.go               # bubbles/textarea wrapper
│       ├── markdown.go            # renderer (chroma for code highlighting)
│       ├── connect.go             # first-run "connect to provider" modal
│       └── theme.go               # Catppuccin Mocha palette (cli parity)
├── docs/                          # this PRD + future design docs
├── go.mod
└── README.md
```

### Bubble Tea idioms

- **Model-Update-View**: state lives on `model`; `Update` returns
  `(model, tea.Cmd)`; `View` returns a string. No mutation of style
  post-render (Bubble Tea has no equivalent of ratatui's `buffer_mut`).
- **Streaming**: `tea.Cmd` returns a `func() tea.Msg` that opens the
  HTTP request, reads the SSE stream chunk-by-chunk, and emits
  `chunkMsg{tokens: "..."}` per chunk. The main loop appends tokens to
  the in-flight assistant message and triggers a re-render.
- **Cancellation**: in-flight requests carry a `context.Context`. Ctrl+C
  sends a `cancelMsg`; the streaming goroutine observes
  `ctx.Done()` and returns. No leaked goroutines.
- **Scrollback**: `bubbles/viewport` for the chat history. Auto-follow
  tail while generating; user can scroll up with mouse wheel or PgUp.

### Ollama Cloud client

- POST `${OLLAMA_CLOUD_BASE}/api/chat` with the standard Ollama
  chat-request body + `stream: true`.
- `Authorization: Bearer ${OLLAMA_CLOUD_API_KEY}` header.
- Response is newline-delimited JSON (one chunk object per line,
  terminated by a final `{ "done": true }` sentinel).
- Match [`cli/src/ollama.rs`](../../cli/src/ollama.rs) for chunk
  parsing exactly — the cli has seen every edge case in production.

---

## 6. v0 feature spec

Concrete checklist — v0 ships when every box is checked.

### Config
- [ ] Reads `~/.config/hmanlab/config.toml` on startup.
- [ ] Reads the existing `ollama_cloud_api_key` field (no schema drift).
- [ ] If absent or key missing, defers to the first-run flow (below).

### First-run flow
- [ ] TUI launches even without a config or API key.
- [ ] Detects missing key, switches to "Connect to Ollama Cloud" modal.
- [ ] Modal shows: provider name (Ollama Cloud), key input field,
      "Get a key from https://ollama.com" hint, `[ connect ]` /
      `[ quit ]` buttons.
- [ ] On submit: writes key to config.toml (creating the file if
      needed), tests the key with a `/api/tags` call, then drops into
      the chat view on success.
- [ ] On failure: stays in the modal, shows the error inline.

### Chat
- [ ] Multi-line input (bubbles/textarea): Enter to send,
      Shift+Enter for newline.
- [ ] On send: append user message to history, open streaming request,
      append placeholder assistant message, stream tokens into it.
- [ ] Streaming caret (blinks while generating, ~480ms cycle to match
      cli's `anim_tick / 4`).
- [ ] Ctrl+C mid-stream cancels the request, leaves partial response
      in history with a trailing `[…]` marker.

### Markdown rendering
- [ ] Fenced code blocks (``` ``` ```) with chroma syntax highlighting.
- [ ] Inline code (`` `code` ``).
- [ ] Bold (`**x**`), italics (`*x*` / `_x_`).
- [ ] Bulleted lists (`- x`, `* x`, `+ x`) and numbered lists (`1. x`).
- [ ] Links (`[text](url)`).
- [ ] Blockquotes (`> x`).
- [ ] Headings (`# x` through `###### x`) — rendered with size/weight
      difference via lipgloss styles.

### Slash commands
- [ ] `/help` — list available commands.
- [ ] `/clear` — wipe chat history, fresh session.
- [ ] `/quit` (or Ctrl+D in empty input) — exit cleanly.
- [ ] `/model <name>` — switch active model (validated against the
      Ollama Cloud catalog from `OLLAMA_CLOUD_MODELS`).
- [ ] `/model` (no arg) — show current model.

### Error handling
- [ ] Network errors surface inline in chat as a styled error line.
- [ ] Auth errors (401 / 403) trigger a re-prompt for the API key.
- [ ] Model-not-found errors list the valid catalog.
- [ ] App never crashes — every error path is recoverable.

### Terminal hygiene
- [ ] Raw mode + alt-screen on entry, restored on exit.
- [ ] Mouse capture enabled (for scroll wheel) and disabled on exit.
- [ ] Clean exit on Ctrl+C, Ctrl+D, `/quit`, window-resize, SIGTERM.
- [ ] `go vet` + `golangci-lint` clean.

---

## 7. First-run flow (detailed)

When the TUI launches:

1. **Try to load config.** If `~/.config/hmanlab/config.toml` doesn't
   exist or doesn't contain an `ollama_cloud_api_key`, switch to the
   connect modal before rendering the chat view.

2. **Connect modal.** A centered card over a dimmed backdrop:
   ```
   ┌─ Connect to Ollama Cloud ───────────────────────┐
   │                                                   │
   │   Ollama Cloud                                    │
   │   Stream from `https://ollama.com`                │
   │                                                   │
   │   Get a key at: https://ollama.com                │
   │                                                   │
   │   API key:  sk-_________________________________  │
   │                                                   │
   │   [ connect ]    [ quit ]                         │
   └───────────────────────────────────────────────────┘
   ```

3. **On connect.** Validate the key with a `GET /api/tags` call. On
   success: write the key to `config.toml`, switch to chat view with
   `glm-4.7` as the default model. On failure: red-text error inline,
   stay in modal.

4. **Subsequent launches.** Skip straight to chat view.

This mirrors the cli's `AddModel` modal flow (`cli/src/app/state.rs:211`)
so the UX feels native to existing users.

---

## 8. Risks & unknowns

| Risk | Severity | Mitigation |
|---|---|---|
| `bubbles/textarea` is less polished than Rust's `tui-textarea` — soft-wrap cursor positioning, multi-byte glyph widths | **High** | Test early (M4). If broken, patch upstream or wrap with our own cursor math. |
| Bubble Tea's `View()` can't post-process cells the way ratatui's `buffer_mut()` can — selection rectangle, hover overlay, animated cell-rewrite aren't possible | Medium | Accept for v0 — those features stay in cli. Revisit in v0.2 with overlapping absolute-positioned components. |
| chroma (Go syntax highlighter) is heavier than the Rust equivalent — first render may lag | Medium | Lazy-load on first code block. Cache styles in `sync.Once`. |
| Ollama Cloud streaming SSE quirks — must match `cli/src/ollama.rs` byte-for-byte | Medium | Port the cli's test fixtures (`cli/src/ollama.rs` tests) as Go test cases. |
| Ollama Cloud API may have undocumented rate limits or chunk-delimiter quirks | Low | Reuse the cli's production-hardened parser; report any new edge cases back to cli. |
| First-run TOML write path could race with concurrent cli process | Low | Atomic write (temp file + rename). Same pattern as cli's config writer. |

---

## 9. Success criteria

v0 ships when **all** of these are true:

- [ ] `go run ./cmd/hmanlab-tui` from `tui/` launches cleanly.
- [ ] First-run with no config shows the connect modal; valid key drops
      into chat with no restart.
- [ ] Existing cli users with a working config.toml can use the new TUI
      without re-configuring.
- [ ] A 5-message back-and-forth with `glm-4.7` works, with streaming
      + markdown rendering (code blocks, bold, lists).
- [ ] `/clear`, `/quit`, `/help`, `/model` all work.
- [ ] Ctrl+C mid-stream cancels cleanly, terminal state restored.
- [ ] `go test ./...` passes with tests for: config loader, Ollama
      Cloud client (mocked SSE), markdown renderer, slash command
      parser, first-run modal state machine.
- [ ] `go vet` + `golangci-lint` clean.

---

## 10. Milestones

Suggested build order — each milestone is a meaningful checkpoint, not
just a checklist item. Solo developer estimate; double for a team
coordinate-by-default.

- [ ] **M1 — Scaffold** (~2 days)
      `go.mod`, `cmd/hmanlab-tui/main.go`, config loader compiles,
      empty Bubble Tea program runs. No network yet.

- [ ] **M2 — Ollama Cloud client** (~3 days)
      POST + SSE parse against `https://ollama.com/api/chat`. CLI-only
      test harness prints streamed tokens to stdout. No TUI yet —
      prove the wire format in isolation.

- [ ] **M3 — Minimal Bubble Tea loop** (~2 days)
      Single-line input, response prints below, no markdown, no
      scrollback. Proves the tea.Cmd streaming pattern.

- [ ] **M4 — Multi-line textarea + scrollback** (~3 days)
      `bubbles/textarea` wired in with Enter-to-send /
      Shift+Enter-newline. `bubbles/viewport` for history with
      follow-tail. This is where we learn if `bubbles/textarea` is
      good enough.

- [ ] **M5 — Markdown rendering** (~4 days)
      Code blocks (chroma), bold, italics, inline code, lists, links,
      blockquotes, headings. Target parity with
      `cli/src/ui/markdown.rs`.

- [ ] **M6 — Slash commands + model picker** (~2 days)
      `/help`, `/clear`, `/quit`, `/model`. Model picker validates
      against `OLLAMA_CLOUD_MODELS`.

- [ ] **M7 — First-run connect modal** (~3 days)
      Detect missing key, modal state machine, TOML write,
      `/api/tags` validation, error handling.

- [ ] **M8 — Streaming caret + Ctrl+C cancellation** (~2 days)
      Blinking caret while generating, context-based cancellation,
      partial-response marker.

- [ ] **M9 — Polish** (~3 days)
      Error states, edge cases (network drop, model deprecation),
      terminal restoration under all exit paths, `go vet` +
      `golangci-lint` clean, README.

**Total estimate: ~24 working days** for one developer who knows both
Go and the cli codebase.

---

## 11. Out of scope — explicit list

Features that exist in [`cli/`](../../cli/) but **will not** ship in v0.
Each will get its own design doc when scheduled for v0.1+.

- Agent loop (`cli/src/agent.rs`)
- Tool execution pipeline (`cli/src/tools/`)
- Multi-tool grouping (`cli/src/ui/chat/helpers.rs::compute_tool_groups`)
- Session JSONL persistence (`cli/src/session.rs`)
- Specialist agents (`/ask`, `consult_specialist`)
- MCP web-search providers (`cli/src/app/mcp_providers.rs`)
- Telegram bridge (`cli/src/telegram.rs`)
- Three-pane layout (`cli/src/ui/layout.rs`)
- Sidebar with sessions/agents tabs (`cli/src/ui/sidebar.rs`)
- Inspector pane with plan tracking (`cli/src/ui/inspector.rs`)
- File viewer (`cli/src/ui/viewer.rs`)
- Hover/select overlays, drag-select, copy-on-drag
- Compaction (`cli/src/compact.rs`)
- Clipboard paste + image attachments
- Multiple BYOK providers (z.ai, OpenRouter, Minimax, Hibiscus)
- Local Ollama support
- Update checker (`cli/src/update_check.rs`)
- Workspace trust gate

---

## 12. Open questions

Decisions to make before/during implementation — not blockers for v0
itself, but worth surfacing now.

- **Branding.** Does this binary ship as `hmanlab-tui` (separate
  command) or eventually replace `hmanlab`? Phase as `hmanlab-tui` for
  v0-v0.2, rename at v1.0 when cli is archived?
- **Distribution.** curl installer + npm shim like cli, or Go-native
  `go install github.com/hmanlab/hmanlab/tui/cmd/hmanlab-tui@latest`?
  Both?
- **Telemetry / crash reports.** cli has none. Continue that?
- **i18n.** cli is English-only. Stay English-only for v0?
- **Windows.** cli supports Windows via the npm shim path. Does v0
  target Windows, or POSIX-first + Windows later?
