# hmanlab tui (Go + Bubble Tea)

> **Status:** scaffold only — no code yet.

This is the new TUI for hmanlab, being built in **Go** with
[**Bubble Tea**](https://github.com/charmbracelet/bubbletea) + friends
(`lipgloss`, `bubbles`, `goroutines + channels`). It will replace the
current Rust + ratatui client that lives in [`../cli`](../cli).

## Why a separate codebase?

The Rust client in `../cli/` is mature — 23k LOC, polished rendering,
multi-tool grouping, MCP integration, Telegram bridge, three-pane
layout. Porting it directly to Bubble Tea would be a 2-3 month rewrite
that ships with feature regressions (see
[`../cli/src/ui/`](../cli/src/ui/) for the rendering tricks that don't
translate cleanly to Bubble Tea's Elm-style View model).

Instead, `tui/` is being built greenfield against the same
**behavioural contract** the Rust client establishes, with `cli/`
serving as the reference implementation. Anyone working on the Go side
should read the Rust source first when in doubt about an edge case.

## Reference contract

The two clients must stay interoperable via these shared artifacts:

| Artifact | Source of truth | Notes |
|---|---|---|
| Tool-call JSON schemas | `../cli/src/tools/definitions/` | The fine-tuned models were trained against this exact shape — don't drift. |
| System prompt | `../cli/src/tools/definitions/prompt.rs` | Mirrored as `TRAINING_SYSTEM_PROMPT` in the finetune repo. |
| Session JSONL format | `../cli/src/session.rs` | Sessions persisted by one client should load in the other. |
| Agent loop algorithm | `../cli/src/agent.rs` | Tool-call ordering, `MAX_TURNS=50`, `consult_specialist` dispatch. |
| Stream-msg taxonomy | `../cli/src/app/stream_msg.rs` | Will translate to a Go interface + type switch, but the variants must match 1:1. |

## Plan

- [ ] Scaffold `go.mod` + `cmd/hmanlab-tui/main.go`
- [ ] Config loader that reads the existing `~/.config/hmanlab/config.toml`
- [ ] Ollama + OpenAI-compat streaming client
- [ ] Single-turn chat (no tools) — minimal Bubble Tea loop
- [ ] Tool execution pipeline (read, write, edit, git, shell)
- [ ] Multi-tool grouping render parity with `cli/` ⚡⚡⚡ batch tiles
- [ ] Inspector pane + plan tracking
- [ ] MCP web-search providers
- [ ] Telegram bridge
- [ ] Polish + animations (the part where Bubble Tea catches up to ratatui)

When this list is done, `cli/` gets archived and this folder becomes
the canonical client.
