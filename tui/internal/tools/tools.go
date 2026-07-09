package tools

import (
	"encoding/json"
	"fmt"
	"strings"
)

// Context carries the per-invocation state handed to each tool.
type Context struct {
	Workspace string
	// Confirm is called by mutating tools before executing. It blocks
	// until the user approves or denies. Returns true = allow.
	Confirm func(prompt string, diff []DiffLine) bool
}

// IsReadonly returns true for tools that don't modify the workspace.
// Used by the renderer to decide styling and by the agent loop to know
// which tools need confirmation.
func IsReadonly(name string) bool {
	switch ResolveAlias(name) {
	case "read_file", "list_dir", "find_files",
		"git_status", "git_log", "git_diff", "git_show":
		return true
	default:
		return false
	}
}

// ResolveAlias maps fine-tuned model outputs (TitleCase, short names)
// to the canonical snake_case tool names. Mirrors
// cli/src/tools/mod.rs:103-117.
func ResolveAlias(name string) string {
	switch strings.ToLower(name) {
	case "read", "readfile":
		return "read_file"
	case "ls", "list":
		return "list_dir"
	case "glob", "find":
		return "find_files"
	case "bash", "shell":
		return "run_command"
	case "edit":
		return "edit_file"
	case "write":
		return "write_file"
	case "gitstatus":
		return "git_status"
	case "gitlog":
		return "git_log"
	case "gitdiff":
		return "git_diff"
	case "gitshow":
		return "git_show"
	default:
		return name
	}
}

// ToolLabel returns a short human-readable label for a tool name,
// used in the grouped tile rendering. Mirrors cli/src/ui/chat/helpers.rs
// tool_label().
func ToolLabel(name string) string {
	switch ResolveAlias(name) {
	case "read_file":
		return "read"
	case "list_dir":
		return "list"
	case "find_files":
		return "find"
	case "edit_file":
		return "edit"
	case "write_file":
		return "write"
	case "run_command":
		return "shell"
	case "git_status":
		return "git status"
	case "git_log":
		return "git log"
	case "git_diff":
		return "git diff"
	case "git_show":
		return "git show"
	default:
		return name
	}
}

// GroupNoun returns the plural noun for a tool type, used in tiles
// like "3 reads" or "5 edits". Mirrors group_noun() in the CLI.
func GroupNoun(canonical string) string {
	switch canonical {
	case "read_file":
		return "files"
	case "list_dir":
		return "dirs"
	case "find_files":
		return "matches"
	case "edit_file", "write_file":
		return "edits"
	case "run_command":
		return "commands"
	case "git_status", "git_log", "git_diff", "git_show":
		return "queries"
	default:
		return "calls"
	}
}

// Execute dispatches a tool call by name. args is the raw JSON
// arguments from the model. Returns the tool output string or an
// error. Mutating tools call ctx.Confirm before executing.
func Execute(name string, args json.RawMessage, ctx *Context) (string, error) {
	canonical := ResolveAlias(name)
	switch canonical {
	case "read_file":
		return ToolReadFile(args, ctx)
	case "list_dir":
		return ToolListDir(args, ctx)
	case "find_files":
		return ToolFindFiles(args, ctx)
	case "edit_file":
		return ToolEditFile(args, ctx)
	case "write_file":
		return ToolWriteFile(args, ctx)
	case "run_command":
		return ToolRunCommand(args, ctx)
	case "git_status":
		return ToolGit("status --porcelain=v1 -b", ctx)
	case "git_log":
		return ToolGitLog(args, ctx)
	case "git_diff":
		return ToolGitDiff(args, ctx)
	case "git_show":
		return ToolGitShow(args, ctx)
	default:
		return "", fmt.Errorf("unknown tool: %s", name)
	}
}

// parseStringArg extracts a string field from JSON args.
func parseStringArg(args json.RawMessage, field string) (string, error) {
	var m map[string]any
	if err := json.Unmarshal(args, &m); err != nil {
		return "", fmt.Errorf("parse args: %w", err)
	}
	v, ok := m[field]
	if !ok {
		return "", fmt.Errorf("missing required parameter: %s", field)
	}
	s, ok := v.(string)
	if !ok {
		return "", fmt.Errorf("parameter %s must be a string", field)
	}
	return s, nil
}

// parseIntArg extracts an int field from JSON args, returning the
// default if absent.
func parseIntArg(args json.RawMessage, field string, def int) int {
	var m map[string]any
	if err := json.Unmarshal(args, &m); err != nil {
		return def
	}
	v, ok := m[field]
	if !ok {
		return def
	}
	f, ok := v.(float64)
	if !ok {
		return def
	}
	return int(f)
}

// parseBoolArg extracts a bool field from JSON args, returning the
// default if absent.
func parseBoolArg(args json.RawMessage, field string, def bool) bool {
	var m map[string]any
	if err := json.Unmarshal(args, &m); err != nil {
		return def
	}
	v, ok := m[field]
	if !ok {
		return def
	}
	b, ok := v.(bool)
	if !ok {
		return def
	}
	return b
}
