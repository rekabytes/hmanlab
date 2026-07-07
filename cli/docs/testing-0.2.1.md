# hmanlab 0.2.1 — manual test guide

Walks through everything that landed this session so you can verify it works end-to-end in the real TUI. Each section has a **setup → action → expected** shape so you can sanity-check at a glance.

All checks assume `cargo build --release` and you're launching with `./target/release/hmanlab`. Pick a workspace with at least one markdown file + one Rust file for the editor tests.

---

## 1. Line-numbered `read_file` output

**Why it matters:** the model now sees `<n>\t<content>` for every line in a read. Lets it point at unique lines for downstream `edit_file` / `move_lines` calls.

**Test:** ask the model: *"read README.md"* (or any file).
**Expected:** the tool tile shows the file with `1<TAB>...`, `2<TAB>...`, etc. — line numbers, tab-separated, then the actual line.
**Red flags:** no line numbers visible; numbers inserted into the file itself (they should only appear in the model's view of the read result).

---

## 2. `edit_file` strict + `replace_all`

**Strict mode (default):**
1. Ask: *"in src/main.rs, change the string `hmanlab` to `HMANLAB` only on the line that says `name = ` "*  (or anything where the snippet is unique).
2. Approve the diff popup.
3. **Expected:** confirm popup shows `Edit file ... (+1L -1L)`, file changes in exactly one place.

**Ambiguity error:**
1. Ask: *"replace every `---` in README.md with `===` using edit_file"* (don't say replace_all yet).
2. **Expected:** the tool returns an error like *"appears N times in README.md. Either expand the snippet... or set `replace_all: true`"*. The error message MUST mention `replace_all` — that's the recovery path the model needs.

**replace_all:**
1. Ask the same thing again with explicit instructions to use `replace_all: true`.
2. **Expected:** confirm popup shows `Edit file ... +NL -NL · N replacements`, all `---` become `===` in one approval.

---

## 3. `multi_edit` with mixed strict + per-edit `replace_all`

**Test:** ask: *"in src/tools/mod.rs, change `read_file` to `read_text_file` everywhere it appears, AND change the comment on line 4 to say something else"*.

**Expected:** model emits a single `multi_edit` with two edits:
- One with `replace_all: true` for the bulk rename
- One strict (default) for the comment change

Diff popup shows both as one cumulative diff with line counts. Approve once, both apply.

**Red flag:** model fires two separate `edit_file` calls — that's a regression of the multi_edit-routing guidance in the system prompt.

---

## 4. Fuzzy-match cascade

The matcher cascade catches `edit_file` / `multi_edit` snippets where the model's `old_string` is slightly off — trailing whitespace drift or middle-line drift. You can't easily force the model to produce a drifty snippet, but you can verify it doesn't false-positive:

1. Ask: *"change the string `qwen3:8b` to `qwen3:14b` somewhere in src/config.rs"*.
2. **Expected:** the edit lands on the exact one place; no warning about ambiguity if there's only one match.

The cascade only kicks in when the exact strategy fails. If you want to see it in action: the unit tests in `src/tools/matchers.rs` cover this — `cargo test --release matchers` shows the line-trimmed and block-anchor cases passing.

---

## 5. `apply_patch` (V4A envelope)

**Update file:**
1. Pick a small markdown file with a `## Section` heading.
2. Ask: *"use apply_patch to rename `## Section` to `## Renamed`"*.
3. **Expected:** confirm popup shows `Apply patch (~path.md) +1L -1L`. Approve; diff applies.

**Add file:**
1. Ask: *"use apply_patch to create a new file `scratch.txt` containing the line `hello world`"*.
2. **Expected:** confirm popup shows `+scratch.txt (added)` with `+hello world`. After approve, the file exists.

**Delete file:**
1. Ask: *"use apply_patch to delete `scratch.txt`"*.
2. **Expected:** confirm popup shows `-scratch.txt (deleted)`. After approve, the file is gone.

**Ambiguous hunk:**
1. Pick a file with a repeated phrase (e.g. README has multiple `---` separators).
2. Ask the model to emit an apply_patch with a hunk whose context+remove lines only contain `-` and ` `.
3. **Expected:** tool returns *"hunk #0 in ... matched N locations. Add more context lines..."*. The error must mention adding context.

**Add File when target exists:**
1. Ask the model to emit apply_patch with `*** Add File: README.md`.
2. **Expected:** *"Add File 'README.md' rejected — target already exists. Use Update File instead."*

---

## 6. Coordinate ops — `move_lines`, `delete_lines`, `insert_at`

**This is the headline fix. The README-rearrange case you couldn't get done before.**

**move_lines (the README test):**
1. Open the TUI in this repo.
2. Ask: *"in README.md, move the Install section above the Features section"*.
3. **Expected:** model reads README.md (sees line numbers), then emits **ONE** `move_lines` call with `from_start` / `from_end` / `to_before` as integers. Confirm popup shows the actual diff. Approve, README is now reordered.
4. **Red flag (the old failure):** model emits `apply_patch` with a giant patch text and gets truncated, or panic-rewrites with `write_file`. Either means the system prompt isn't steering toward `move_lines`.

**delete_lines:**
1. Ask: *"delete lines 5-7 of README.md"*.
2. **Expected:** confirm popup, three lines disappear.

**insert_at:**
1. Ask: *"insert a new paragraph before line 10 of README.md saying 'hello from the test guide'"*.
2. **Expected:** confirm popup shows the new line, approve, content lands at line 10.

**Append at end:**
1. Ask: *"append a new line `## Test footer` to the end of README.md"*.
2. **Expected:** model uses `insert_at` with `before_line` = `line_count + 1`.

**Out-of-bounds error:**
1. Ask: *"move lines 9000-9001 of README.md to the top"* (assuming the file isn't 9000+ lines).
2. **Expected:** *"from_end (9001) is past the end of README.md (file has N lines)."*

---

## 7. UI — split-pane popups

Every modal popup now occupies the **bottom half** of the chat column while open (instead of floating centered). Chat shows in the top half so you can still see context.

| Popup | How to open | What to verify |
|---|---|---|
| Model picker | `Ctrl+M` or `/model` | List in bottom half; chat visible above; `Esc` closes |
| Sessions | `/sessions` | Session list in bottom; chat above |
| Disconnect | `/disconnect` | BYOK list in bottom |
| Add model | Pick "+ Add ... key" from model picker | Paste-key field in bottom |
| Confirm dialog | Trigger any edit_file / write_file / run_command and watch the y/n popup | Diff preview in bottom; chat above; y/n keys still work |
| Telegram setup | `/telegram setup` | Wizard in bottom |
| Agents setup | `/agents add` | 5-step wizard in bottom |
| Shell monitor | Click the footer indicator while a shell is running (see §8) | Live output in bottom |

**Red flags:** popup floats over the chat instead of taking the bottom half; chat is hidden completely.

---

## 8. UI — shell monitor

**Setup:** ask the model to run a shell command that produces visible output, e.g. *"run `ls -la /usr/bin | head -20`"* or *"run `sleep 5 && echo done`"*.

**While running:**
- Footer status bar shows `● 1 shell running` with a pulsing peach dot to the right of the regular status text.
- The footer indicator is clickable.

**Click it:**
- The shell monitor overlay opens in the bottom half of the chat column.
- Stdout/stderr stream live; stderr in red.
- A `▌` cursor blinks at the tail while the shell is running.
- Footer hint inside the monitor reads `Esc hide · Ctrl+C kill shell · PgUp/PgDn scroll`.

**Press `Esc`:**
- Monitor closes, returns to chat.
- Footer indicator still shows (shell still running).
- Click it again — output picks up where it was.

**Press `Ctrl+C` while monitor is open:**
- Shell process is killed.
- Badge in monitor title flips to `✗ killed`.
- Footer indicator disappears.
- The final tool-result tile in chat shows `[killed by user]`.

**After shell exits normally:**
- Footer indicator disappears.
- Tool tile shows `[exit 0]` (or whatever exit code).
- Monitor (if still open) shows `✓ exit 0` badge until you `Esc`.

---

## 9. UI — streaming caret on assistant replies

**Test:** ask any question that triggers a streaming reply.
**Expected:** while the assistant is generating, a `▌` cursor blinks at the very end of the in-flight message text. Disappears the instant the turn ends. Does **not** appear on historical (completed) messages.

**Red flag:** caret stays on after the turn; caret on past messages; caret renders inside the gutter bar.

---

## 10. UI — memory tile compaction

**Test:** ask the model to save a memory: *"save a memory called test-memo (project scope, type=project, description=test) with a body of 'this is a test'"*.

**Expected:** the chat tile reads `memory · save test-memo` instead of dumping the full JSON args. No separate `✓ Allowed: SAVE memory ...` system line follows.

---

## 11. Refactor regression checks (this session's internal cleanup)

These shouldn't change behavior, but worth a spot-check:

- **`read_text_file` helper:** trigger an `edit_file` on a binary file (e.g. an image). Expected error: `"edit_file: 'path/to/image.png' is not valid UTF-8"`. Now goes through the shared helper, but the user-facing wording is unchanged.
- **`definitions.rs` split:** the regression test `full_tool_surface_includes_new_primitives` (in `src/tools/definitions.rs::tests`) asserts every tool name still registers. Run `cargo test --release definitions` to verify it passes.

---

## Quick sanity script

```bash
# Build + verify the four CI gates locally
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --release
cargo build --release

# Smoke-check the binary
./target/release/hmanlab --version    # expect: hmanlab 0.2.1
./target/release/hmanlab --help       # expect: clean Usage block
```

If those all pass, the build is healthy. Then run through sections 1-10 in the TUI for behavioral verification.

---

## What to do if something's off

1. **Note the section number** that failed.
2. **Copy the exact tool call** from chat (open the tool tile to see the args).
3. **Copy the exact error message** if the tool failed.
4. **Paste them back** and ask: *"this is what happened on section N of the test guide"*. The combination of {action, args, error} is enough to triage.
