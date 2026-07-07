# Multi-agent specialists

Configure up to **5 named specialists**, each with their own model and a one-line *"use this when …"* description. The main agent delegates to them automatically via the `consult_specialist` tool, or you can route by hand with `/ask <name> <query>`. Specialists run with a **read-only tool surface** (file reads, git, memory recall) — writes, shell, and memory mutation stay with the main agent so cost stays bounded and loops can't form.

## Quick start

```text
/agents add                  # 5-step wizard: template → name → model → task → prompt
/agents on                   # enable specialist consultation for this session
"use agents to review src/agent.rs"   # main agent auto-delegates to a reviewer
/ask reviewer "what could go wrong in src/agent.rs?"   # manual invoke
/agents list                 # see the roster + per-agent token tally in the header
```

## Setup wizard

`/agents add` walks you through:

1. **Template** — pick `blank` to fill every field by hand, or one of 7 opinionated recipes that pre-fill name, task, and system prompt:
   - `code-reviewer` — second-pass review for bugs + style
   - `planner` — break tasks into steps, write PRDs
   - `file-explorer` — summarise files/directories
   - `researcher` — investigate "where is X used / how does Y work"
   - `triage` — diagnose bugs from traces, logs, behavior
   - `test-advisor` — list test cases (names + assertions) for a target
   - `doc-reviewer` — check docs against actual code
2. **Name** — short slug (3-30 chars; letters/digits/`_`/`-`). Used in `/ask <name>` and as the `consult_specialist` argument.
3. **Model** — pick from your live Ollama models + BYOK extras. Specialists can run on a different provider than the main agent.
4. **Task** — one-line "use this when…" description (≤ 200 chars). Shown in `/agents list` and fed into the consult tool description so the main agent knows when to delegate.
5. **System prompt** — full persona instructions. Multi-line (`Alt+Enter` / `Ctrl+J` for newline). Templates pre-fill an opinionated default; edit freely.

## Roster commands

| Command | Action |
|---|---|
| `/agents` | Show roster + session state + subcommand list |
| `/agents add` | Open the 5-step wizard for a new specialist |
| `/agents edit <name>` | Re-open the wizard pre-filled for an existing specialist |
| `/agents remove <name>` | Drop a specialist (`/agents rm` and `/agents del` also work) |
| `/agents list` | Pretty-print the current roster |
| `/agents on` / `/agents off` | Flip per-session activation (default: off on every launch) |
| `/agents enable-agent <name>` / `disable-agent <name>` | Park a specialist without removing it |
| `/ask <name> <query>` | Manually invoke a specialist (bypasses the main agent entirely) |

## How delegation works

When `/agents on` is active and at least one specialist is enabled, the main agent sees a `consult_specialist(name, query)` tool whose description includes each enabled specialist's task line. The model decides when to delegate based on those hints — e.g. it'll consult a `reviewer` for "review this code" prompts. Each consult shows in chat as a single tool row (collapsed by default; click to expand and see the query + specialist's reply). The header tally splits tokens per agent so consult costs stay legible.

## Important details

- **Per-session opt-in.** `/agents on` resets to off on every TUI restart — by design, so you don't surprise-bill yourself.
- **Roster persists.** The 5-slot roster lives in `~/.config/hmanlab/config.json` and survives restarts.
- **No chaining.** Specialists can't call other specialists (their tool surface excludes `consult_specialist`). One level deep, predictable cost.
- **Cancellation chains.** `Ctrl+C` during a consult aborts both the main agent and the specialist task.
- **`/ask` works without `/agents on`?** No — both paths gate on the session toggle so the opt-in stays meaningful.
