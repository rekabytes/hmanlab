package tools

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os/exec"
	"regexp"
	"strings"
	"time"
	"unicode"
)

// gitExec runs a git command in the workspace and returns trimmed output.
func gitExec(ctx *Context, args ...string) (string, error) {
	cctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	cmd := exec.CommandContext(cctx, "git", args...)
	cmd.Dir = ctx.Workspace
	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		return "", fmt.Errorf("git %s: %w (stderr: %s)",
			strings.Join(args, " "), err, strings.TrimSpace(stderr.String()))
	}
	out := stdout.String()
	if len(out) > MaxCmdBytes {
		out = TruncateUTF8(out, MaxCmdBytes)
	}
	return strings.TrimSpace(out), nil
}

// ToolGit runs an arbitrary git subcommand (used by git_status).
func ToolGit(subcmd string, ctx *Context) (string, error) {
	args := strings.Fields(subcmd)
	return gitExec(ctx, args...)
}

// ToolGitLog runs git log --oneline -n N.
func ToolGitLog(args json.RawMessage, ctx *Context) (string, error) {
	limit := parseIntArg(args, "limit", 10)
	if limit < 1 {
		limit = 1
	}
	if limit > 100 {
		limit = 100
	}
	return gitExec(ctx, "log", "--oneline", "-n", fmt.Sprintf("%d", limit))
}

// ToolGitDiff runs git diff with optional range and path.
func ToolGitDiff(args json.RawMessage, ctx *Context) (string, error) {
	gitArgs := []string{"diff"}
	if r := parseStringArgSafe(args, "range"); r != "" {
		gitArgs = append(gitArgs, r)
	}
	if p := parseStringArgSafe(args, "path"); p != "" {
		gitArgs = append(gitArgs, "--", p)
	}
	return gitExec(ctx, gitArgs...)
}

// revWhitelist matches safe characters for a git rev.
var revWhitelist = regexp.MustCompile(`^[\w\-/.~^]+$`)

// ToolGitShow runs git show <rev>.
func ToolGitShow(args json.RawMessage, ctx *Context) (string, error) {
	rev, err := parseStringArg(args, "rev")
	if err != nil {
		return "", err
	}
	if !revWhitelist.MatchString(rev) {
		return "", fmt.Errorf("invalid rev: %s (allowed: alphanumeric, -, _, /, ., ~, ^)", rev)
	}
	return gitExec(ctx, "show", rev)
}

// parseStringArgSafe returns the string value or "" if absent/error.
func parseStringArgSafe(args json.RawMessage, field string) string {
	s, err := parseStringArg(args, field)
	if err != nil {
		return ""
	}
	return s
}

// isSafeRev checks that all chars in s are safe for a git rev.
func isSafeRev(s string) bool {
	for _, r := range s {
		if !unicode.IsLetter(r) && !unicode.IsDigit(r) &&
			r != '-' && r != '_' && r != '/' && r != '.' &&
			r != '~' && r != '^' {
			return false
		}
	}
	return true
}
