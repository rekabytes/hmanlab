package tools

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

// ToolReadFile reads a text file, capping at MaxFileBytes. Each line
// is prefixed with `<line_no>\t`.
func ToolReadFile(args json.RawMessage, ctx *Context) (string, error) {
	path, err := parseStringArg(args, "path")
	if err != nil {
		return "", err
	}
	abs, err := ResolveInWorkspace(ctx.Workspace, path)
	if err != nil {
		return "", err
	}
	data, err := os.ReadFile(abs)
	if err != nil {
		return "", fmt.Errorf("read %s: %w", path, err)
	}
	if len(data) > MaxFileBytes {
		data = []byte(TruncateUTF8(string(data), MaxFileBytes))
	}
	// Prefix each line with line number + tab.
	lines := strings.Split(string(data), "\n")
	var out strings.Builder
	for i, line := range lines {
		fmt.Fprintf(&out, "%d\t%s\n", i+1, line)
	}
	return strings.TrimRight(out.String(), "\n"), nil
}

// ToolListDir lists the contents of a directory.
func ToolListDir(args json.RawMessage, ctx *Context) (string, error) {
	path := "."
	if p, err := parseStringArg(args, "path"); err == nil && p != "" {
		path = p
	}
	abs, err := ResolveInWorkspace(ctx.Workspace, path)
	if err != nil {
		return "", err
	}
	entries, err := os.ReadDir(abs)
	if err != nil {
		return "", fmt.Errorf("list %s: %w", path, err)
	}
	sort.Slice(entries, func(i, j int) bool {
		return strings.ToLower(entries[i].Name()) < strings.ToLower(entries[j].Name())
	})
	var out strings.Builder
	for _, e := range entries {
		name := e.Name()
		if e.IsDir() {
			name += "/"
		} else if e.Type()&os.ModeSymlink != 0 {
			name += "@"
		}
		fmt.Fprintln(&out, name)
	}
	return strings.TrimRight(out.String(), "\n"), nil
}

// ToolFindFiles finds files matching a glob pattern.
func ToolFindFiles(args json.RawMessage, ctx *Context) (string, error) {
	pattern, err := parseStringArg(args, "pattern")
	if err != nil {
		return "", err
	}
	var matches []string
	err = filepath.Walk(ctx.Workspace, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return nil
		}
		if info.IsDir() {
			if IgnoredDirs[info.Name()] {
				return filepath.SkipDir
			}
			return nil
		}
		matched, _ := filepath.Match(pattern, filepath.Base(path))
		if !matched {
			return nil
		}
		rel, _ := filepath.Rel(ctx.Workspace, path)
		matches = append(matches, rel)
		if len(matches) >= FindFilesCap {
			return filepath.SkipAll
		}
		return nil
	})
	if err != nil {
		return "", err
	}
	if len(matches) == 0 {
		return "no matches", nil
	}
	return strings.Join(matches, "\n"), nil
}
