<h1 align="center">Hibiscus</h1>

<p align="center">
  <b>The agentic terminal client for any LLM you have a key for.</b><br>
  Local Ollama · Cloud Ollama · z.ai · OpenCode Go · Hibiscus · OpenRouter · No account required
</p>

<p align="center">
  <a href="https://www.npmjs.com/package/hmanlab"><img alt="npm" src="https://img.shields.io/npm/v/hmanlab?label=npm&color=cb3837"></a>
  <a href="https://github.com/hmanlab/hmanlab/actions/workflows/ci.yml"><img alt="ci" src="https://github.com/hmanlab/hmanlab/actions/workflows/ci.yml/badge.svg"></a>
  <a href="LICENSE"><img alt="license" src="https://img.shields.io/badge/license-MIT-blue"></a>
  <a href="https://github.com/hmanlab/hmanlab/releases"><img alt="downloads" src="https://img.shields.io/github/downloads/hmanlab/hmanlab/total?label=downloads&color=green"></a>
  <img alt="status" src="https://img.shields.io/badge/status-alpha-orange">
</p>

<!-- Drop a demo recording here once you have one (asciinema cast or VHS-generated GIF).
<p align="center">
  <img src="docs/demo.gif" alt="hmanlab demo" width="800">
</p>
-->

