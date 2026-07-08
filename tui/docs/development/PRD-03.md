# PRD-03 — hmanlab TUI: Agent Loop + Tool Pipeline + Compaction

| | |
|---|---|
| **Status** | Draft |
| **Target** | `tui/` phase 3 — agent loop + tools + compaction |
| **Depends on** | [PRD-02](./PRD-02.md) (shipped — sessions + multi-provider BYOK) |
| **Reference impl** | [`cli/`](../../cli/) (Rust + ratatui, production) |

---

## 1. Vision

PRD-02 made the TUI a persistent, multi-provider chat client. Phase 3
turns it into a **coding agent**: the model can read files, edit code,
run shell commands, and iterate until the task is done — all inside the
TUI, with compact visual tiles for tool calls and automatic context
compaction for long sessions.

Three features, each complete end-to-end:

- **Agent loop.** The model emits tool calls, the TUI executes them
  (read/write/edit/bash/git), feeds results back, and the model
  continues until it produces a final text reply. Up to 50 tool-call
  rounds per turn. Tools run sequentially within a batch.
- **Multi-tool grouping.** Consecutive tool calls collapse into compact
  tiles with a single `✿` flower mark: `✿ 3 reads`, `✿ 5 edits`,
  `✿ 3 calls · read 2 · shell 1` (mixed). Expandable to show individual
  results. Live progress while tools run.
- **Compaction.** When context exceeds 24k tokens, the TUI summarizes
  old messages into a bullet-list briefing, replaces visible history
  with the summary, and continues. Triggered automatically before a
  turn or manually via `/compact`.

No specialist agents, no MCP web-search, no memory store, no image
attachments. Those land in PRD-04+.

---

## 2. Goals for phase 3

### Agent loop + tool pipeline
- Model emits `tool_calls` in a streaming response → TUI executes
  each tool sequentially → results appended to history → model
  re-sent with updated history → loop until final text reply.
- 10 core tools: `read_file`, `list_dir`, `find_files`, `edit_file`,
  `write_file`, `run_command`, `git_status`, `git_diff`, `git_log`,
  `git_show`.
- Mutating tools (`edit_file`, `write_file`, `run_command`) require
  user confirmation via an inline Y/N prompt before execution.
- Tool definitions sent to the model as JSON Schema (same format as
  the CLI).
- Agent runs as a goroutine, communicates back via a channel of
  typed messages (content chunks, tool starts, tool results, done,
  errors).
- Ctrl+C cancels the in-flight agent turn (kills shell children,
  aborts HTTP stream, exits the goroutine cleanly).
- `MAX_TURNS = 50` — hard cap on tool-call rounds.

### Multi-tool grouping UI
- Consecutive `role: "tool"` messages collapse into a single tile
  with one `✿` flower mark (not repeated bolts).
- Uniform groups: `✿ 3 reads`, `✿ 10 reads`.
- Mixed groups: `✿ 3 calls · read 2 · shell 1`.
- Live progress while tools run: `✿ reading files … ▰▰▱ 2/3`.
- Expandable (chevron ⌄) to show per-tool details.
- Recomputed every render frame (pure function over message list).

### Compaction
- Auto-trigger when `last_prompt_tokens > 24,000` before a new turn.
- Manual trigger via `/compact`.
- Renders old messages as a transcript (tool outputs → one-line stubs),
  sends to model with a compaction system prompt, gets back a bullet
  list (8–15 items).
- Replaces visible history with a single `summary` message.
- Prior summaries fold in cumulatively (`PRIOR-SUMMARY:` in transcript).
- Buffered user message replays after compaction completes.

---

## 3. Non-goals for phase 3

- Specialist agents (`/ask`, `consult_specialist`)
- MCP web-search providers
- Memory store (save_memory, read_memory)
- Image attachments / clipboard paste
- File viewer / inspector pane
- Todo / plan tracking
- `multi_edit`, `apply_patch`, `move_lines`, `delete_lines`,
  `insert_at` — ship the core edit tool first; advanced edit variants
  deferred to PRD-04.
- OpenRouter live model catalog refresh

---

## 4. Architecture overview

### New file layout (additions to phase 2)

