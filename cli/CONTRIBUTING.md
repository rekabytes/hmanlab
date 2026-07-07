# Contributing to hmanlab

Thanks for considering a contribution! hmanlab is a small Rust TUI; the contribution loop is intentionally light.

## Prerequisites

- Rust 1.74 or newer (`rustup toolchain install stable`).
- A reachable Ollama daemon for end-to-end smoke testing (`http://localhost:11434` is the default), or a BYOK key for one of the cloud providers.
- Node 18+ if you're touching the npm packaging under `npm/`.

## Build & run

```bash
cargo build --release
./target/release/hmanlab --help
```

For a fast inner loop:

```bash
cargo run --release -- --host http://localhost:11434
```

## Before you open a PR

Please run these locally — CI will run the same checks and reject the PR if any fail:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all
cargo build --release
```

If you need to silence a clippy lint with `#[allow(...)]`, add a one-line comment explaining why.

## Code style

- rustfmt defaults; no custom `rustfmt.toml`.
- Keep functions short — if a function in `src/app/` or `src/ui/` is creeping past ~100 lines, that's a hint to extract a helper.
- Comments explain **why**, not what. Don't restate what the code does.
- No `unwrap()` / `expect()` in code paths that could realistically fail at runtime — return `anyhow::Result` and surface the error to the UI via `StreamMsg::Error`.

## PR conventions

- Single-purpose PRs. Refactors, features, and bugfixes go in separate PRs.
- Conventional-commit-style subjects preferred: `feat: …`, `fix: …`, `chore: …`, `docs: …`, `refactor: …`. Not enforced by CI.
- If your change is user-visible, add a one-line entry under `## [Unreleased]` in `CHANGELOG.md`.
- Squash on merge so `main` stays linear.
- By participating in this project (issues, PRs, discussions) you agree to abide by the [Code of Conduct](CODE_OF_CONDUCT.md).

## Adding a new LLM provider

Provider routing lives in `src/app/backend.rs::LlmBackend` and `src/app/backend.rs::make_backend`. New providers should:

1. Add provider constants to `src/config.rs` (base URL, default models, key field name).
2. Wire the key field into the `Config` struct and the `/disconnect` picker.
3. Add a `+ Add <Provider> key` entry to the model picker in `src/app/event.rs`.
4. Route the new provider in `make_backend` to either `Backend::Ollama` (native Ollama protocol) or `Backend::OpenAi` (OpenAI-compatible `/chat/completions`).

If the wire format isn't one of those two, you'll also need a new client module — model it on `src/openai_compat.rs`.

## Local npm packaging loop

If you're working on the npm shim or release workflow:

```bash
scripts/npm-local.sh build    # cargo build → npm/@hmanlab/<plat>-<arch>/bin/
scripts/npm-local.sh pack     # produce tarballs under dist/
scripts/npm-local.sh install  # npm i -g the local tarballs
hmanlab --help                # should run via the JS shim
scripts/npm-local.sh clean
```

## Reporting bugs vs requesting features

- **Bug?** Open an issue using the **Bug report** template. Include the version, the platform, and the steps to reproduce.
- **Feature?** Open an issue using the **Feature request** template, or start a discussion if you want to sound out the idea first.
- **Security issue?** See [SECURITY.md](SECURITY.md) — please don't open a public issue for vulnerabilities.

## Releasing (maintainers only)

```bash
scripts/release.sh 0.X.Y
git push --follow-tags
```

The `release.yml` workflow builds binaries for all five targets and publishes to npm if `NPM_TOKEN` is set on the repo.
