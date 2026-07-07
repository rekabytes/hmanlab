# hmanlab

A terminal UI for chatting with local [Ollama](https://ollama.com) models — plus BYOK providers (z.ai, Ollama Cloud, OpenCode Go) — built in Rust with [ratatui](https://ratatui.rs).

```bash
# Global install
npm install -g hmanlab

# Run once without installing
npx hmanlab

# Per-project (e.g. add to a repo's dev deps)
npm install --save-dev hmanlab
npx hmanlab
```

## Quick start

After installing, run:

```bash
hmanlab
```

On first launch, hmanlab asks for an API key. **Register a free account at
[hmanlab.senireka.my](https://hmanlab.senireka.my) → API keys**, paste the
`bai_…` key when prompted, and you're in. The TUI itself is free.

### Who provides the LLM?

**You do.** hmanlab is a frontend — it doesn't host or proxy any model.
Plug in either:

- **Your local [Ollama](https://ollama.com)** — runs entirely on your
  machine, no traffic leaves your network. Default and fully free.
- **Your own provider key** (BYOK) — z.ai, Ollama Cloud, or OpenCode Go.
  hmanlab stores the key locally (mode `0600`) and talks to the provider
  directly; the hmanlab backend never sees it.

### What does the hmanlab API key do?

It authenticates the TUI to the [hmanlab.senireka.my](https://hmanlab.senireka.my)
backend that stores your chat sessions so you can resume them later
(`/sessions`, `/load`, `/more`, cross-device history). Without a key the
TUI still runs against your local Ollama / BYOK provider — you just lose
saved-session sync.

## Features

- Streaming replies, agentic tool calls (`read_file`, `edit_file`, `run_command`, git, find), foldable `<think>` blocks
- BYOK providers: z.ai (subscription + usage), Ollama Cloud, OpenCode Go
- Session persistence via `hmanlab-api`, with `/sessions`, `/load`, `/more`
- Workspace sidebar with click-to-expand folders, scroll, click-to-open file viewer
- `/compact` slash command + automatic compaction at high context tokens; compactions are persisted to `<workspace>/.hmanlab/memory/compact-current.md` so the model can resume across sessions
- Memory store at `~/.hmanlab/memory/` (user-scope) and `<workspace>/.hmanlab/memory/` (project-scope), surfaced to the model every turn

## Supported platforms

Prebuilt binaries ship for:

- `linux-x64`, `linux-arm64` (musl, statically-linked)
- `darwin-x64`, `darwin-arm64`
- `win32-x64`

On other platforms, `npm install` will succeed but `hmanlab` will print a friendly "no prebuilt binary" message and exit. Build from source via `cargo install --git https://github.com/hmanlab/hmanlab`.

## Where does `.hmanlab/` live?

Wherever you launch `hmanlab` from — that becomes the **workspace**:

- Project install: `npx hmanlab` from a project dir → `<project>/.hmanlab/`
- Global install: `cd ~/myrepo && hmanlab` → `~/myrepo/.hmanlab/`. `cd ~ && hmanlab` → `~/.hmanlab/`

User-scope state (cross-project preferences, identity) always lives at `~/.hmanlab/`.

## License

MIT. See https://github.com/hmanlab/hmanlab for source.
