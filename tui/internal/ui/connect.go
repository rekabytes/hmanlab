package ui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/bubbles/textinput"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"

	"github.com/hmanlab/hmanlab/tui/internal/config"
	"github.com/hmanlab/hmanlab/tui/internal/ui/theme"
)

// connectModal is the first-run UX from PRD §7. Centered card over a
// dimmed backdrop, asks for the Ollama Cloud API key, validates it via
// /api/tags, writes the key to config.toml on success.
//
// Mirrors cli/src/app/state.rs:211 AddModel in shape: text-input
// modal + buttons + inline error state. TUI-only; this view doesn't
// need to round-trip with the cli since the persisted config is just
// a single string field.
type connectModal struct {
	input   textinput.Model
	state   connectState
	errMsg  string
	validating bool
}

type connectState int

const (
	connectInput connectState = iota
	connectValidating
	connectSuccess
)

func newConnectModal() connectModal {
	ti := textinput.New()
	ti.Placeholder = "Paste your Ollama Cloud API key"
	ti.PlaceholderStyle = lipgloss.NewStyle().Foreground(theme.HibiscusDim)
	ti.Prompt = ""
	ti.EchoMode = textinput.EchoNormal // cloud keys aren't secret-secret; show them so the user can verify they pasted right
	ti.Focus()
	ti.CharLimit = 256
	return connectModal{input: ti}
}

// Update handles input for the modal. Returns the (possibly updated)
// modal + a tea.Cmd + a bool "done" signal — done=true means the modal
// is closing (either successfully connected, or user aborted).
func (m connectModal) Update(msg tea.Msg, width, height int) (connectModal, tea.Cmd, bool) {
	switch msg := msg.(type) {
	case connectResultMsg:
		m.validating = false
		if msg.Err != nil {
			m.state = connectInput
			m.errMsg = msg.Err.Error()
			// Re-focus input for retry.
			m.input.Focus()
			return m, nil, false
		}
		// Success — caller will swap to chat view.
		m.state = connectSuccess
		return m, nil, true

	case tea.KeyMsg:
		if m.validating {
			// Swallow all keys while we're mid-request.
			return m, nil, false
		}
		switch msg.Type {
		case tea.KeyEnter:
			key := strings.TrimSpace(m.input.Value())
			if key == "" {
				m.errMsg = "API key can't be empty"
				return m, nil, false
			}
			m.errMsg = ""
			m.state = connectValidating
			m.validating = true
			m.input.Blur()
			// The connect action is sent as a tea.Cmd so the Update
			// loop sees the result via connectResultMsg. We pass the
			// key through the message; the parent Model handles the
			// actual validation + config save.
			return m, func() tea.Msg {
				return connectAttemptMsg{Key: key}
			}, false
		case tea.KeyCtrlC, tea.KeyEsc:
			// Abort = quit the whole program (no key to fall back to).
			return m, tea.Quit, true
		}
	}

	// Default: forward to text input.
	var cmd tea.Cmd
	m.input, cmd = m.input.Update(msg)
	return m, cmd, false
}

// View renders the modal as a centered card with a hibiscus border +
// glowing title. The card sits over a dimmed backdrop so it reads as
// an overlay rather than another panel.
func (m connectModal) View(width, height int) string {
	// Card sizing — fits inside any reasonable terminal, scales down
	// on narrow terms.
	cardW := 60
	if width < cardW+4 {
		cardW = width - 4
	}
	if cardW < 30 {
		cardW = 30
	}

	// Title: "✿ Connect to Ollama Cloud" — hibiscus flower mark in
	// the glow tone, the rest in primary hibiscus. Reads as the
	// brand's invitation to sign in.
	mark := lipgloss.NewStyle().Foreground(theme.HibiscusGlow).Render("✿")
	titleText := lipgloss.NewStyle().
		Foreground(theme.Hibiscus).
		Bold(true).
		Render(" Connect to Ollama Cloud ")
	title := mark + titleText + mark

	intro := "Stream from " + config.OllamaCloudBase
	hint := lipgloss.NewStyle().Foreground(theme.HibiscusGlow).Render("Get a key at: https://ollama.com")

	var actionLine string
	switch m.state {
	case connectValidating:
		actionLine = lipgloss.NewStyle().Foreground(theme.Hibiscus).Render("● validating…")
	case connectSuccess:
		actionLine = lipgloss.NewStyle().Foreground(theme.Success).Render("✓ connected")
	default:
		actionLine = fmt.Sprintf("[ %s ]   [ %s ]",
			lipgloss.NewStyle().Foreground(theme.Success).Bold(true).Render("connect"),
			lipgloss.NewStyle().Foreground(theme.Error).Bold(true).Render("quit (esc)"))
	}

	errLine := ""
	if m.errMsg != "" {
		errLine = lipgloss.NewStyle().Foreground(theme.Error).Render("✕ " + m.errMsg)
	}

	// Hibiscus-tinted border. Rounded corners + the brand color make
	// the modal read as the entry point to the hibiscus TUI.
	cardStyle := lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(theme.Hibiscus).
		Padding(1, 2).
		Width(cardW)

	body := lipgloss.JoinVertical(lipgloss.Left,
		title,
		"",
		lipgloss.NewStyle().Foreground(theme.FG).Render(intro),
		hint,
		"",
		"API key: "+m.input.View(),
		"",
		actionLine,
		errLine,
	)
	card := cardStyle.Render(strings.TrimRight(body, "\n"))

	// Center on the available area. Backdrop is the deepest bg so the
	// modal pops visually.
	return lipgloss.Place(width, height,
		lipgloss.Center, lipgloss.Center,
		card,
		lipgloss.WithWhitespaceChars(" "),
		lipgloss.WithWhitespaceForeground(theme.BGBase),
	)
}

// connectAttemptMsg is emitted by the modal when the user submits a
// key. The parent Model intercepts this, runs validation against the
// Ollama Cloud /api/tags endpoint, and emits a connectResultMsg.
type connectAttemptMsg struct {
	Key string
}

// connectResultMsg is emitted by the parent Model's validation
// goroutine when the /api/tags call returns. The modal uses this to
// either close (Err == nil) or show an inline error.
type connectResultMsg struct {
	Err error
}
