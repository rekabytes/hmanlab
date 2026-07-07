// Package theme holds the Catppuccin Mocha palette the cli uses, as
// RGB literals. Mirrors cli/src/ui/theme.rs color-for-color so the two
// clients look identical when run side by side.
package theme

import "github.com/charmbracelet/lipgloss"

// Base palette — Catppuccin Mocha.
//
// Values copied from cli/src/ui/theme.rs. If a token gets tweaked on
// the cli side, mirror it here so the two clients stay visually in sync.
var (
	Accent    = lipgloss.Color("#fab387") // peach
	AccentDim = lipgloss.Color("#b4825f")
	AccentAlt = lipgloss.Color("#cba6f7") // mauve

	// Role markers
	User      = lipgloss.Color("#a6e3a1") // green
	Assistant = lipgloss.Color("#89dceb") // sky
	System    = lipgloss.Color("#b4befe") // lavender
	Tool      = lipgloss.Color("#89dceb") // sky
	ToolError = lipgloss.Color("#f38ba8") // red

	// Surfaces
	FG       = lipgloss.Color("#cdd6f4") // text
	FGDim    = lipgloss.Color("#6c7086") // overlay0
	FGDimmer = lipgloss.Color("#454759") // surface1

	BGBase = lipgloss.Color("#1c1c1c") // neutral dark grey
	BGChat = lipgloss.Color("#3c3c3c") // elevated input surface

	BGCard       = lipgloss.Color("#313244") // catppuccin surface0
	BGCardHover  = lipgloss.Color("#585b70") // catppuccin surface2
	BGInspector  = lipgloss.Color("#23243a")
	BGInspectorH = lipgloss.Color("#3c3e54")

	// Borders
	BorderActive = Accent
	BorderIdle   = FGDimmer

	// Status / diff
	Success = User
	Error   = ToolError
	Warning = lipgloss.Color("#f9e2af") // yellow

	DiffAdd = lipgloss.Color("#a6e3a1")
	DiffRm  = lipgloss.Color("#f38ba8")

	// Semantic
	StateIdle      = FGDim
	StateThinking  = AccentAlt
	StateTooling   = Accent
	StateCompacting = lipgloss.Color("#89b4fa")

	PermissionInfo        = Warning
	PermissionDestructive = ToolError

	// Tab + todo
	TabActive   = Accent
	TabInactive = FGDim
	TodoPending = lipgloss.Color("#a6adc8")
	TodoActive  = Accent
	TodoDone    = User
)

// RoleLabelStyle returns the lipgloss style for a chat message's
// role label. Mirrors cli/src/ui/theme.rs::role_label.
func RoleLabelStyle(role string) lipgloss.Style {
	switch role {
	case "user":
		return lipgloss.NewStyle().Foreground(User).Bold(true)
	case "assistant":
		return lipgloss.NewStyle().Foreground(Assistant).Bold(true)
	case "info", "summary":
		return lipgloss.NewStyle().Foreground(System).Bold(true)
	case "tool":
		return lipgloss.NewStyle().Foreground(Tool).Bold(true)
	default:
		return lipgloss.NewStyle().Foreground(FGDim).Bold(true)
	}
}

// GutterStyle is the colored vertical bar that sits under each
// speaker label, matching cli's `▎` glyph gutter.
func GutterStyle(role string) lipgloss.Style {
	c := FGDimmer
	switch role {
	case "user":
		c = User
	case "assistant":
		c = Assistant
	case "info", "summary":
		c = System
	case "tool":
		c = Tool
	}
	return lipgloss.NewStyle().Foreground(c)
}