```
tui/
├── internal/
│   ├── agent/
│   │   ├── agent.go              # NEW: the agent loop goroutine
│   │   └── message.go            # NEW: StreamMsg type (content/tool/done/error)
│   ├── tools/
│   │   ├── tools.go              # NEW: Tool type, ToolContext, execute_tool dispatch
│   │   ├── definitions.go        # NEW: tool_definitions() — JSON Schema for all tools
│   │   ├── read.go               # NEW: read_file, list_dir, find_files
│   │   ├── write.go              # NEW: edit_file, write_file + confirm + diff
│   │   ├── shell.go              # NEW: run_command (streaming + kill)
│   │   ├── git.go                # NEW: git_status, git_diff, git_log, git_show
│   │   ├── workspace.go          # NEW: path safety (resolve_in_workspace)
│   │   └── diff.go               # NEW: DiffLine type + diff builder
│   ├── llm/
│   │   ├── provider.go           # expanded: Tool, ToolCall types added
│   │   ├── ollama.go             # expanded: tool_calls in request + response
│   │   └── openai_compat.go      # expanded: tool_calls in request + response
│   ├── compact/
│   │   └── compact.go            # NEW: compact_history() + transcript renderer
│   └── ui/
│       ├── model.go              # expanded: agent state, confirm flow
│       ├── chat.go               # expanded: tool message rendering
│       ├── commands.go           # expanded: /compact command
│       └── tool_tile.go          # NEW: grouped tool tile rendering
├── docs/development/
│   └── PRD-03.md                 # this document
```

### Agent loop design

The agent runs as a **detached goroutine** — it holds no reference to
the Model and communicates only via a `chan agent.StreamMsg`. This
mirrors the CLI's design (`agent.rs:5-7`).

```go
// agent/message.go
type StreamMsg struct {
    Kind        StreamMsgKind
    Text        string          // Chunk
    ToolName    string          // ToolStart, ToolResult
    ToolArgs     json.RawMessage // ToolStart
    ToolOutput  string          // ToolResult
    Err         error           // Error
    PromptTokens    int         // Done
    CompletionTokens int        // Done
    Confirm     *ConfirmRequest // ConfirmRequest
}

type StreamMsgKind int
const (
    MsgChunk StreamMsgKind = iota
    MsgToolStart
    MsgToolResult
    MsgDone
    MsgError
    MsgConfirmRequest
    MsgNewTurn
)

type ConfirmRequest struct {
    Prompt  string
    Diff    []DiffLine
    Reply   chan bool
}
```

```go
// agent/agent.go
func RunLoop(
    backend   llm.Provider,
    model     string,
    history   []llm.Message,
    workspace string,
    toolDefs  []llm.Tool,
    ch        chan<- agent.StreamMsg,
)
```

**Loop pseudocode** (mirrors `cli/src/agent.rs:94-181`):
```
fullHistory = [systemPrompt] + history
for turn := 0; turn < 50; turn++ {
    streamCh = backend.StreamChat(ctx, model, fullHistory, toolDefs)
    content = ""
    toolCalls = []
    for ev := range streamCh {
        if ev.Text != "" { ch <- Chunk(ev.Text); content += ev.Text }
        if ev.ToolCalls != nil { toolCalls = append(toolCalls, ev.ToolCalls...) }
        if ev.Done { promptTokens = ev.PromptTokens; completionTokens = ev.CompletionTokens }
        if ev.Err != nil { ch <- Error(ev.Err); return }
    }
    if len(toolCalls) == 0 {
        ch <- Done(promptTokens, completionTokens)
        return
    }
    fullHistory.append(assistantWithToolCalls(content, toolCalls))
    for i, tc := range toolCalls {
        ch <- ToolStart(tc.Name, tc.Args)
        output = executeTool(tc.Name, tc.Args, ctx)  // may block on confirm
        ch <- ToolResult(output)
        fullHistory.append(toolResult(tc.Name, output, "call_"+i))
    }
    ch <- NewTurn
}
ch <- Error("agent stopped after 50 tool-call rounds")
```

### Tool pipeline design

