package ui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/bubbles/textarea"
	"github.com/charmbracelet/bubbles/textinput"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"

	"github.com/hmanlab/hmanlab/tui/internal/config"
	"github.com/hmanlab/hmanlab/tui/internal/ui/theme"
)

// renderKeyInputModal draws a centered card for entering an API key
// for a specific provider.
func renderKeyInputModal(width, height int, provider string, input textinput.Model) string {
	cardW := 58
	if width < cardW+4 {
		cardW = width - 4
	}
	if cardW < 36 {
		cardW = 36
	}

	mark := lipgloss.NewStyle().Foreground(theme.HibiscusGlow).Render("✿")
	title := lipgloss.NewStyle().
		Foreground(theme.Hibiscus).
		Bold(true).
		Render("Add API Key")
	titleRow := fmt.Sprintf("  %s %s", mark, title)

	label := lipgloss.NewStyle().
		Foreground(theme.FG).
		Bold(true).
		Render(config.ProviderLabel(provider))

	hint := lipgloss.NewStyle().
		Foreground(theme.HibiscusGlow).
		Render("Get a key at: " + config.ProviderGetURL(provider))

	actions := fmt.Sprintf("[ %s ]   [ %s ]",
		lipgloss.NewStyle().Foreground(theme.Success).Bold(true).Render("save"),
		lipgloss.NewStyle().Foreground(theme.Error).Bold(true).Render("cancel (esc)"))

	content := lipgloss.JoinVertical(lipgloss.Left,
		titleRow,
		"",
		label,
		hint,
		"",
		"API key: "+input.View(),
		"",
		actions,
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

// updateKeyInput handles keys while the API key input modal is open.
func (m Model) updateKeyInput(msg tea.Msg) (tea.Model, tea.Cmd) {
	k, ok := msg.(tea.KeyMsg)
	if !ok {
		var cmd tea.Cmd
		m.keyInput, cmd = m.keyInput.Update(msg)
		return m, cmd
	}
	switch k.Type {
	case tea.KeyEsc, tea.KeyCtrlC:
		m.keyInputActive = false
		m.keyInputProvider = ""
		m.keyInput.Blur()
		m.input.Focus()
		return m, textarea.Blink
	case tea.KeyEnter:
		key := strings.TrimSpace(m.keyInput.Value())
		if key == "" {
			return m, nil
		}
		// Save the key to config.
		m.cfg.SetByokKey(m.keyInputProvider, key)
		if err := m.cfg.Save(); err != nil {
			m.history = append(m.history, infoLine(fmt.Sprintf(
				"⚠ saved key but couldn't write config: %v", err)))
		}
		provider := m.keyInputProvider
		m.history = append(m.history, infoLine(fmt.Sprintf(
			"✓ %s configured. Default model: %s",
			config.ProviderLabel(provider),
			config.ProviderDefaultModel(provider))))
		m.keyInputActive = false
		m.keyInputProvider = ""
		m.keyInput.Blur()
		m.keyInput.SetValue("")
		m.input.Focus()
		m.refreshViewportContent()
		return m, textarea.Blink
	}
	// Default: forward to textinput.
	var cmd tea.Cmd
	m.keyInput, cmd = m.keyInput.Update(msg)
	return m, cmd
}