Built in [Rust](https://www.rust-lang.org) with [ratatui](https://ratatui.rs). Streams from any OpenAI-compatible endpoint or native Ollama. **No account required** — download, connect a provider, and chat. Sessions persist locally as JSONL files (no server).

---

## Install

| Method | Command | Binary location |
|---|---|---|
| **Curl** | `curl -fsSL https://github.com/hmanlab/hmanlab/releases/latest/download/install.sh \| sh` | `~/.local/bin/hmanlab` |
| **npm (global)** | `npm i -g hmanlab` | `$(npm root -g)/../bin/hmanlab` |
| **npm (one-off)** | `npx hmanlab` | (no install) |
| **From source** | `cargo install --git https://github.com/hmanlab/hmanlab` | `~/.cargo/bin/hmanlab` |

Prebuilt binaries cover `linux-x64`, `linux-arm64`, `darwin-x64`, `darwin-arm64`, and `win32-x64`. Windows users: use the npm path — the curl installer is POSIX-only.

> **Pick one install method per machine and stick with it.** Each channel drops the binary in its own directory; mixing channels is the most common reason "updates don't take effect."

---

## First run

Launch `hibiscus` — no key, no account needed. On first launch, an optional wizard lets you connect a provider:

1. **Choose a provider** (all optional — skip everything and add later with `/model`):
   - Local Ollama URL
   - z.ai subscription or usage-based key
   - Ollama Cloud key
   - OpenCode Go key
   - OpenRouter key
   - Hibiscus key (`sk-…` from [ai.hmanlab.pro](https://ai.hmanlab.pro))
   - MiniMax key
2. **Chat.** That's it.

If you already have Ollama running locally, you can skip the wizard entirely — hibiscus will pick it up automatically if you pass `--host`.

---

## Features

### Chat

- **Streaming replies** — tokens render as they arrive from any provider.
- **Multi-provider** — local Ollama, Ollama Cloud, z.ai (subscription + usage-based), OpenCode Go, OpenRouter, and Hibiscus from one TUI; switch with `/model` or `Ctrl+M`.
- **Inline markdown** — `**bold**` and `` `code` `` render styled in the chat panel.
- **Thinking block folding** — `<think>…</think>` reasoning blocks collapse by default; `Ctrl+T` to expand.
- **Y/N quick-reply** — when the model asks a yes/no question, just press `Y` or `N`.
- **Inline autocomplete** — type `/` for slash-command autocomplete, `@` for file/folder mention. ↑↓ to navigate, Tab/Enter to insert.

### Tools & memory

- **Agentic tool calls** — the model reads files, explores directories, runs git commands, edits/writes files, executes shell commands, and recalls persistent memories. Every destructive action requires your confirmation with a diff preview.
- **Workspace trust** — on first launch in a new directory, Hibiscus asks whether to trust it. Untrusted workspaces allow read-only tools but block destructive ones. Use `/trust` or `/untrust` to change later.
- **Persistent memory** — save and recall durable facts across sessions. Two scopes: user-wide (`~/.hmanlab/memory/`) and project-local (`<workspace>/.hmanlab/memory/`).
- **Auto-compaction** — when the context window fills up, old turns are summarised so the conversation keeps going.

### Sessions & UX

- **Local session persistence** — chats save automatically to `~/.config/hmanlab/sessions/` as JSONL files. No server, no login. See [docs/sessions.md](docs/sessions.md).
- **Session browsing** — `/sessions` to list, `/load <id-prefix>` to resume.
- **Sidebar + file viewer** — browse your workspace tree and open files inline.
- **Mouse support** — drag to select and copy (OSC 52), wheel to scroll, click to toggle tool blocks.
- **Catppuccin Mocha theme** — coherent palette across all panels.
- **Telegram connect** — pair your own Telegram bot to chat from your phone. See [docs/telegram.md](docs/telegram.md).
- **Token tracking** — running prompt + completion count in the header.
- **Specialist agents** — configure up to 5 named sub-agents on different models. See [docs/multiagents.md](docs/multiagents.md).

---

## Updating

```bash
which hmanlab
```

| If `which` points to… | Installed via… | Update with |
|---|---|---|
| `~/.local/bin/hmanlab` | curl | re-run the curl command above |
| `<npm prefix>/bin/hmanlab` | npm | `npm install -g hmanlab@latest` |
| `~/.cargo/bin/hmanlab` | cargo | `cargo install hmanlab --force` |

From inside the TUI: `/update` — detects the install channel and runs the right command.

---

## Configuration

<details>
<summary><b>CLI flags & environment variables</b></summary>

| Flag | Default | Env |
|---|---|---|
| `--host` | `http://localhost:11434` | `OLLAMA_HOST` |
| `--model` | first available | `OLLAMA_MODEL` |
| `--workspace` | current directory | — |

Examples:

```bash
# Local Ollama
hibiscus --host http://192.168.3.3:11434 --model qwen3:8b

# No flags needed if Ollama is on localhost and a provider key is already saved
hmanlab
```

</details>

<details>
<summary><b>Slash commands</b></summary>

| Command | Action |
|---|---|
| `/help`, `/?` | Show inline help |
| `/new`, `/n` | Start a fresh session (`Ctrl+N`) |
| `/clear` | Clear visible chat |
| `/sessions`, `/hist` | List local saved sessions |
| `/load <id-prefix>` | Resume a saved session |
| `/model` | Open model picker (`Ctrl+M`) |
| `/model <name>` | Switch model (partial match works) |
| `/host <url>` | Change Ollama host |
| `/workspace <path>` | Change agent workspace |
| `/trust` | Authorise this workspace for file edits & shell |
| `/untrust` | Remove this workspace from the trusted list |
| `/compact` | Manually compact conversation history |
| `/disconnect` | Remove a BYOK provider and its models |
| `/settings` | Show version, model, and configured providers |
| `/telegram setup [token]` | Set up the Telegram bot |
| `/telegram pair [code]` | Redeem a pairing code |
| `/telegram status` | Show bot status |
| `/telegram unpair` | Clear all paired users |
| `/telegram off` | Stop the bot |
| `/telegram notify [on\|off]` | Toggle idle notifications |
| `/agents [sub]` | Manage specialist agents |
| `/ask <name> <query>` | Invoke a specialist manually |
| `/update` | Update to the latest release |
| `/quit`, `/exit` | Quit (`Ctrl+Q` or `Ctrl+C` when idle) |

</details>

<details>
<summary><b>Key bindings</b></summary>

| Key | Action |
|---|---|
| `Enter` | Send message |
| `Alt+Enter` / `Ctrl+J` | Newline in input |
| `Ctrl+N` | New session |
| `Ctrl+M` | Open model picker |
| `Ctrl+T` | Fold/unfold all tool and thinking blocks |
| `Ctrl+C` | Cancel generation (or quit when idle) |
| `Ctrl+Q` | Quit |
| `Ctrl+L` | Clear chat history |
| `Esc` | Interrupt generation, clear draft, or close popup |
| `/` at start of input | Slash-command autocomplete |
| `@` after whitespace | File/folder autocomplete |
| `↑` / `↓` | Navigate autocomplete popup |
| `Tab` / `Enter` in popup | Insert completion |
| `Mouse wheel` | Scroll chat |
| `Drag` | Select text; release copies to clipboard |
| `Click` on tool/thinking block | Toggle fold |
| `PgUp/PgDn`, `Home/End` | Scroll |
| `Y` / `N` | Quick-reply to a yes/no question |

</details>

---

## Providers

Add a BYOK provider with `Ctrl+M` → pick a `+ Add` entry.

| Provider | Endpoint | Key source |
|---|---|---|
| **Ollama** (local or LAN) | `--host` or `/host <url>` | none (local) |
| **Ollama Cloud** | `https://ollama.com` | [ollama.com/settings/keys](https://ollama.com/settings/keys) |
| **z.ai subscription** | `https://api.z.ai/api/coding/paas/v4` | z.ai dashboard |
| **z.ai usage-based** | `https://api.z.ai/api/paas/v4` | z.ai dashboard |
| **OpenCode Go** | `https://opencode.ai/zen/go/v1` | opencode.ai |
| **OpenRouter** | `https://openrouter.ai/api/v1` | [openrouter.ai/settings/keys](https://openrouter.ai/settings/keys) |
| **Hibiscus** | `https://ai.hmanlab.pro/v1` | [ai.hmanlab.pro](https://ai.hmanlab.pro) |
| **MiniMax** | `https://api.minimax.io/v1` | MiniMax dashboard |

Keys are stored in `~/.config/hmanlab/config.json` (mode `0600`) and sent **only** to the matching provider.

---

## Architecture

```
Hibiscus (Rust TUI binary)
   │
   ├── Ollama API          local / LAN / Ollama Cloud
   ├── OpenAI-compat       z.ai · OpenCode Go · OpenRouter · hmanlab
   ├── Telegram bot        api.telegram.org long-poll
   ├── Memory store        ~/.hmanlab/memory/ + <workspace>/.hmanlab/memory/
   └── Local sessions      ~/.config/hmanlab/sessions/<cwd-hash>/<uuid>.jsonl
```

No required backend. Sessions are append-only JSONL files on disk — one file per session, one JSON record per turn.

<details>
<summary><b>Source layout</b></summary>

| File | Purpose |
|---|---|
| `src/main.rs` | CLI parsing, terminal setup, event loop |
| `src/agent.rs` | Agent loop — streams from LLM, dispatches tool calls |
| `src/session.rs` | Local JSONL session persistence (write, list, load) |
| `src/app/mod.rs` | `App` struct, constructor, shared helpers |
| `src/app/event.rs` | Keyboard/mouse handling, slash-command dispatch |
| `src/app/backend.rs` | `LlmBackend` enum, provider routing, BYOK key management |
| `src/app/stream/` | `StreamMsg` handlers — chunks, tools, sessions, compaction |
| `src/app/commands/` | Slash-command implementations |
| `src/ui/` | Ratatui renderers — chat, sidebar, popups, file viewer |
| `src/ollama.rs` | Ollama `/api/chat` streaming client |
| `src/openai_compat.rs` | OpenAI-compatible SSE streaming client |
| `src/compact.rs` | Conversation compaction (manual + auto) |
| `src/config.rs` | Config file I/O, first-run wizard, provider constants |
| `src/memory.rs` | Memory store I/O, MEMORY.md index |
| `src/api.rs` | `Session` type (picker) |
| `src/telegram.rs` | Telegram bot — long-poll, pairing, message bridge |
| `src/tools/` | Tool definitions, dispatch, confirmation, diff preview |

</details>

---

## Local development

### Prerequisites

- [Rust](https://rustup.rs) stable (1.75+)
- macOS or Linux (Windows via WSL)

### Build & run

```bash
git clone https://github.com/hmanlab/hmanlab
cd hmanlab

# Debug build — fast compile, slower runtime
cargo run

# Optimised build — same as the released binary
cargo build --release
./target/release/hmanlab

# With a specific Ollama host
cargo run -- --host http://localhost:11434

# Install the release build to PATH
cp target/release/hmanlab ~/.local/bin/hmanlab
```

### Workflow

```bash
# Check everything compiles with no errors
cargo build

# Run tests
cargo test

# Check for lint issues
cargo clippy

# Watch + rebuild on save (requires cargo-watch)
cargo watch -x run
```

### Config & data locations

| Path | What lives there |
|---|---|
| `~/.config/hmanlab/config.json` | Provider keys, Ollama host, last model, trusted workspaces |
| `~/.config/hmanlab/sessions/<hash>/` | JSONL session files for each working directory |
| `~/.hmanlab/memory/` | User-scope persistent memory |
| `<workspace>/.hmanlab/memory/` | Project-scope persistent memory |

To start completely fresh (wipe all local state):

```bash
rm -rf ~/.config/hmanlab ~/.hmanlab
```

### Adding a provider

1. Add constants to `src/config.rs` (`*_PROVIDER`, `*_BASE`, `*_MODELS`, `*_DEFAULT_MODEL`)
2. Add the provider to `BYOK_PROVIDERS`
3. Add a `provider_label` arm
4. Add a config field (`*_api_key`) to `Config` + `hmanlab_api_key` fan-out in `persist_config` (`src/app/backend.rs`)
5. Add a `current_host` arm and routing arm in `make_backend_for` (`src/app/backend.rs`)
6. Add an `ensure_*_models` function and call it from `ensure_byok_models_pub`
7. Load the key in `main.rs` with `app.set_byok_key(...)`

---

## Detailed docs

- **[docs/sessions.md](docs/sessions.md)** — local JSONL session format, file layout, and how `/sessions` + `/load` work.
- **[docs/multiagents.md](docs/multiagents.md)** — specialist agents: configure up to 5 named experts on different models.
- **[docs/telegram.md](docs/telegram.md)** — Telegram bot: pair your own bot, inline approve/deny buttons, idle notifications.

---

## Security

hmanlab is local-first. Your secrets stay on your machine:

- **Provider keys** live in `~/.config/hmanlab/config.json` (`0600`), sent only to the matching provider.
- **Sessions** are plain JSONL files on your disk — not uploaded anywhere.
- **Destructive tools** (`edit_file`, `write_file`, `run_command`, `save_memory`, `forget_memory`) require confirmation with a diff preview before running.
- **Bugs and vulnerabilities** — see [SECURITY.md](SECURITY.md) before opening a public issue.

---

## Contributing

Bug reports, features, and PRs welcome — see [CONTRIBUTING.md](CONTRIBUTING.md). By participating you agree to the [Code of Conduct](CODE_OF_CONDUCT.md). For security issues, follow [SECURITY.md](SECURITY.md).

## License

MIT — see [LICENSE](LICENSE).
