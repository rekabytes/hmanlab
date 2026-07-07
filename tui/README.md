# hmanlab tui (Go + Bubble Tea)

> **Status:** v0 — walking skeleton. Streaming chat with Ollama Cloud,
> markdown rendering, slash commands, first-run connect modal. The agent
> loop (tools, multi-tool grouping, sessions) is v0.1+.

This is the next-gen TUI for hmanlab, being built in **Go** with
[**Bubble Tea**](https://github.com/charmbracelet/bubbletea). It will
eventually replace the Rust + ratatui client in
[`../cli`](../cli). v0 is intentionally minimal — see
[`docs/development/PRD.md`](docs/development/PRD.md) for the scope and
roadmap.

## Build & run

```sh
cd tui
go run ./cmd/hmanlab-tui
```

Or build a binary:

```sh
go build -o ./bin/hmanlab-tui ./cmd/hmanlab-tui
./bin/hmanlab-tui
```

Tests + lint:

```sh
go test ./...
go vet ./...
```

## First run

The TUI launches even without a config file. You'll see the
**Connect to Ollama Cloud** modal asking for an API key.

1. Get a key from <https://ollama.com>.
2. Paste it into the modal, hit Enter.
3. The TUI validates against `/api/tags`, writes the key to
   `~/.config/hmanlab/config.toml`, and drops you into chat with
   `glm-4.7` as the default model.

Subsequent launches skip the modal — your key is already on file. The
config file is the same one the Rust client (`../cli`) uses, so if you
already have one set up there, the TUI picks it up automatically.

## Keys

| Key | Action |
|---|---|
| **Enter** | Send message |
| **Shift+Enter** | New line (in input box) |
| **Ctrl+C** | Cancel in-flight response (or quit on empty input) |
| **Ctrl+D** | Quit (on empty input) |
| **PgUp / PgDn** | Scroll chat history |
| **mouse wheel** | Scroll chat history |

## Slash commands

| Command | Action |
|---|---|
| `/help` | Show in-app help |
| `/model [name]` | Switch model (no arg: show current). Valid: `glm-4.7`, `glm-5`, `qwen3.5`, `kimi-k2.6`, … |
| `/clear` | Wipe chat history, fresh session |
| `/quit` | Exit |

## Repository layout

```
tui/
├── cmd/hmanlab-tui/           # binary entrypoint
├── docs/development/PRD.md    # v0 scope + roadmap
├── internal/
│   ├── config/                # TOML loader (mirrors cli config.rs schema)
│   ├── llm/                   # Provider interface + Ollama Cloud client
│   └── ui/                    # Bubble Tea Model-Update-View
│       └── theme/             # Catppuccin Mocha palette (cli parity)
├── go.mod
└── README.md                  # this file
```

## Reference contract

The Rust client in [`../cli`](../cli) is the source of truth for every
behavioural edge case. When in doubt, read the Rust source:

| Concern | cli source of truth |
|---|---|
| Config TOML schema | [`cli/src/config.rs`](../cli/src/config.rs) |
| Ollama wire format | [`cli/src/ollama.rs`](../cli/src/ollama.rs) |
| System prompt | [`cli/src/tools/definitions/prompt.rs`](../cli/src/tools/definitions/prompt.rs) |
| Markdown feature set | [`cli/src/ui/markdown.rs`](../cli/src/ui/markdown.rs) |
| Theme palette | [`cli/src/ui/theme.rs`](../cli/src/ui/theme.rs) |

## What's missing vs cli (v0)

The agent surface is **not** in v0 — by design. These features exist
in cli and land in v0.1+:

- Agent loop (tool calls, multi-turn)
- Tool execution pipeline (read / write / edit / git / shell / memory)
- Multi-tool grouping (`⚡⚡⚡ 3 reads …` batch tiles)
- Session persistence (JSONL read+write)
- Specialist agents (`/ask`, `consult_specialist`)
- MCP web-search providers
- Telegram bridge
- Multi-pane layout (sidebar + inspector)
- File viewer, hover/select overlays, drag-select
- Multiple BYOK providers (z.ai, OpenRouter, Minimax, Hibiscus)
- Local Ollama support

See [`docs/development/PRD.md`](docs/development/PRD.md) for the full
roadmap + milestone checklist.

## License

MIT — see [../LICENSE](../LICENSE).
