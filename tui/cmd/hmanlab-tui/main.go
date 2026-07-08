// Command hmanlab-tui is the Go + Bubble Tea client for hmanlab.
// v0 (current) is a walking skeleton: streaming chat with Ollama Cloud,
// markdown rendering, slash commands, first-run connect modal. See
// tui/docs/development/PRD.md for the v0 scope + v0.1+ roadmap.
package main

import (
	"fmt"
	"os"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"

	"github.com/hmanlab/hmanlab/tui/internal/config"
	"github.com/hmanlab/hmanlab/tui/internal/ui"
)

func main() {
	// Prevent lipgloss/termenv from sending an OSC 11 background-color
	// query to the terminal. The query's response leaks a stray "\"
	// (the ST terminator's backslash) into Bubble Tea's input reader,
	// which lands in the textarea. Setting this explicitly skips the
	// query entirely — no response, no leak.
	lipgloss.SetHasDarkBackground(true)

	cfg, err := config.Load()
	if err != nil {
		fmt.Fprintf(os.Stderr, "hmanlab-tui: load config: %v\n", err)
		os.Exit(1)
	}

	cwd, err := os.Getwd()
	if err != nil {
		fmt.Fprintf(os.Stderr, "hmanlab-tui: get cwd: %v\n", err)
		os.Exit(1)
	}

	// Bubble Tea program. Mouse-motion is enabled so we can add hover
	// affordances later; for v0 only wheel-scroll routes through it.
	p := tea.NewProgram(
		ui.New(cfg, cwd),
		tea.WithAltScreen(),
		tea.WithMouseCellMotion(),
		tea.WithFPS(30), // smooth streaming caret + viewport scroll
	)

	if _, err := p.Run(); err != nil {
		fmt.Fprintf(os.Stderr, "hmanlab-tui: %v\n", err)
		os.Exit(1)
	}
}
