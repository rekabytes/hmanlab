package tools

import (
	"fmt"
	"os"
	"path/filepath"
)

// MaxFileBytes is the output cap for read_file.
const MaxFileBytes = 50_000

// MaxCmdBytes is the output cap for run_command + git tools.
const MaxCmdBytes = 4_000

// MaxWriteBytes is the size cap for edit_file + write_file content.
const MaxWriteBytes = 500_000

// CmdTimeout is the max duration for a single run_command.
const CmdTimeoutSec = 600

// FindFilesCap limits the number of find_files results.
const FindFilesCap = 100

// IgnoredDirs are directory names that find_files skips.
var IgnoredDirs = map[string]bool{
	"target": true, "node_modules": true, ".git": true, "dist": true,
	"build": true, ".next": true, ".turbo": true, ".cache": true,
	"__pycache__": true, ".venv": true, "venv": true,
	".pytest_cache": true, ".mypy_cache": true, ".ruff_cache": true,
	"vendor": true, ".idea": true, ".vscode": true,
}

// ResolveInWorkspace canonicalizes both paths and rejects anything
// outside the workspace. Mirrors cli/src/tools/workspace.rs:15-54.
func ResolveInWorkspace(workspace, input string) (string, error) {
	abs := input
	if !filepath.IsAbs(abs) {
		abs = filepath.Join(workspace, abs)
	}
	// Resolve symlinks where possible. For non-existent paths, resolve
	// the parent and append the remaining component.
	absCan, err := filepath.EvalSymlinks(abs)
	if err != nil {
		parent := filepath.Dir(abs)
		base := filepath.Base(abs)
		parentCan, perr := filepath.EvalSymlinks(parent)
		if perr != nil {
			// Parent doesn't exist either — use the raw abs path.
			absCan = filepath.Clean(abs)
		} else {
			absCan = filepath.Join(parentCan, base)
		}
	}
	wsCan, err := filepath.EvalSymlinks(workspace)
	if err != nil {
		wsCan = filepath.Clean(workspace)
	}
	if !filepath.HasPrefix(absCan, wsCan) {
		return "", fmt.Errorf("path '%s' escapes the workspace", input)
	}
	return abs, nil
}

// TruncateUTF8 truncates s to at most maxBytes, respecting UTF-8
// boundaries. Mirrors cli/src/tools/workspace.rs truncate_utf8.
func TruncateUTF8(s string, maxBytes int) string {
	if len(s) <= maxBytes {
		return s
	}
	// Walk backward to find a valid UTF-8 boundary.
	cut := maxBytes
	for cut > 0 && !utf8RuneStart(s[cut]) {
		cut--
	}
	return s[:cut] + "\n…[truncated]"
}

func utf8RuneStart(b byte) bool {
	return b&0xC0 != 0x80
}

// FileExists returns true if path exists (file or directory).
func FileExists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}
