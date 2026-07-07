# Telegram bot

Pair your own Telegram bot to chat with hmanlab from your phone. DMs from paired users become user turns; the assistant's reply DMs back. Destructive tools get inline approval buttons.

## Setup (one-time)

1. **Create a bot** via [@BotFather](https://t.me/BotFather) → `/newbot` → pick a name + handle. BotFather replies with a token (looks like `123456:ABC-DEF…`).
2. **Setup in hmanlab.** Run `/telegram setup` (opens a wizard) or paste the token directly: `/telegram setup 123456:ABC-DEF…`. hmanlab validates the token with `getMe` and starts the long-poll loop.
3. **Pair your Telegram account.** DM your bot any text from your phone. The bot replies with a 6-character pairing code (e.g. `K7M3Q9` — `0`/`O`/`1`/`I`/`L` excluded to avoid confusion). Codes expire after 10 minutes.
4. **Redeem the code** in the TUI: `/telegram pair K7M3Q9`. Your Telegram account is now on the allowlist; codes are one-shot.

## Using it

Once paired, DM your bot like you'd chat in the TUI:

```text
(you)  → write me a haiku about rust
(bot)  ← Three rules guard your code, /
         compiler smiles, types align, /
         segfaults flee at dawn.
```

Slash commands work from Telegram too — same aliases as the local terminal (`/m`, `/n`, `/ls`, etc.):

| Telegram command | What it does |
|---|---|
| `/help` | List the commands available over Telegram |
| `/models` | List available models (read-only over Telegram) |
| `/model <name>` | Switch the active model (BYOK keys must be configured locally) |
| `/new` | Start a fresh session |
| `/sessions` | List recent saved sessions |
| `/settings` | Account, version, configured providers |
| `/agents` | Show specialist roster + session state |
| `/agents on` / `/agents off` | Toggle specialist session |
| `/ask <name> <query>` | Manually invoke a specialist |

Anything not in the allowlist (`/quit`, `/host`, `/workspace`, `/agents add`, `/agents remove`, …) gets a "not available via Telegram" reply. Roster editing stays local-only because the wizard is the only sane way to write multi-line system prompts.

## Approving destructive tools

When the main agent wants to write a file, run a shell command, or save a memory, you'll get a Telegram DM with inline buttons:

```
write_file: src/agent.rs (+12 -3 lines)
[✅ Allow]  [🔏 Always]  [❌ Deny]
```

- **Allow** — runs once.
- **Always** — runs once AND adds the tool head (`write_file:`, `run_command:`, etc.) to a session-only allowlist so further matching prompts auto-approve without a DM. Resets on TUI restart.
- **Deny** — rejects; the tool returns an error the agent surfaces in chat.

If your phone doesn't render the buttons (rare), text fallback works: reply `y` / `yes` / `allow` or `n` / `no` / `deny`.

## Local commands

| Command | Action |
|---|---|
| `/telegram setup [token]` | Set / replace the bot token (opens wizard if no token given) |
| `/telegram pair [code]` | Redeem a pairing code from a Telegram DM |
| `/telegram status` | Bot status, paired users count, last event |
| `/telegram unpair` | Clear the allowlist (token + bot stay running) |
| `/telegram off` | Stop the bot, clear token + allowlist |
| `/telegram notify [on\|off]` | Toggle idle notifications — DM paired users when a long local turn finishes after the terminal goes idle |

## Important details

- **Allowlist gate.** Strangers DM'ing your bot get a pairing code only — they can't send chat turns until you redeem their code locally. Codes are 6 chars from an unambiguous alphabet, expire in 10 minutes, one-shot.
- **One reply route at a time.** If you're already mid-reply to one Telegram chat, a second chat's DM gets a "another Telegram chat is mid-conversation" rejection. Prevents reply mix-up.
- **Local cancel cuts the bridge.** `Ctrl+C` mid-stream tells the Telegram side too (with a "cancelled by the local user" note) so the DM thread doesn't sit silent.
- **Token storage.** The bot token lives in `~/.config/hmanlab/config.json` (mode `0600`) and is only sent to `api.telegram.org` — never to the hmanlab-api backend.
