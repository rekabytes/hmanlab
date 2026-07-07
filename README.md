<h1 align="center">hmanlab</h1>

<p align="center">
  <b>The agentic terminal client for any LLM you have a key for.</b><br>
  Local Ollama · Cloud Ollama · z.ai · OpenCode Go · Hibiscus · OpenRouter · No account required
</p>

<p align="center">
  <a href="https://www.npmjs.com/package/hmanlab"><img alt="npm" src="https://img.shields.io/npm/v/hmanlab?label=npm&color=cb3837"></a>
  <a href="https://github.com/hmanlab/hmanlab/actions/workflows/ci.yml"><img alt="ci" src="https://img.shields.io/github/actions/workflow/status/hmanlab/hmanlab/ci.yml?label=ci"></a>
  <a href="LICENSE"><img alt="license" src="https://img.shields.io/badge/license-MIT-blue"></a>
  <a href="https://github.com/hmanlab/hmanlab/releases"><img alt="downloads" src="https://img.shields.io/github/downloads/hmanlab/hmanlab/total?label=downloads&color=green"></a>
  <img alt="status" src="https://img.shields.io/badge/status-alpha-orange">
</p>

---

## Repository layout

This is a **monorepo** holding two clients that share the same agent
contract. Pick the one that matches what you want to do:

| Path | Lang | Status | What it is |
|---|---|---|---|
| [`cli/`](cli/) | Rust + [ratatui](https://ratatui.rs) | **Production** | The current shipping client. ~23k LOC, polished, full feature set. |
| [`tui/`](tui/) | Go + [Bubble Tea](https://github.com/charmbracelet/bubbletea) | Scaffold | The next-gen client. Building now. `cli/` is the reference implementation. |

Everything end-users interact with today (the npm package, the curl
installer, the GitHub Release binaries, the docs, the changelog) lives
under [`cli/`](cli/). When `tui/` reaches feature parity, `cli/` will
be archived.

## Install (the shipping client)

| Method | Command | Binary location |
|---|---|---|
| **Curl** | `curl -fsSL https://github.com/hmanlab/hmanlab/releases/latest/download/install.sh \| sh` | `~/.local/bin/hmanlab` |
| **npm (global)** | `npm i -g hmanlab` | `$(npm root -g)/../bin/hmanlab` |
| **npm (one-off)** | `npx hmanlab` | (no install) |
| **From source** | `cargo install --git https://github.com/hmanlab/hmanlab` (run from [`cli/`](cli/)) | `~/.cargo/bin/hmanlab` |

Prebuilt binaries cover `linux-x64`, `linux-arm64`, `darwin-x64`,
`darwin-arm64`, and `win32-x64`. Windows users: use the npm path — the
curl installer is POSIX-only.

> **Pick one install method per machine and stick with it.** Each
> channel drops the binary in its own directory; mixing channels is the
> most common reason "updates don't take effect."

## Building from source

The Rust client builds from [`cli/`](cli/):

```sh
cd cli
cargo build --release
# Binary: cli/target/release/hmanlab
```

The Go TUI is not buildable yet — see [`tui/README.md`](tui/README.md)
for the plan and current status.

## Documentation

- [**`cli/README.md`**](cli/README.md) — full feature docs, keybindings,
  config file format, troubleshooting. This is the user-facing README
  for the shipping client.
- [**`cli/CHANGELOG.md`**](cli/CHANGELOG.md) — release history.
- [**`cli/docs/`**](cli/docs/) — architecture notes and design docs.
- [**`tui/README.md`**](tui/README.md) — Go client plan + reference
  contract the two clients must stay interoperable against.

## Contributing

For changes to the shipping Rust client, see
[`cli/CONTRIBUTING.md`](cli/CONTRIBUTING.md). For work on the Go TUI,
the rules are simpler: read the Rust source in `cli/` first when in
doubt about behaviour — it's the source of truth until the Go client
reaches parity.

## License

MIT — see [LICENSE](LICENSE).
