package ui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/lipgloss"

	"github.com/hmanlab/hmanlab/tui/internal/config"
	"github.com/hmanlab/hmanlab/tui/internal/ui/theme"
)

// modelEntry is one row in the /model picker.
type modelEntry struct {
	provider    string
	model       string
	configured  bool
}

// buildModelList assembles the full model list from all providers,
// grouped by provider. Configured providers first, then unconfigured.
func (m Model) buildModelList() []modelEntry {
	configured := make(map[string]bool)
	for _, p := range m.cfg.ConfiguredProviders() {
		configured[p] = true
	}

	var entries []modelEntry
	// Configured providers first.
	for _, p := range config.BYOKProviderIDs {
		if !configured[p] {
			continue
		}
		for _, model := range config.ProviderModels(p) {
			entries = append(entries, modelEntry{provider: p, model: model, configured: true})
		}
		// User-added models.
		for _, em := range m.cfg.ExtraModels {
			if em.Provider == p {
				entries = append(entries, modelEntry{provider: p, model: em.Name, configured: true})
			}
		}
	}
	// Unconfigured providers — show as "+ Add key" rows.
	for _, p := range config.BYOKProviderIDs {
		if configured[p] {
			continue
		}
		entries = append(entries, modelEntry{provider: p, model: "+ Add " + config.ProviderLabel(p) + " key", configured: false})
	}
	return entries
}

// renderModelOverlay draws the /model picker — a centered list of all
// models grouped by provider. Arrow keys navigate, Enter selects.
func renderModelOverlay(width, height int, entries []modelEntry, cursor int, currentModel string) string {
	cardW := 64
	if width < cardW+4 {
		cardW = width - 4
	}
	if cardW < 40 {
		cardW = 40
	}

	maxRows := height / 2
	if maxRows < 5 {
		maxRows = 5
	}
	if maxRows > len(entries) {
		maxRows = len(entries)
	}

	// Header.
	mark := lipgloss.NewStyle().Foreground(theme.HibiscusGlow).Render("✿")
	title := lipgloss.NewStyle().Foreground(theme.Hibiscus).Bold(true).Render("Models")
	headerRow := fmt.Sprintf("  %s %s", mark, title)

	sep := lipgloss.NewStyle().Foreground(theme.HibiscusDim).Render(strings.Repeat("─", cardW-4))

	// Rows.
	var rows []string
	start := 0
	if cursor >= maxRows {
		start = cursor - maxRows + 1
	}
	end := start + maxRows
	if end > len(entries) {
		end = len(entries)
	}

	innerW := cardW - 6
	var lastProvider string
	for i := start; i < end; i++ {
		e := entries[i]

		// Provider group header when provider changes.
		if e.provider != lastProvider {
			label := config.ProviderLabel(e.provider)
			if e.configured {
				label = "✓ " + label
			} else {
				label = "  " + label
			}
			rows = append(rows, lipgloss.NewStyle().
				Foreground(theme.FGDim).
				Italic(true).
				Width(innerW).
				Render(label))
			lastProvider = e.provider
		}

		isCursor := i == cursor
		isCurrent := e.model == currentModel

		var marker string
		if isCurrent {
			marker = lipgloss.NewStyle().Foreground(theme.HibiscusGlow).Render("✿")
		} else {
			marker = " "
		}

		modelText := e.model
		if !e.configured {
			modelText = lipgloss.NewStyle().Foreground(theme.FGDim).Render(e.model)
		}

		var row string
		if isCursor {
			row = lipgloss.NewStyle().
				Background(theme.Hibiscus).
				Foreground(theme.BGBase).
				Bold(true).
				Width(innerW).
				Render(fmt.Sprintf("▸ %s %s", marker, modelText))
		} else {
			row = lipgloss.NewStyle().
				Width(innerW).
				Render(fmt.Sprintf("  %s %s", marker, modelText))
		}
		rows = append(rows, row)
	}

	hint := lipgloss.NewStyle().
		Foreground(theme.FGDim).
		Italic(true).
		Render("↑↓ navigate   ⏎ select   esc cancel")

	content := lipgloss.JoinVertical(lipgloss.Left,
		headerRow,
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