**Tool type** (added to `llm/provider.go`):
```go
type Tool struct {
    Type     string         `json:"type"`      // always "function"
    Function ToolFunction   `json:"function"`
}
type ToolFunction struct {
    Name        string         `json:"name"`
    Description string         `json:"description"`
    Parameters  map[string]any `json:"parameters"` // JSON Schema
}
type ToolCall struct {
    Function ToolCallFunction `json:"function"`
}
type ToolCallFunction struct {
    Name      string          `json:"name"`
    Arguments json.RawMessage `json:"arguments"`
}
```

**Provider interface** — `StreamChat` gains an optional `tools` parameter:
```go
type Provider interface {
    Name() string
    Models() []string
    StreamChat(ctx context.Context, model string, messages []Message, tools []Tool) <-chan StreamEvent
}
```

`StreamEvent` gains a `ToolCalls []ToolCall` field.

**ToolContext:**
```go
type ToolContext struct {
    Workspace string
    Confirm   func(prompt string, diff []DiffLine) bool
}
```

**Dispatch** (`tools/tools.go`):
```go
func Execute(name string, args json.RawMessage, ctx *ToolContext) (string, error)
```

Matches `cli/src/tools/mod.rs:140-200`. Resolves aliases (`Read` →
`read_file`, `Bash` → `run_command`, etc.) before dispatch.

### Multi-tool grouping design

Pure functions over `[]chatMessage`, recomputed every render frame.
Mirrors `cli/src/ui/chat/helpers.rs`.

```go
type ToolGroup struct {
    Start, End int  // inclusive indices into history
}

func ComputeToolGroups(messages []chatMessage) []ToolGroup
func SummarizeGroup(messages []chatMessage, g ToolGroup) GroupSummary
```

**Grouping logic:** maximal runs of consecutive `role: "tool"` messages.
Any non-tool message breaks a run.

**Tile rendering** (`tool_tile.go`):
- Single `✿` flower mark (not repeated — one flower regardless of count)
- Uniform: `✿ 3 reads`
- Mixed: `✿ 3 calls · read 2 · shell 1`
- In-flight: `✿ reading files … ▰▰▱ 2/3`
- Expandable: chevron ⌄ reveals individual tool rows
- Animations: progress bar (▰▰▱▱▱ fills over 6s), breathing count
  (`2/3` pulses), animated ellipsis (`.` `..` `...` ` `), hibiscus
  color pulse on the flower mark while tools run

### Compaction design

```go
// compact/compact.go
func CompactHistory(
    backend llm.Provider,
    model string,
    messages []llm.Message,
) (summary string, promptTokens, completionTokens int, err error)
```

**Transcript rendering** — converts messages to a flat text string:
- `user` → `USER: {content}`
- `assistant` → `ASSISTANT: {content}` (strips `<think>`)
- `tool` → `[tool {name} → {N} lines]` (one-line stub, NOT full output)
- `summary` → `PRIOR-SUMMARY:\n{content}`

**Auto-trigger** in `sendUserMessage`:
```go
if m.lastPromptTokens > 24000 && !m.streaming && !m.compacting {
    m.startCompaction(text)  // buffers user message
    return
}
m.startTurn(text)
```

**Result:** replaces `m.history` with a single summary message, resets
`lastPromptTokens = 0`, replays buffered user message.

---

## 5. Tool inventory

### Read-only (no approval)

| Tool | Params | Notes |
|---|---|---|
| `read_file` | `path` | 50 KB cap, lines prefixed `<n>\t` |
| `list_dir` | `path?` (default `.`) | dirs suffixed `/`, symlinks `@` |
| `find_files` | `pattern` | glob, filters node_modules/.git/etc, 100 cap |
| `git_status` | — | `git status --porcelain=v1 -b` |
| `git_log` | `limit?` (default 10, max 100) | `git log --oneline -n N` |
| `git_diff` | `range?`, `path?` | |
| `git_show` | `rev` | rev char-whitelisted |

### Mutating (require Y/N confirmation)

| Tool | Params | Notes |
|---|---|---|
| `edit_file` | `path`, `old_string`, `new_string`, `replace_all?` | fuzzy match, diff preview |
| `write_file` | `path`, `content` | 500 KB cap |
| `run_command` | `command` | `sh -c`, 600s timeout, streaming output, killable |

### Constants

