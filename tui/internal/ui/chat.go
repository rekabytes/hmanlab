package ui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/lipgloss"
	"github.com/charmbracelet/x/ansi"
	"github.com/hmanlab/hmanlab/tui/internal/llm"
	"github.com/hmanlab/hmanlab/tui/internal/ui/theme"
)

// chatMessage is one row in the chat history. Mirrors the shape of
// cli/src/ollama.rs::ChatMessage but only the fields we need at v0 —
// no tool_calls, no tool_args, no attachments.
type chatMessage struct {
	role    llm.Role
	content string
	// streaming is true iff this message is the in-flight assistant
	// response (tokens still arriving). The renderer shows a blinking
	// caret at its tail while streaming is true.
	streaming bool
	// cancelled is true if the user pressed Ctrl+C mid-stream. The
	// renderer shows a trailing […] marker so it's clear the response
	// was truncated, not finished.
	cancelled bool
}

// appendContent is the only safe mutator for streaming messages —
// called from the bubbletea Update on every token chunk.
func (m *chatMessage) appendContent(text string) {
	m.content += text
}

// render returns the styled string for this message, ready to be
// concatenated into the chat viewport. Width is the column count to
// wrap markdown + prose to.
func (m chatMessage) render(bodyW int, fullW int, animTick uint64) string {
	role := string(m.role)
	label := role
	switch role {
	case "user":
		label = "user"
	case "assistant":
		label = "hibiscus"
	case "info":
		label = "system"
	}

	// Header: colored role label with brand mark for hibiscus.
	var header string
	if role == "assistant" {
		if m.streaming {
			spinFrames := []string{"✿", "❋", "✾", "❀"}
			spin := spinFrames[int(animTick/2)%len(spinFrames)]
			// Pulse the text between Hibiscus and HibiscusGlow in sync
			// with the flower frames.
			textColor := theme.Hibiscus
			if int(animTick/2)%2 == 0 {
				textColor = theme.HibiscusGlow
			}
			header = lipgloss.NewStyle().Foreground(theme.HibiscusGlow).Render(spin) +
				lipgloss.NewStyle().Foreground(textColor).Bold(true).Render(" hibiscus — generating")
		} else {
			mark := lipgloss.NewStyle().Foreground(theme.HibiscusGlow).Render("✿")
			header = theme.RoleLabelStyle(role).Render(fmt.Sprintf("%s hibiscus", mark))
		}
	} else {
		header = theme.RoleLabelStyle(role).Render("▎ " + label)
	}

	// Body: assistant content goes through the markdown renderer; user
	// + system content render as plain text (markdown in user input is
	// usually an accident, not a styling intent).
	body := m.content
	if role == "assistant" {
		body = RenderMarkdown(m.content, bodyW-2)
	}
	if m.cancelled && !strings.HasSuffix(body, "[...]") {
		body += " [...]"
	}

	// Indent every body line by 2 cols to clear the gutter.
	indent := "  "
	bodyLines := strings.Split(body, "\n")
	for i, l := range bodyLines {
		bodyLines[i] = indent + l
	}

	result := header + "\n" + strings.Join(bodyLines, "\n")

	// User messages get a light grey background card.
	if role == "user" {
		lines := strings.Split(result, "\n")
		for i, l := range lines {
			clean := ansi.Strip(l)
			if i == 0 {
				lines[i] = theme.RoleLabelStyle("user").
					Background(theme.BGChat).
					Width(fullW).
					Render(clean)
			} else {
				lines[i] = lipgloss.NewStyle().
					Background(theme.BGChat).
					Width(fullW).
					Render(clean)
			}
		}
		result = strings.Join(lines, "\n")
	}

	return result
}

// String is the debug representation — never shown in the TUI.
func (m chatMessage) String() string {
	return fmt.Sprintf("[%s] %q", m.role, m.content)
}
