# Security policy

## Project status

hmanlab is a **single-maintainer project**, owned and reviewed by [@rekabytes](https://github.com/rekabytes). Every change — code, dependencies, CI, releases — passes through the same person. There is no second human reviewer, and we do not enforce a "requires N approving reviews" branch-protection rule on `main`, because doing so would mean no PR could ever merge.

What this means for you as a user or contributor:

- The maintainer self-reviews every change before merging. There is no separate code reviewer.
- Security-sensitive PRs (anything touching auth, key handling, the agent confirmation flow, the release pipeline, or supply-chain tooling) get extra scrutiny — diff read twice, test loop run locally, dependency tree compared against the previous release.
- OpenSSF Scorecard's `Code-Review` and `Maintained` checks will report low scores until either a second maintainer joins (Code-Review) or the repo passes its 90-day anniversary (Maintained). Both are dismissed in the repository's Security tab with reason *"single-maintainer project; no second reviewer available"* and *"time-based metric; resolves automatically"* respectively. Neither indicates an unpatched vulnerability — please read the actual `Vulnerabilities` check for that.

If you'd like to help close the Code-Review gap by becoming a reviewer, open a discussion or reach out to the maintainer.

## Supported versions

Only the latest minor release receives security fixes. If you're more than one minor behind, please upgrade before filing a report.

## Reporting a vulnerability

If you believe you've found a security issue in hmanlab, please **do not** open a public GitHub issue. Instead, either:

- Open a private advisory: <https://github.com/hmanlab/hmanlab/security/advisories/new>
- Or email <hello@rekabytes.com> with the subject line `hmanlab security`.

Please include:

- A description of the issue and its impact.
- Steps to reproduce (a minimal repro is ideal).
- The hmanlab version (`hmanlab --version`).
- Whether you've shared this report with anyone else.

You should expect an initial acknowledgement within a few days. Coordinated disclosure is appreciated — we'll work with you on a fix and a timeline before any public discussion.

## What hmanlab does with your secrets

hmanlab is a local-first TUI. Your secrets stay on your machine:

- **LLM provider API keys** (Ollama Cloud, z.ai, OpenCode Go) are stored in `~/.config/hmanlab/config.json` with file mode `0600` and are sent **only** to the matching provider endpoint. They are never sent to the hmanlab-api session backend.
- **hmanlab-api keys** (`HMANLAB_API_KEY`) authenticate session persistence against the hmanlab-api backend you configured with `--api-url`. The default is the maintainer's hosted backend at `https://be-ai.senireka.my`; you can point at your own.
- **Conversation history** is sent to whichever LLM provider you have selected so the model can respond — same as any chat client.

## Agentic tools — local-machine reach

Tool-capable models can call `read_file`, `list_dir`, `find_files`, `git_*`, `edit_file`, `write_file`, `run_command`, and the memory tools. The first three groups read only. Anything that mutates state (`edit_file`, `write_file`, `run_command`, `save_memory`, `forget_memory`) opens an in-TUI confirmation dialog showing the exact change before it runs. `run_command` additionally has a 30-second timeout.

If you find a way to bypass a confirmation prompt, treat that as a security issue and report it under the process above.
