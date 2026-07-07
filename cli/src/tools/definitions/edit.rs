//! Mutating-file tool schemas: `edit_file`, `multi_edit`, `apply_patch`,
//! `move_lines`, `delete_lines`, `insert_at`, `write_file`.
//!
//! Every tool here requires user approval. Descriptions tell the model
//! *which* primitive to reach for in *which* situation — picking the
//! right one matters a lot, since the wrong primitive leads to the
//! "panic-rewrite via write_file" failure pattern we built this whole
//! family to retire:
//!
//!   - **edit_file** — single surgical string swap.
//!   - **multi_edit** — N batched swaps to the same file.
//!   - **apply_patch** — multi-region content changes (V4A envelope).
//!   - **move_lines** / **delete_lines** / **insert_at** — coordinate
//!     primitives that don't require reproducing file content (best
//!     for section reorders and structural cleanups).
//!   - **write_file** — last-resort, only for genuinely new files
//!     or wholesale rewrites.

use serde_json::json;

use crate::ollama::Tool;

pub(super) fn edit_tools() -> Vec<Tool> {
    vec![
        Tool::function(
            "edit_file",
            "Replace an exact string in a file. By default `old_string` must appear EXACTLY \
             once — if it's missing or ambiguous, the call fails. On ambiguity the right move \
             is to expand `old_string` with surrounding context until it's unique, NOT to fall \
             back to write_file (that loses surgical-diff granularity). For genuine bulk \
             renames (changing every occurrence of a variable name, fixing a typo throughout) \
             pass `replace_all: true`. Don't include the `<n>\\t` line-number prefixes from \
             read_file output in `old_string` — strip them so the snippet matches the raw file \
             content. User must approve before it writes.",
            json!({
                "type": "object",
                "properties": {
                    "path":        {"type": "string", "description": "Path (relative to workspace)"},
                    "old_string":  {"type": "string", "description": "Exact text to find (must be unique in the file unless replace_all is true)"},
                    "new_string":  {"type": "string", "description": "Replacement text"},
                    "replace_all": {"type": "boolean", "description": "If true, replace every occurrence of old_string instead of requiring a unique match. Default false. Use for bulk renames; leave off for surgical edits."}
                },
                "required": ["path", "old_string", "new_string"]
            }),
        ),
        Tool::function(
            "multi_edit",
            "Apply MULTIPLE surgical edits to the same file in one call. Takes an `edits` \
             array of {old_string, new_string} pairs applied in order — each `old_string` must \
             appear EXACTLY once in the current file state (after prior edits in this batch), \
             unless that edit sets `replace_all: true`. All-or-nothing: if any edit fails \
             validation, nothing is written and the error names the failing edit index. On \
             ambiguity, expand the offending snippet with surrounding context — don't fall \
             back to write_file. Prefer multi_edit over multiple edit_file calls when changing \
             several places in the same file — one approval, one cumulative diff, one tool \
             call. Don't include `<n>\\t` line-number prefixes from read_file output in any \
             `old_string`. User must approve before it writes.",
            json!({
                "type": "object",
                "properties": {
                    "path":  {"type": "string", "description": "Path (relative to workspace)"},
                    "edits": {
                        "type": "array",
                        "description": "Sequence of edits to apply. Order matters — each old_string is matched against the state AFTER prior edits.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "old_string":  {"type": "string", "description": "Exact text to find (must be unique in the current state unless replace_all is true)"},
                                "new_string":  {"type": "string", "description": "Replacement text"},
                                "replace_all": {"type": "boolean", "description": "If true, replace every occurrence of this edit's old_string in the current state. Default false. Per-edit so a single batch can mix surgical edits with rename-style edits."}
                            },
                            "required": ["old_string", "new_string"]
                        }
                    }
                },
                "required": ["path", "edits"]
            }),
        ),
        Tool::function(
            "apply_patch",
            "Apply a V4A-format patch envelope to one or more files in a single atomic call. \
             Use this — NOT write_file — when you need to rearrange content (swap sections, \
             move blocks, reorder a markdown file's headings), apply multi-region changes, \
             or perform changes that would otherwise need many edit_file calls strung \
             together. write_file should be reserved for genuinely new files or true \
             full-file rewrites; reach for apply_patch on rearrangements first.\n\n\
             Format (literal text, line-by-line):\n\
             ```\n\
             *** Begin Patch\n\
             *** Update File: path/to/file\n\
             @@ optional anchor context\n\
              context line (leading space)\n\
             -line to remove\n\
             +line to add\n\
              another context line\n\
             *** Add File: path/to/newfile\n\
             +line of new content\n\
             +another line\n\
             *** Delete File: path/to/old\n\
             *** End Patch\n\
             ```\n\n\
             Rules: every line in an Update hunk must start with '@@', '+', '-', or ' ' \
             (space prefix for context). Add File lines all start with '+'. The text in \
             each hunk (context+remove lines, in order) must locate uniquely in the \
             current file state; if a hunk is ambiguous, add more surrounding context \
             lines around the change. All operations are validated in memory before any \
             write touches disk; if any hunk fails, nothing is written and you get an \
             error pointing at the failing hunk. Don't include `<n>\\t` line-number \
             prefixes from read_file output. User must approve before it writes.",
            json!({
                "type": "object",
                "properties": {
                    "patch": {
                        "type": "string",
                        "description": "Full V4A patch envelope as a single string, starting with '*** Begin Patch' and ending with '*** End Patch'."
                    }
                },
                "required": ["patch"]
            }),
        ),
        Tool::function(
            "move_lines",
            "Move a range of lines (`from_start`..=`from_end`) to a new position \
             (before `to_before`) in the same file. PREFERRED primitive for section \
             reorders, swapping headings, moving blocks of code — anything where the \
             content is staying intact but its position is changing. Use this INSTEAD \
             of apply_patch or write_file for moves: you don't have to reproduce any \
             of the file content as an argument, which means the operation can't fail \
             from whitespace drift, dropped list markers, or token-budget truncation. \
             All line numbers are 1-indexed and inclusive, matching the `<n>\\t` \
             prefixes from read_file output. Use `to_before = <line_count> + 1` to \
             move a block to the very end of the file. User must approve before it \
             writes.\n\n\
             Workflow: read_file to see the line numbers → call move_lines with the \
             coordinates → done. No content emit, no reproduction risk.",
            json!({
                "type": "object",
                "properties": {
                    "path":       {"type": "string", "description": "Path (relative to workspace)"},
                    "from_start": {"type": "integer", "minimum": 1, "description": "1-indexed first line of the block to move (inclusive)"},
                    "from_end":   {"type": "integer", "minimum": 1, "description": "1-indexed last line of the block to move (inclusive)"},
                    "to_before":  {"type": "integer", "minimum": 1, "description": "1-indexed line BEFORE which the block lands. Pass line_count+1 to append at end."}
                },
                "required": ["path", "from_start", "from_end", "to_before"]
            }),
        ),
        Tool::function(
            "delete_lines",
            "Remove a contiguous range of lines (`start`..=`end`) from a file. Use for \
             structural deletions where the block is identified by position rather than \
             text — e.g. 'drop the deprecated section on lines 80-95'. For content-based \
             deletions ('drop every line containing FOO') prefer edit_file with \
             replace_all. 1-indexed, inclusive. User must approve before it writes.",
            json!({
                "type": "object",
                "properties": {
                    "path":  {"type": "string", "description": "Path (relative to workspace)"},
                    "start": {"type": "integer", "minimum": 1, "description": "1-indexed first line to remove (inclusive)"},
                    "end":   {"type": "integer", "minimum": 1, "description": "1-indexed last line to remove (inclusive)"}
                },
                "required": ["path", "start", "end"]
            }),
        ),
        Tool::function(
            "insert_at",
            "Insert new content into a file BEFORE a given line. Unlike edit_file / \
             apply_patch, you only emit the NEW content — you never have to reproduce \
             existing file content as an argument, so this can't fail on reproduction \
             drift. Use for inserting a new section, adding a function, prepending a \
             header, etc. 1-indexed. Pass `before_line = <line_count> + 1` to append \
             at end of file. The `content` string is split on '\\n'; a single trailing \
             newline on `content` is normalised away (so 'x\\ny' and 'x\\ny\\n' insert \
             the same two lines). User must approve before it writes.",
            json!({
                "type": "object",
                "properties": {
                    "path":        {"type": "string", "description": "Path (relative to workspace)"},
                    "before_line": {"type": "integer", "minimum": 1, "description": "1-indexed line BEFORE which the new content lands. Pass line_count+1 to append at end."},
                    "content":     {"type": "string", "description": "New content to insert. Newlines separate lines. Don't include `<n>\\t` line-number prefixes from read_file output."}
                },
                "required": ["path", "before_line", "content"]
            }),
        ),
        Tool::function(
            "write_file",
            "Write contents to a file, creating or overwriting it. Use ONLY for \
             genuinely new files, or when the entire file content is being replaced. \
             For section reorders or block moves, prefer move_lines (no content \
             reproduction required). For multi-region content changes, prefer \
             apply_patch. write_file clobbers the rest of the file and discards \
             surgical-diff granularity — it's the last-resort primitive, not the \
             first. User must approve before it writes.",
            json!({
                "type": "object",
                "properties": {
                    "path":    {"type": "string", "description": "Path (relative to workspace)"},
                    "content": {"type": "string", "description": "Full file contents to write"}
                },
                "required": ["path", "content"]
            }),
        ),
    ]
}