| Constant | Value |
|---|---|
| `MAX_TURNS` | 50 |
| `MAX_FILE_BYTES` | 50,000 |
| `MAX_CMD_BYTES` | 4,000 |
| `MAX_WRITE_BYTES` | 500,000 |
| `CMD_TIMEOUT` | 600s |
| `FIND_FILES_CAP` | 100 |
| `AUTO_COMPACT_THRESHOLD` | 24,000 tokens |

---

## 6. Wire format changes

### Ollama native (`/api/chat`) — tool support added

**Request** gains a `tools` field:
```json
{
  "model": "glm-4.7",
  "messages": [...],
  "stream": true,
  "tools": [
    {"type":"function","function":{"name":"read_file","description":"...","parameters":{...}}}
  ]
}
```

**Response** — tool calls in `message.tool_calls`:
```json
{"message":{"content":"Let me read that.","tool_calls":[{"function":{"name":"read_file","arguments":{"path":"main.go"}}}]},"done":false}
{"message":{"content":""},"done":true,"prompt_eval_count":1234,"eval_count":56}
```

### OpenAI-compat (`/chat/completions`) — tool support added

**Request** gains `tools`:
```json
{
  "model": "glm-4.7",
  "messages": [...],
  "stream": true,
  "max_tokens": 32000,
  "tools": [{"type":"function","function":{...}}],
  "tool_choice": "auto"
}
```

**Response** — tool calls arrive as fragmented deltas:
```json
data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"read_file","arguments":"{\"pa"}}]}}]}
data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"th\":\"main.go\"}"}}]}}]}
data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1234,"completion_tokens":56}}
data: [DONE]
```

Arguments arrive as **fragmented strings** — accumulate by `index`,
concatenate, then parse as JSON.

Assistant tool_calls get synthesized `id: "call_<i>"` for OpenAI-compat
providers. Tool results sent back with `tool_call_id: "call_<i>"`.

---

## 7. Confirmation flow

When a mutating tool is called, the agent goroutine sends a
`MsgConfirmRequest` with a `chan bool`. The UI shows an inline Y/N
card in the chat history. The user presses Y or N, which sends the
reply back through the channel. The goroutine blocks until the reply
arrives.

```
┌─ chat history ─────────────────────────────┐
│  ▎ user                                     │
│  fix the bug in main.go                     │
│                                             │
│  ✿ hibiscus                                 │
│  Let me look at the file…                   │
│  ✿ read main.go                             │
│                                             │
│  ✿ hibiscus                                 │
│  I see the bug. Let me fix it.              │
│  ✿ edit main.go                             │
│  ┌─ Confirm ─────────────────────────────┐ │
│  │ Edit main.go                           │ │
│  │ - fmt.Println("hello")                 │ │
│  │ + fmt.Println("hello, world")          │ │
│  │ [Y] allow   [N] deny                   │ │
│  └────────────────────────────────────────┘ │
└─────────────────────────────────────────────┘
```

---

## 8. Session persistence changes

Tool records are written to the JSONL file:
```json
{"type":"tool","session_id":"...","uuid":"...","timestamp":"...","content":"1\t...","name":"read_file"}
```

`<think>…</think>` blocks are stripped from assistant content before
persistence (matching CLI behavior).

Tool-call IDs (`call_0`, `call_1`, etc.) are NOT persisted — they're
per-request artifacts. When loading a session, tool-call IDs are
backfilled positionally (`backfill_tool_call_ids`) for strict
OpenAI-compat providers.

---

## 9. Milestones

- [ ] **M1 — Tool types + definitions** (~2 days)
      `llm/provider.go` gains `Tool`, `ToolCall` types. `tools/definitions.go`
      with JSON Schema for all 10 tools. `tools/tools.go` dispatch skeleton.
      `tools/workspace.go` path safety. No execution yet.

- [ ] **M2 — Read tools** (~2 days)
      `tools/read.go` (read_file, list_dir, find_files). `tools/git.go`
      (git_status, git_log, git_diff, git_show). Test each tool standalone.

- [ ] **M3 — Write tools + confirmation** (~3 days)
      `tools/write.go` (edit_file, write_file). `tools/diff.go` diff builder.
      Inline Y/N confirmation card in chat UI. Diff preview rendering.

- [ ] **M4 — Shell tool** (~2 days)
      `tools/shell.go` (run_command). Streaming output, kill via Ctrl+C,
      600s timeout. Live output monitor in chat.

