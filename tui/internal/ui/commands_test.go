package ui

import "testing"

// TestParseSlashCommand covers every accepted form plus the
// not-a-command case. Mirrors the cases cli/src/app/commands/mod.rs
// tests cover (parse_command + aliases).
func TestParseSlashCommand(t *testing.T) {
	cases := []struct {
		in     string
		ok     bool
		kind   string
		arg    string
	}{
		{"hello", false, "", ""},
		{"/help", true, "help", ""},
		{"/?", true, "help", ""},
		{"/HELP", true, "help", ""}, // case-insensitive head
		{"/clear", true, "clear", ""},
		{"/quit", true, "quit", ""},
		{"/exit", true, "quit", ""}, // alias
		{"/q", true, "quit", ""},    // alias
		{"/model", true, "model", ""},
		{"/model glm-4.7", true, "model", "glm-4.7"},
		{"  /model   glm-5  ", true, "model", "glm-5"}, // whitespace trimmed
		{"/bogus", true, "unknown", "bogus"},
	}
	for _, c := range cases {
		cmd, ok := parseSlashCommand(c.in)
		if ok != c.ok {
			t.Errorf("parseSlashCommand(%q): ok = %v, want %v", c.in, ok, c.ok)
			continue
		}
		if ok {
			if cmd.kind != c.kind {
				t.Errorf("parseSlashCommand(%q): kind = %q, want %q", c.in, cmd.kind, c.kind)
			}
			if cmd.arg != c.arg {
				t.Errorf("parseSlashCommand(%q): arg = %q, want %q", c.in, cmd.arg, c.arg)
			}
		}
	}
}

// TestHelpTextMentionsCommands is a smoke test that /help output
// mentions every slash command + the current model. Cheap guard
// against drift if someone adds a command but forgets the help text.
func TestHelpTextMentionsCommands(t *testing.T) {
	out := helpText("glm-4.7")
	for _, want := range []string{"glm-4.7", "/help", "/clear", "/quit", "/model", "Ollama Cloud", "Hibiscus"} {
		if !contains(out, want) {
			t.Errorf("helpText missing %q in output:\n%s", want, out)
		}
	}
}

func contains(haystack, needle string) bool {
	return len(haystack) >= len(needle) && (haystack == needle ||
		(len(haystack) > 0 && len(needle) > 0 &&
			indexOf(haystack, needle) >= 0))
}

func indexOf(s, sub string) int {
	for i := 0; i+len(sub) <= len(s); i++ {
		if s[i:i+len(sub)] == sub {
			return i
		}
	}
	return -1
}
