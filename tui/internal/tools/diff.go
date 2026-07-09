package tools

// DiffLineKind labels a line in a unified-diff preview.
type DiffLineKind int

const (
	DiffContext DiffLineKind = iota
	DiffAdded
	DiffRemoved
	DiffSummary
)

// DiffLine is one line of a coloured diff preview shown in the
// confirmation card before a mutating tool runs.
type DiffLine struct {
	Kind DiffLineKind
	Text string
}

// String returns the text with a prefix suitable for rendering:
// "+", "-", or " " for context.
func (d DiffLine) Prefix() string {
	switch d.Kind {
	case DiffAdded:
		return "+"
	case DiffRemoved:
		return "-"
	default:
		return " "
	}
}

// BuildSimpleDiff creates a diff from an old and new string. Used by
// edit_file to show what changed.
func BuildSimpleDiff(oldText, newText string) []DiffLine {
	var lines []DiffLine
	oldLines := splitLines(oldText)
	newLines := splitLines(newText)

	lines = append(lines, DiffLine{Kind: DiffSummary, Text: "Changes:"})

	// Simple approach: show removed lines then added lines.
	for _, l := range oldLines {
		lines = append(lines, DiffLine{Kind: DiffRemoved, Text: l})
	}
	for _, l := range newLines {
		lines = append(lines, DiffLine{Kind: DiffAdded, Text: l})
	}
	return lines
}

func splitLines(s string) []string {
	var out []string
	start := 0
	for i := 0; i < len(s); i++ {
		if s[i] == '\n' {
			out = append(out, s[start:i])
			start = i + 1
		}
	}
	if start < len(s) {
		out = append(out, s[start:])
	}
	if len(out) == 0 {
		out = []string{""}
	}
	return out
}
