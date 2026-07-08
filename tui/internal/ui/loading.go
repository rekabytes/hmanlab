package ui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/lipgloss"

	"github.com/hmanlab/hmanlab/tui/internal/ui/theme"
)

// loadingMaxTicks is how many ticks the loading screen lasts before
// transitioning. At 120ms/tick this is ~600ms — enough for terminal
// OSC responses to arrive and be swallowed.
const loadingMaxTicks = 5

// viewLoading renders the startup loading screen: a centered spinning
// hibiscus flower + wordmark with a breathing color effect. All key
// events are swallowed during this phase so terminal OSC responses
// can't leak into the textarea.
func (m Model) viewLoading() string {
	if m.width == 0 || m.height == 0 {
		return "loading…"
	}

	// Spinning flower frames.
	frames := []string{"✿", "❋", "✾", "❀"}
	spin := frames[int(m.animTick/2)%len(frames)]

	// Breathing color between Hibiscus and HibiscusGlow.
	breath := breatheHibiscus(m.animTick)

	flower := lipgloss.NewStyle().
		Foreground(breath).
		Render(spin)

	wordmark := lipgloss.NewStyle().
		Foreground(breath).
		Bold(true).
		Render("hibiscus")

	// Animated dots: "loading" → "loading." → "loading.." → "loading…"
	dots := strings.Repeat(".", int(m.animTick/3)%4)
	loading := lipgloss.NewStyle().
		Foreground(theme.FGDim).
		Italic(true).
		Render(fmt.Sprintf("loading%s", dots))

	content := lipgloss.JoinVertical(lipgloss.Center,
		flower,
		"",
		wordmark,
		loading,
	)

	return lipgloss.Place(m.width, m.height,
		lipgloss.Center, lipgloss.Center,
		content,
		lipgloss.WithWhitespaceChars(" "),
		lipgloss.WithWhitespaceForeground(theme.BGBase),
	)
}