- [ ] **M5 — Wire format: tool_calls in streaming** (~3 days)
      Expand `ollama.go` + `openai_compat.go` to send `tools` in the
      request and parse `tool_calls` in the response. Fragmented argument
      accumulation for OpenAI-compat. `StreamEvent.ToolCalls` field.
      Test with mocked SSE + real z.ai key.

- [ ] **M6 — Agent loop** (~3 days)
      `agent/agent.go` goroutine. `agent/message.go` StreamMsg types.
      Wire into Model: spawn goroutine on send, drain channel on tick.
      Ctrl+C cancels (kills goroutine + shell children).

- [ ] **M7 — Multi-tool grouping UI** (~2 days)
      `tool_tile.go`: ComputeToolGroups, SummarizeGroup, render tiles.
      ⚡ bolts, progress bars, expand/collapse. Recompute every frame.

- [ ] **M8 — Compaction** (~2 days)
      `compact/compact.go`: transcript renderer + model call. Auto-trigger
      at 24k tokens. `/compact` manual trigger. History replacement +
      buffered message replay.

- [ ] **M9 — Polish** (~2 days)
      Error states, edge cases, session persistence of tool records,
      `<think>` stripping, alias resolution. `go vet` clean. README.

**Total estimate: ~21 working days** for one developer.

---

## 10. Risks & unknowns

| Risk | Severity | Mitigation |
|---|---|---|
| Fragmented tool-call argument parsing in OpenAI-compat SSE — each provider splits differently | **High** | Port CLI's test fixtures. Test with z.ai + OpenRouter early. Accumulate by index, concatenate, parse. |
| Goroutine leak on Ctrl+C — agent goroutine + shell child must die cleanly | **High** | Use `context.WithCancel`. Shell children get `kill_on_drop` equivalent (process.Kill on ctx.Done). |
| Confirmation deadlock — agent goroutine blocks on `chan bool` if UI never responds | Medium | UI always sends a reply (Y or N). On Ctrl+C during confirm, send N. Timeout after 5 min → N. |
| Tool path escape — `../../etc/passwd` | **High** | `resolve_in_workspace` canonicalizes both paths, rejects anything outside workspace. Port CLI's `tools/workspace.rs`. |
| Shell command hangs forever | Medium | 600s timeout. Live monitor shows output. User can Ctrl+C to kill. |
| Compaction summary quality — model produces bad summary | Medium | Tight system prompt (8-15 bullets, specific categories). Test with real sessions. |
| Tool-call ID backfill for loaded sessions — strict providers 400 without IDs | Medium | Positional `call_<i>` synthesis. `backfill_tool_call_ids` before sending. |
| `<think>` tag handling across 4 contexts (render, persist, compact, display) | Low | Centralize in one `stripThinking()` function. |

---

## 11. Success criteria

Phase 3 ships when **all** of these are true:

### Agent loop
- [ ] Ask the model to "read main.go and fix the import" — it calls
      `read_file`, sees the content, calls `edit_file`, confirms the
      edit, then produces a final text reply.
- [ ] Multiple tool calls in one response execute sequentially.
- [ ] Ctrl+C mid-turn kills the goroutine + any running shell command.
- [ ] MAX_TURNS (50) cap works — agent stops with an error message.

### Tool grouping
- [ ] 3 consecutive `read_file` calls render as `✿ 3 reads`.
- [ ] 5+ calls render as `✿ 5 reads`.
- [ ] Mixed tools render as `✿ 3 calls · read 2 · shell 1`.
- [ ] In-flight tools show progress: `✿ reading files … 2/3`.
- [ ] Tiles expand to show individual tool results.

### Compaction
- [ ] After 24k+ tokens, sending a message auto-triggers compaction.
- [ ] `/compact` manually triggers compaction.
- [ ] History is replaced with a summary; conversation continues.
- [ ] Prior summaries fold in cumulatively.

### Quality
- [ ] `go test ./...` passes with tests for:
      - Tool dispatch (alias resolution, path safety)
      - Tool grouping (uniform, mixed, in-flight)
      - Compaction transcript rendering
      - SSE tool-call fragment accumulation (mocked)
- [ ] `go vet` clean.
- [ ] No goroutine leaks (agent goroutine exits, shell children killed).
