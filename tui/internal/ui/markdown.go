package ui

import (
	"strings"
	"sync"

	"github.com/charmbracelet/glamour"
	"github.com/charmbracelet/lipgloss"
)

// markdownRenderer is a thin wrapper around glamour that auto-selects
// the right width + theme for the current terminal. Glamour itself
// wraps chroma for syntax highlighting, so we get code-block colors for
// free — matches cli/src/ui/markdown.rs's target parity.
//
// Glamour renderers carry mutable state (auto-detect buffers) so they
// aren't safe to share across goroutines. We guard with a mutex.
type markdownRenderer struct {
	mu       sync.Mutex
	renderer glamour.TermRenderer
	width    int
}

var md markdownRenderer

// reinit rebuilds the renderer if the width has changed. Called lazily
// on every Render so a window resize picks up automatically without a
// separate Resize handler.
func reinit(width int) error {
	if width < 20 {
		width = 20
	}
	if md.width == width {
		return nil
	}
	r, err := glamour.NewTermRenderer(
		glamour.WithAutoStyle(),
		// Dark background → catppuccin-ish dark theme. Matches cli's
		// BG_BASE palette.
		glamour.WithEnvironmentConfig(),
		glamour.WithWordWrap(width),
	)
	if err != nil {
		return err
	}
	md.renderer = *r
	md.width = width
	return nil
}

// RenderMarkdown renders src (CommonMark + GFM) into a string of
// ANSI-styled terminal output. Width is the target wrap column for
// both prose and code blocks. Returns the raw src on render error so
// the chat never blanks out — we'd rather show unformatted text than
// nothing.
func RenderMarkdown(src string, width int) string {
	src = strings.TrimRight(src, "\n")
	if src == "" {
		return ""
	}
	if err := reinit(width); err != nil {
		return src
	}
	md.mu.Lock()
	defer md.mu.Unlock()
	out, err := md.renderer.Render(src)
	if err != nil {
		return src
	}
	// Glamour pads each line with trailing whitespace + a leading
	// indent. Strip the indent so the rendered block aligns with our
	// chat column; we manage our own indentation.
	lines := strings.Split(out, "\n")
	for i, l := range lines {
		lines[i] = strings.TrimRight(l, " \t")
	}
	return strings.Join(lines, "\n")
}

// StyleToken is a tiny helper that wraps `s` in a foreground color for
// embedding in styled strings. lipgloss already does this but the call
// site is more compact.
func StyleToken(s string, color lipgloss.Color) string {
	return lipgloss.NewStyle().Foreground(color).Render(s)
}
