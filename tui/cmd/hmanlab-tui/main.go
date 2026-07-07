// Command hmanlab-tui is the Go + Bubble Tea client for hmanlab.
// v0 (current) is a walking skeleton: streaming chat with Ollama Cloud,
// markdown rendering, slash commands, first-run connect modal. See
// tui/docs/development/PRD.md for the v0 scope + v0.1+ roadmap.
package main

import (
	"fmt"
	"os"

	tea "github.com/charmbracelet/bubbletea"

	"github.com/hmanlab/hmanlab/tui/internal/config"
	"github.com/hmanlab/hmanlab/tui/internal/ui"
)

func main() {
	cfg, err := config.Load()
	if err != nil {
		fmt.Fprintf(os.Stderr, "hmanlab-tui: load config: %v\n", err)
		os.Exit(1)
	}

	// Bubble Tea program. Mouse-motion is enabled so we can add hover
	// affordances later; for v0 only wheel-scroll routes through it.
	p := tea.NewProgram(
		ui.New(cfg),
		tea.WithAltScreen(),
		tea.WithMouseCellMotion(),
		tea.WithFPS(30), // smooth streaming caret + viewport scroll
	)

	if _, err := p.Run(); err != nil {
		fmt.Fprintf(os.Stderr, "hmanlab-tui: %v\n", err)
		os.Exit(1)
	}
}
