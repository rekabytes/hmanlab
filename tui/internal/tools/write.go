package tools

import (
	"encoding/json"
	"fmt"
	"os"
	"strings"
)

// ToolEditFile replaces old_string with new_string in a file. Calls
// ctx.Confirm before writing, showing a diff preview.
func ToolEditFile(args json.RawMessage, ctx *Context) (string, error) {
	path, err := parseStringArg(args, "path")
	if err != nil {
		return "", err
	}
	oldStr, err := parseStringArg(args, "old_string")
	if err != nil {
		return "", err
	}
	newStr, err := parseStringArg(args, "new_string")
	if err != nil {
		return "", err
	}
	replaceAll := parseBoolArg(args, "replace_all", false)

	abs, err := ResolveInWorkspace(ctx.Workspace, path)
	if err != nil {
		return "", err
	}
	data, err := os.ReadFile(abs)
	if err != nil {
		return "", fmt.Errorf("read %s: %w", path, err)
	}
	content := string(data)

	if !strings.Contains(content, oldStr) {
		return "", fmt.Errorf("old_string not found in %s", path)
	}

	var newContent string
	if replaceAll {
		newContent = strings.ReplaceAll(content, oldStr, newStr)
	} else {
		newContent = strings.Replace(content, oldStr, newStr, 1)
	}

	if len(newContent) > MaxWriteBytes {
		return "", fmt.Errorf("file too large after edit (cap: %d bytes)", MaxWriteBytes)
	}

	// Confirm before writing.
	if ctx.Confirm != nil {
		diff := BuildSimpleDiff(oldStr, newStr)
		if !ctx.Confirm(fmt.Sprintf("Edit %s", path), diff) {
			return "edit denied by user", nil
		}
	}

	if err := os.WriteFile(abs, []byte(newContent), 0644); err != nil {
		return "", fmt.Errorf("write %s: %w", path, err)
	}
	return fmt.Sprintf("edited %s", path), nil
}

// ToolWriteFile writes content to a file, creating or overwriting.
// Calls ctx.Confirm before writing.
func ToolWriteFile(args json.RawMessage, ctx *Context) (string, error) {
	path, err := parseStringArg(args, "path")
	if err != nil {
		return "", err
	}
	content, err := parseStringArg(args, "content")
	if err != nil {
		return "", err
	}
	if len(content) > MaxWriteBytes {
		return "", fmt.Errorf("content too large (cap: %d bytes)", MaxWriteBytes)
	}

	abs, err := ResolveInWorkspace(ctx.Workspace, path)
	if err != nil {
		return "", err
	}

	// Confirm before writing.
	if ctx.Confirm != nil {
		diff := []DiffLine{{Kind: DiffSummary, Text: fmt.Sprintf("Write %d bytes to %s", len(content), path)}}
		if !ctx.Confirm(fmt.Sprintf("Write %s", path), diff) {
			return "write denied by user", nil
		}
	}

	if err := os.WriteFile(abs, []byte(content), 0644); err != nil {
		return "", fmt.Errorf("write %s: %w", path, err)
	}
	return fmt.Sprintf("wrote %s", path), nil
}
