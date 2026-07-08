package ui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/lipgloss"

	"github.com/hmanlab/hmanlab/tui/internal/ui/theme"
)

// renderCmdDropdown renders the slash-command autocomplete list that
// appears above the input area. Each row shows the command name + a
// short description. The highlighted row is the one Enter will execute.
//
// The list is borderless and compact — it reads as a floating suggestion
// bar, not a modal. Each row is exactly one terminal line so the height
// = number of matches.
func renderCmdDropdown(matches []commandInfo, cursor, width int) string {
	// Column widths: command name gets a fixed slot, description fills
	// the rest.
	nameW := 14
	descW := width - nameW - 4 // 4 = gutter + gap
	if descW < 10 {
		descW = 10
	}

	var rows []string
	for i, c := range matches {
		var row string
		if i == cursor {
			// Single style with background + width so the hibiscus
			// highlight fills the entire row, not just the text.
			row = lipgloss.NewStyle().
				Background(theme.Hibiscus).
				Foreground(theme.BGBase).
				Bold(true).
				Width(width).
				Render(fmt.Sprintf("▸ %-*s  %-*s", nameW, c.display, descW, c.desc))
		} else {
			name := lipgloss.NewStyle().
				Foreground(theme.HibiscusGlow).
				Render(fmt.Sprintf("%-*s", nameW, "  "+c.display))
			desc := lipgloss.NewStyle().
				Foreground(theme.FGDim).
				Render(fmt.Sprintf("%-*s", descW, c.desc))
			row = lipgloss.NewStyle().Width(width).Render(name + "  " + desc)
		}
		rows = append(rows, row)
	}

	return strings.Join(rows, "\n")
}
