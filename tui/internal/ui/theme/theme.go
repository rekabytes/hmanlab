// Package theme holds the palette for the hibiscus TUI.
//
// The base surfaces + role-marker colors are Catppuccin Mocha (matching
// cli/src/ui/theme.rs) so chat content looks identical between the two
// clients. The brand accent is **hibiscus pink** — a vibrant tropical
// pink-red (#ff5d8f) that gives the TUI its own identity separate from
// the cli's peach accent. Three hibiscus tones:
//
//   Hibiscus      #ff5d8f  — primary accent (borders, badges, wordmark)
//   HibiscusGlow  #ff8fb1  — lighter, for "glow" emphasis (active dots)
//   HibiscusDim   #c43658  — darker, for backgrounds / pressed states
package theme

import "github.com/charmbracelet/lipgloss"

// Base palette — Catppuccin Mocha surfaces + role markers.
// Values mirror cli/src/ui/theme.rs color-for-color so chat content
// looks identical between the two clients.
var (
	// Hibiscus brand accent — the TUI's identity color. Used for
	// borders on the input box + connect modal, the "● connected"
	// status dot, the wordmark in the header, and the streaming
	// indicator. Three tones for visual hierarchy.
	Hibiscus     = lipgloss.Color("#ff5d8f") // primary — vibrant tropical pink
	HibiscusGlow = lipgloss.Color("#ff8fb1") // lighter — for glow / emphasis
	HibiscusDim  = lipgloss.Color("#c43658") // darker — for backgrounds, "pressed"

	AccentAlt = lipgloss.Color("#cba6f7") // mauve (secondary accent)

	// Role markers — kept identical to the cli so chat content reads
	// the same in both clients. These are semantic (User=green,
	// Assistant=sky) and intentionally NOT hibiscus-tinted.
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

	// Borders — hibiscus when active (matches brand), dim otherwise.
	BorderActive = Hibiscus
	BorderIdle   = FGDimmer

	// Status / diff
	Success = User
	Error   = ToolError
	Warning = lipgloss.Color("#f9e2af") // yellow

	DiffAdd = lipgloss.Color("#a6e3a1")
	DiffRm  = lipgloss.Color("#f38ba8")

	// Semantic state colors. Tooling/Thinking now use the hibiscus
	// brand accent so the "agent is doing something" state matches
	// the rest of the TUI's identity.
	StateIdle       = FGDim
	StateThinking   = AccentAlt
	StateTooling    = Hibiscus
	StateCompacting = lipgloss.Color("#89b4fa")

	PermissionInfo        = Warning
	PermissionDestructive = ToolError

	// Tab + todo
	TabActive   = Hibiscus
	TabInactive = FGDim
	TodoPending = lipgloss.Color("#a6adc8")
	TodoActive  = Hibiscus
	TodoDone    = User
)

// RoleLabelStyle returns the lipgloss style for a chat message's
// role label. Mirrors cli/src/ui/theme.rs::role_label — role colors
// are NOT hibiscus-tinted, they stay semantic so chat content reads
// the same in both clients.
func RoleLabelStyle(role string) lipgloss.Style {
	switch role {
	case "user":
		return lipgloss.NewStyle().Foreground(User).Bold(true)
	case "assistant":
		return lipgloss.NewStyle().Foreground(Hibiscus).Bold(true)
	case "info", "summary":
		return lipgloss.NewStyle().Foreground(System).Bold(true)
	case "tool":
		return lipgloss.NewStyle().Foreground(Tool).Bold(true)
	default:
		return lipgloss.NewStyle().Foreground(FGDim).Bold(true)
	}
}

// GutterStyle is the colored vertical bar that sits under each
// speaker label.
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
