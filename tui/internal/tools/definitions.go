package tools

import "github.com/hmanlab/hmanlab/tui/internal/llm"

// Definitions returns the JSON-Schema tool definitions for all 10
// core tools. Sent to the model so it knows what it can call.
// Mirrors cli/src/tools/definitions.rs::tool_definitions().
func Definitions() []llm.Tool {
	return []llm.Tool{
		readFileDef(),
		listDirDef(),
		findFilesDef(),
		editFileDef(),
		writeFileDef(),
		runCommandDef(),
		gitStatusDef(),
		gitLogDef(),
		gitDiffDef(),
		gitShowDef(),
	}
}

func fn(name, desc string, props map[string]any, required []string) llm.Tool {
	props["type"] = "object"
	return llm.Tool{
		Type: "function",
		Function: llm.ToolFunction{
			Name:        name,
			Description: desc,
			Parameters: map[string]any{
				"type":       "object",
				"properties": props,
				"required":   required,
			},
		},
	}
}

func strProp(desc string) map[string]any {
	return map[string]any{"type": "string", "description": desc}
}

func intProp(desc string) map[string]any {
	return map[string]any{"type": "integer", "description": desc}
}

func boolProp(desc string) map[string]any {
	return map[string]any{"type": "boolean", "description": desc}
}

// ── Read-only tools ──────────────────────────────────────────

func readFileDef() llm.Tool {
	return fn("read_file",
		"Read a text file from the workspace. Output is capped at ~50 KB. Each line is returned prefixed with `<line_no>\\t<content>` so you can reference exact line numbers.",
		map[string]any{
			"path": strProp("Path (relative to workspace, e.g. 'src/main.go')"),
		},
		[]string{"path"},
	)
}

func listDirDef() llm.Tool {
	return fn("list_dir",
		"List the contents of a directory. Directories are suffixed with '/', symlinks with '@'.",
		map[string]any{
			"path": strProp("Directory path (relative to workspace, defaults to '.')"),
		},
		nil,
	)
}

func findFilesDef() llm.Tool {
	return fn("find_files",
		"Find files matching a glob pattern. Automatically filters out node_modules, .git, target, build, dist. Caps at 100 matches.",
		map[string]any{
			"pattern": strProp("Glob pattern (e.g. '*.go', '**/*.rs')"),
		},
		[]string{"pattern"},
	)
}

// ── Mutating tools (require confirmation) ────────────────────

func editFileDef() llm.Tool {
	return fn("edit_file",
		"Edit a file by replacing old_string with new_string. Uses fuzzy matching — the first match is replaced unless replace_all is true. Requires confirmation before writing.",
		map[string]any{
			"path":        strProp("Path (relative to workspace)"),
			"old_string":  strProp("The text to find (must be unique unless replace_all)"),
			"new_string":  strProp("The replacement text"),
			"replace_all": boolProp("Replace all occurrences (default: false)"),
		},
		[]string{"path", "old_string", "new_string"},
	)
}

func writeFileDef() llm.Tool {
	return fn("write_file",
		"Write content to a file, creating it if it doesn't exist or overwriting if it does. Requires confirmation.",
		map[string]any{
			"path":    strProp("Path (relative to workspace)"),
			"content": strProp("The full file content to write"),
		},
		[]string{"path", "content"},
	)
}

func runCommandDef() llm.Tool {
	return fn("run_command",
		"Run a shell command via `sh -c`. Output (stdout + stderr) is captured and returned, capped at ~4 KB. Timeout is 600 seconds. Requires confirmation.",
		map[string]any{
			"command": strProp("The shell command to run"),
		},
		[]string{"command"},
	)
}

// ── Git tools (read-only) ───────────────────────────────────

func gitStatusDef() llm.Tool {
	return fn("git_status",
		"Show the working tree status (git status --porcelain=v1 -b).",
		map[string]any{},
		nil,
	)
}

func gitLogDef() llm.Tool {
	return fn("git_log",
		"Show recent commit history (git log --oneline -n N).",
		map[string]any{
			"limit": intProp("Number of commits (default: 10, max: 100)"),
		},
		nil,
	)
}

func gitDiffDef() llm.Tool {
	return fn("git_diff",
		"Show changes (git diff). Optionally specify a range or path.",
		map[string]any{
			"range": strProp("Git range (e.g. 'HEAD~3..HEAD')"),
			"path":  strProp("Limit diff to a specific file path"),
		},
		nil,
	)
}

func gitShowDef() llm.Tool {
	return fn("git_show",
		"Show the contents of a commit (git show <rev>).",
		map[string]any{
			"rev": strProp("Commit hash, tag, or ref (e.g. 'HEAD', 'abc123')"),
		},
		[]string{"rev"},
	)
}
