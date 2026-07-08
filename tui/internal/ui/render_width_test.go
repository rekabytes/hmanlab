package ui

import (
	"strings"
	"testing"

	"github.com/charmbracelet/lipgloss"

	"github.com/hmanlab/hmanlab/tui/internal/config"
)

// TestViewChatFullRowWidth is a regression guard for a bug we fixed:
// bubbles/textarea only paints its own background on cells it actually
// outputs. When the textarea was empty, the wrapper's BG_CHAT only
// extended as far as the rendered content (~1 col wide for just the
// placeholder), so the rest of the input row fell through to whatever
// was rendered beneath. The same issue bit the header + status bar.
//
// Fix: each wrapper that paints a horizontal bg uses lipgloss
// `.Width(m.width)` to force the rendered block to span the full row.
//
// This test renders `viewChat()` into a string and asserts every row
// equals exactly `width` cols. If a future refactor drops the `.Width()`
// on any of the three wrappers, this fails.
func TestViewChatFullRowWidth(t *testing.T) {
	const width = 80
	cfg, _ := config.Load()
	cfg.OllamaCloudAPIKey = "sk-test" // skip connect modal, boot straight into chat

	m := New(cfg, "/test/project")
	// Skip the loading screen — boot straight into chat for the test.
	m.finishLoading()
	// Set terminal dimensions without going through the WindowSizeMsg
	// path — relayout + refreshViewportContent pick them up.
	m.width = width
	m.height = 24
	m.relayout()
	m.refreshViewportContent()

	out := m.View()

	lines := strings.Split(out, "\n")
	if len(lines) == 0 {
		t.Fatal("empty render")
	}
	for i, row := range lines {
		w := lipgloss.Width(row)
		if w != width {
			t.Errorf("row %d width = %d, want %d\n  %q", i, w, width, row)
		}
	}
}