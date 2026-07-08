package ui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/lipgloss"

	"github.com/hmanlab/hmanlab/tui/internal/session"
	"github.com/hmanlab/hmanlab/tui/internal/ui/theme"
)

// renderSidebar renders the full-height right panel: session list with
// a header and a token-stats footer.
//
// Layout (top → bottom):
//
//	✿ Sessions           ← header
//	sessions              ← section label
//	──────────────        ← separator
//	(session list)        ← scrollable, 2 rows per session
//	──────────────        ← separator
//	↑ 12.4k   ↓ 3.8k      ← in/out on one row
//	ctx  16.3k            ← context total
//	(blank)               ← padding
func (m Model) renderSidebar() string {
	w := sidebarWidth
	h := m.height

	// ── Header ───────────────────────────────────────────────────
	mark := lipgloss.NewStyle().Foreground(theme.HibiscusGlow).Render("✿")
	sessTitle := lipgloss.NewStyle().
		Foreground(theme.Hibiscus).
		Bold(true).
		Render("Sessions")
	headerRow := lipgloss.NewStyle().
		Width(w).
		Background(theme.BGBase).
		Render(fmt.Sprintf(" %s %s", mark, sessTitle))

	// ── Section label ────────────────────────────────────────────
	labelRow := lipgloss.NewStyle().
		Foreground(theme.HibiscusGlow).
		Width(w).
		Background(theme.BGBase).
		Render(" sessions")

	sep := lipgloss.NewStyle().
		Foreground(theme.HibiscusDim).
		Width(w).
		Background(theme.BGBase).
		Render(strings.Repeat("─", w))

	// ── Session list ─────────────────────────────────────────────
	// Fixed rows: header(1) + label(1) + sep(1) + sep(1) + footer(4) = 8.
	fixedH := 8
	listH := h - fixedH
	if listH < 2 {
		listH = 2
	}
	maxSessions := listH / 2

	activeID := ""
	if m.activeSession != nil {
		activeID = m.activeSession.SessionID
	}

	var listRows []string
	if len(m.sidebarSessions) == 0 {
		listRows = append(listRows, lipgloss.NewStyle().
			Foreground(theme.FGDim).
			Italic(true).
			Width(w).
			Background(theme.BGBase).
			Render(" No sessions yet"))
		for i := 1; i < listH; i++ {
			listRows = append(listRows, blankSidebarRow(w))
		}
	} else {
		start := 0
		if m.sidebarCursor >= maxSessions {
			start = m.sidebarCursor - maxSessions + 1
		}
		end := start + maxSessions
		if end > len(m.sidebarSessions) {
			end = len(m.sidebarSessions)
		}

		for i := start; i < end; i++ {
			s := m.sidebarSessions[i]
			isActive := s.SessionID == activeID
			isCursor := i == m.sidebarCursor && m.sidebarFocus

			listRows = append(listRows, m.renderSessionRow(s, isActive, isCursor, w))

			rel := relativeTime(s.UpdatedAt)
			timeStyle := lipgloss.NewStyle().
				Foreground(theme.FGDim).
				Width(w).
				Background(theme.BGBase)
			if isCursor {
				timeStyle = timeStyle.Background(theme.BGCardHover)
			}
			listRows = append(listRows, timeStyle.Render(fmt.Sprintf("   %s", rel)))
		}

		used := (end - start) * 2
		for i := used; i < listH; i++ {
			listRows = append(listRows, blankSidebarRow(w))
		}
	}

	// ── Token footer (4 rows) ────────────────────────────────────
	upArrow := lipgloss.NewStyle().Foreground(theme.User).Render("↑")
	downArrow := lipgloss.NewStyle().Foreground(theme.Assistant).Render("↓")
	inNum := lipgloss.NewStyle().Foreground(theme.FG).Render(formatTokens(m.totalPromptTokens))
	outNum := lipgloss.NewStyle().Foreground(theme.FG).Render(formatTokens(m.totalCompletionTokens))

	// Row 1: in/out on one line.
	tokenRow := lipgloss.NewStyle().
		Width(w).
		Background(theme.BGBase).
		Render(fmt.Sprintf(" %s %s   %s %s", upArrow, inNum, downArrow, outNum))

	// Row 2: context total (prompt + completion cumulative).
	ctxTotal := m.totalPromptTokens + m.totalCompletionTokens
	ctxLabel := lipgloss.NewStyle().Foreground(theme.FGDim).Render("ctx")
	ctxNum := lipgloss.NewStyle().Foreground(theme.FG).Render(formatTokens(ctxTotal))
	ctxRow := lipgloss.NewStyle().
		Width(w).
		Background(theme.BGBase).
		Render(fmt.Sprintf(" %s  %s", ctxLabel, ctxNum))

	// Row 3: padding.
	padRow := blankSidebarRow(w)

	// ── Assemble ─────────────────────────────────────────────────
	content := lipgloss.JoinVertical(lipgloss.Left,
		headerRow,
		labelRow,
		sep,
		strings.Join(listRows, "\n"),
		sep,
		tokenRow,
		ctxRow,
		padRow,
	)

	borderStyle := lipgloss.NewStyle().
		BorderLeft(true).
		BorderForeground(theme.FGDimmer).
		PaddingLeft(0)
	return borderStyle.Render(content)
}

// renderSessionRow renders one session's title row in the sidebar.
func (m Model) renderSessionRow(s session.SessionSummary, isActive, isCursor bool, w int) string {
	var marker string
	if isActive {
		marker = lipgloss.NewStyle().Foreground(theme.HibiscusGlow).Render("✿")
	} else {
		marker = lipgloss.NewStyle().Foreground(theme.FGDim).Render("○")
	}

	title := s.Title
	if title == "" {
		title = "(untitled)"
	}

	maxTitleW := w - 4 // marker + spaces
	if lipgloss.Width(title) > maxTitleW {
		title = truncateStr(title, maxTitleW)
	}

	var row string
	if isCursor {
		row = lipgloss.NewStyle().
			Background(theme.BGCardHover).
			Foreground(theme.FG).
			Bold(true).
			Width(w).
			Render(fmt.Sprintf(" %s %s", marker, title))
	} else if isActive {
		row = lipgloss.NewStyle().
			Foreground(theme.Hibiscus).
			Bold(true).
			Width(w).
			Background(theme.BGBase).
			Render(fmt.Sprintf(" %s %s", marker, title))
	} else {
		row = lipgloss.NewStyle().
			Foreground(theme.FG).
			Width(w).
			Background(theme.BGBase).
			Render(fmt.Sprintf(" %s %s", marker, title))
	}
	return row
}

// blankSidebarRow returns an empty row matching the sidebar background.
func blankSidebarRow(w int) string {
	return lipgloss.NewStyle().
		Width(w).
		Background(theme.BGBase).
		Render(strings.Repeat(" ", w))
}

// formatTokens formats a token count for compact display.
func formatTokens(n int) string {
	if n < 1000 {
		return fmt.Sprintf("%d", n)
	}
	if n < 1000000 {
		return fmt.Sprintf("%.1fk", float64(n)/1000)
	}
	return fmt.Sprintf("%.1fM", float64(n)/1000000)
}
