package ui

import (
	"fmt"
	"strings"

	"github.com/hmanlab/hmanlab/tui/internal/llm"
)

// slashCommand is the parsed result of a /command line. Mirrors the
// shape of cli/src/app/commands/mod.rs::Command but only the variants
// we ship at v0.
type slashCommand struct {
	kind string // "help" "clear" "quit" "model" "unknown"
	// arg holds the rest of the line after the command word. For
	// /model this is the model name (may be empty for the show-current
	// form). For other commands it's unused.
	arg string
}

// parseSlashCommand inspects a textarea line, returning a slashCommand
// if the line starts with `/`, or ok=false if it's a normal message.
//
// Matches the cli's parse_command (cli/src/app/commands/mod.rs:136) in
// shape: leading `/`, the head word selects the command, the rest of
// the line is the argument (trimmed).
func parseSlashCommand(line string) (cmd slashCommand, ok bool) {
	t := strings.TrimSpace(line)
	if !strings.HasPrefix(t, "/") {
		return slashCommand{}, false
	}
	body := strings.TrimPrefix(t, "/")
	head, rest := body, ""
	if i := strings.IndexAny(body, " \t"); i >= 0 {
		head, rest = body[:i], strings.TrimSpace(body[i+1:])
	}
	head = strings.ToLower(head)
	switch head {
	case "help", "?":
		return slashCommand{kind: "help"}, true
	case "clear":
		return slashCommand{kind: "clear"}, true
	case "quit", "exit", "q":
		return slashCommand{kind: "quit"}, true
	case "model":
		return slashCommand{kind: "model", arg: rest}, true
	}
	return slashCommand{kind: "unknown", arg: head}, true
}

// helpText is what /help renders. Keep this list in lockstep with the
// parseSlashCommand switch above — if a command exists, it should be
// documented here, and vice versa.
func helpText(currentModel string) string {
	return strings.TrimSpace(fmt.Sprintf(`
**Hibiscus** — terminal client for Ollama Cloud (v0)

Connected to: **Ollama Cloud**
Current model: %s

**Slash commands**
- /help            Show this help
- /model [name]    Switch model (no arg: show current). Valid: glm-4.7, glm-5, qwen3.5, kimi-k2.6, …
- /clear           Wipe chat history, start a fresh session
- /quit            Exit (Ctrl+D on empty input also works)

**Keys**
- Enter            Send message
- Alt+Enter        New line (in input box)
- Ctrl+C           Cancel in-flight response / exit on empty input
- PgUp / PgDn      Scroll chat history
- mouse wheel      Scroll chat history

The agent loop (tool calls, multi-tool grouping, sessions, specialist
agents) is v0.1+. See tui/docs/development/PRD.md for the roadmap.
`, currentModel))
}

// infoLine builds a [system] info message for the chat history.
func infoLine(text string) chatMessage {
	return chatMessage{role: llm.RoleSystem, content: text}
}
