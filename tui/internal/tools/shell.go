package tools

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os/exec"
	"strings"
	"time"
)

// ToolRunCommand executes a shell command via `sh -c`. Output is
// captured (stdout + stderr), capped at MaxCmdBytes. Timeout is
// CmdTimeoutSec. Calls ctx.Confirm before running.
func ToolRunCommand(args json.RawMessage, ctx *Context) (string, error) {
	command, err := parseStringArg(args, "command")
	if err != nil {
		return "", err
	}

	if ctx.Confirm != nil {
		if !ctx.Confirm(fmt.Sprintf("Run shell command:\n  %s", command), nil) {
			return "command denied by user", nil
		}
	}

	timeout := time.Duration(CmdTimeoutSec) * time.Second
	cctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()

	cmd := exec.CommandContext(cctx, "sh", "-c", command)
	cmd.Dir = ctx.Workspace

	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr

	err = cmd.Run()

	out := stdout.String()
	if stderr.Len() > 0 {
		out += "\n[stderr]\n" + stderr.String()
	}

	if cctx.Err() == context.DeadlineExceeded {
		out += fmt.Sprintf("\n[timed out after %d seconds]", CmdTimeoutSec)
	}

	if err != nil {
		if exitErr, ok := err.(*exec.ExitError); ok {
			out += fmt.Sprintf("\n[exit %d]", exitErr.ExitCode())
		}
	}

	if len(out) > MaxCmdBytes {
		out = TruncateUTF8(out, MaxCmdBytes)
	}
	return strings.TrimSpace(out), nil
}
