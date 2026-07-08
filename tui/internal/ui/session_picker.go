package ui

import (
	"fmt"
	"strings"
	"time"

	"github.com/charmbracelet/lipgloss"

	"github.com/hmanlab/hmanlab/tui/internal/session"
	"github.com/hmanlab/hmanlab/tui/internal/ui/theme"
)

// renderSessionOverlay draws a modern centered session picker. Each row
// shows the session title (primary) and relative time (dim, right-aligned).
// Arrow keys navigate, Enter loads, Esc cancels.
func renderSessionOverlay(width, height int, list []session.SessionSummary, cursor int) string {
	// Card sizing — generous but fits any reasonable terminal.
	cardW := 76
	if width < cardW+4 {
		cardW = width - 4
	}
	if cardW < 44 {
		cardW = 44
	}

	maxRows := height / 2
	if maxRows < 4 {
		maxRows = 4
	}
	if maxRows > len(list) {
		maxRows = len(list)
	}

	// ── Title bar ──────────────────────────────────────────────
	mark := lipgloss.NewStyle().Foreground(theme.HibiscusGlow).Render("✿")
	titleText := lipgloss.NewStyle().
		Foreground(theme.Hibiscus).
		Bold(true).
		Render("Sessions")
	title := fmt.Sprintf("  %s %s", mark, titleText)

	countLabel := lipgloss.NewStyle().
		Foreground(theme.FGDim).
		Render(fmt.Sprintf("%d session(s)", len(list)))

	// Right-pad the title row so the count sits on the far right.
	titlePad := cardW - lipgloss.Width(title) - lipgloss.Width(countLabel) - 4
	if titlePad < 0 {
		titlePad = 0
	}
	titleRow := title + strings.Repeat(" ", titlePad) + countLabel

	// ── Separator ──────────────────────────────────────────────
	sep := lipgloss.NewStyle().
		Foreground(theme.HibiscusDim).
		Render(strings.Repeat("─", cardW-4))

	// ── Session rows ───────────────────────────────────────────
	var rows []string
	start := 0
	if cursor >= maxRows {
		start = cursor - maxRows + 1
	}
	end := start + maxRows
	if end > len(list) {
		end = len(list)
	}

	innerW := cardW - 6 // padding (2 each side) + border (2)
	for i := start; i < end; i++ {
		s := list[i]
		rel := relativeTime(s.UpdatedAt)
		titleText := s.Title
		if titleText == "" {
			titleText = "(untitled)"
		}

		// Truncate title if too long to leave room for the timestamp.
		maxTitleW := innerW - len(rel) - 3
		if maxTitleW < 10 {
			maxTitleW = 10
		}
		if lipgloss.Width(titleText) > maxTitleW {
			titleText = truncateStr(titleText, maxTitleW)
		}

		var row string
		if i == cursor {
			selected := lipgloss.NewStyle().
				Background(theme.Hibiscus).
				Foreground(theme.BGBase).
				Bold(true)
			label := fmt.Sprintf("  ▸ %s", titleText)
			ts := fmt.Sprintf("%s  ", rel)
			padN := innerW - lipgloss.Width(label) - lipgloss.Width(ts)
			if padN < 0 {
				padN = 0
			}
			// Render the whole row (including padding) in one shot so
			// the hibiscus background fills the entire width.
			row = selected.Width(innerW).Render(label + strings.Repeat(" ", padN) + ts)
		} else {
			label := lipgloss.NewStyle().
				Foreground(theme.FG).
				Render(fmt.Sprintf("    %s", titleText))
			ts := lipgloss.NewStyle().
				Foreground(theme.FGDim).
				Render(rel)
			padN := innerW - lipgloss.Width(label) - lipgloss.Width(ts)
			if padN < 0 {
				padN = 0
			}
			row = label + strings.Repeat(" ", padN) + ts
		}
		rows = append(rows, row)
	}

	// ── Footer hint ────────────────────────────────────────────
	hint := lipgloss.NewStyle().
		Foreground(theme.FGDim).
		Italic(true).
		Render("↑↓ navigate   ⏎ load   esc cancel")

	// ── Assemble card ──────────────────────────────────────────
	content := lipgloss.JoinVertical(lipgloss.Left,
		titleRow,
		sep,
		"",
		strings.Join(rows, "\n"),
		"",
		hint,
	)

	cardStyle := lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(theme.Hibiscus).
		Padding(1, 2).
		Width(cardW)

	card := cardStyle.Render(strings.TrimRight(content, "\n"))

	return lipgloss.Place(width, height,
		lipgloss.Center, lipgloss.Center,
		card,
		lipgloss.WithWhitespaceChars(" "),
		lipgloss.WithWhitespaceForeground(theme.BGBase),
	)
}

// truncateStr trims s to maxW visible columns, appending an ellipsis.
func truncateStr(s string, maxW int) string {
	if lipgloss.Width(s) <= maxW {
		return s
	}
	// Rune-by-rune until we fit + the ellipsis.
	r := []rune(s)
	for len(r) > 0 && lipgloss.Width(string(r)+"…") > maxW {
		r = r[:len(r)-1]
	}
	return string(r) + "…"
}

// relativeTime renders a short, human-friendly relative timestamp.
func relativeTime(t time.Time) string {
	if t.IsZero() {
		return ""
	}
	d := time.Since(t)
	switch {
	case d < time.Minute:
		return "just now"
	case d < time.Hour:
		return fmt.Sprintf("%dm ago", int(d.Minutes()))
	case d < 24*time.Hour:
		return fmt.Sprintf("%dh ago", int(d.Hours()))
	case d < 7*24*time.Hour:
		return fmt.Sprintf("%dd ago", int(d.Hours()/24))
	default:
		return t.Format("Jan 2")
	}
}
