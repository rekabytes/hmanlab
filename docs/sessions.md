# Local sessions

hmanlab persists every conversation to disk as an append-only JSONL file. No server, no account — sessions survive restarts and are browsable with `/sessions`.

## Storage layout

```
~/.config/hmanlab/sessions/
  <cwd-hash>/               # one directory per working directory (hash of the path)
    <session-uuid>.jsonl    # one file per session
    <session-uuid>.jsonl
    ...
```

The `<cwd-hash>` is a hex hash of the workspace path, so sessions are scoped to the directory you were in when you started chatting. Sessions from `/home/user/projectA` and `/home/user/projectB` never mix.

## File format

Each `.jsonl` file is a sequence of newline-delimited JSON records, one per event:

```jsonl
{"type":"meta","session_id":"...","uuid":"...","timestamp":"2026-05-31T14:00:00Z","model":"glm-5.1","title":"Fix the auth bug"}
{"type":"user","session_id":"...","uuid":"...","timestamp":"2026-05-31T14:00:01Z","content":"Why is the login failing?","model":"glm-5.1"}
{"type":"assistant","session_id":"...","uuid":"...","timestamp":"2026-05-31T14:00:05Z","content":"The token expiry is set to 0…","model":"glm-5.1"}
{"type":"tool","session_id":"...","uuid":"...","timestamp":"2026-05-31T14:00:06Z","name":"read_file","content":"<file contents>"}
```

### Record types

| `type` | When written | Key fields |
|---|---|---|
| `meta` | First message of the session | `title`, `model` |
| `user` | Every user message sent | `content`, `model` |
| `assistant` | End of each assistant turn | `content`, `model` |
| `tool` | After each tool result | `name`, `content` |

Hidden Y/N injections (internal follow-up prompts) are not written to disk.

## Session commands

| Command | What it does |
|---|---|
| `/sessions` | List all sessions for the current workspace, newest first. Opens a picker. |
| `/load <prefix>` | Load a session by the first few characters of its UUID. Replays all turns into the visible chat. |
| `/new` | Start a fresh session. The old file stays on disk. |
| `/clear` | Wipe visible history and start a new session (same as `/new` for persistence purposes). |

## Auto-start

A session file is created automatically on the first user message of each TUI launch. You don't need to do anything — just chat and hmanlab writes to disk in the background.

The session title is derived from your first message (truncated to 60 characters). It's written in the `meta` record and shown in the `/sessions` picker.

## Inspecting sessions manually

Sessions are plain text. You can read them directly:

```bash
# List all sessions for the current directory
ls -lt ~/.config/hmanlab/sessions/$(python3 -c "
import hashlib, struct
h = 0
for c in '$PWD':
    h = (h * 1540483477 + ord(c)) & 0xffffffffffffffff
print(f'{h:016x}')
")/

# Read a session
cat ~/.config/hmanlab/sessions/<hash>/<uuid>.jsonl | python3 -m json.tool
```

Or use `jq`:

```bash
cat ~/.config/hmanlab/sessions/<hash>/<uuid>.jsonl \
  | jq 'select(.type == "user" or .type == "assistant") | {type, content}'
```

## Clearing history

To delete all sessions for the current workspace:

```bash
rm -rf ~/.config/hmanlab/sessions/<cwd-hash>/
```

To delete everything:

```bash
rm -rf ~/.config/hmanlab/sessions/
```
