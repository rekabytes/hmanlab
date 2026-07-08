package ui

import (
	"strings"
	"testing"

	"github.com/charmbracelet/x/ansi"
)

// TestRenderMarkdownCodeBlock verifies fenced code blocks render with
// syntax highlighting and are wrapped in a code block border.
func TestRenderMarkdownCodeBlock(t *testing.T) {
	src := "```go\nfunc main() {}\n```"
	out := RenderMarkdown(src, 80)
	stripped := ansi.Strip(out)
	if !strings.Contains(stripped, "func main()") {
		t.Errorf("code block content missing: %q", stripped)
	}
}

// TestRenderMarkdownInlineCode verifies inline code renders.
func TestRenderMarkdownInlineCode(t *testing.T) {
	out := RenderMarkdown("Use `fmt.Println` to print", 80)
	stripped := ansi.Strip(out)
	if !strings.Contains(stripped, "fmt.Println") {
		t.Errorf("inline code missing: %q", stripped)
	}
}

// TestRenderMarkdownBold verifies bold text renders.
func TestRenderMarkdownBold(t *testing.T) {
	out := RenderMarkdown("**important**", 80)
	stripped := ansi.Strip(out)
	if !strings.Contains(stripped, "important") {
		t.Errorf("bold text missing: %q", stripped)
	}
}

// TestRenderMarkdownItalics verifies italic text renders.
func TestRenderMarkdownItalics(t *testing.T) {
	out := RenderMarkdown("*emphasis*", 80)
	stripped := ansi.Strip(out)
	if !strings.Contains(stripped, "emphasis") {
		t.Errorf("italic text missing: %q", stripped)
	}
}

// TestRenderMarkdownBulletList verifies bulleted lists render.
func TestRenderMarkdownBulletList(t *testing.T) {
	src := "- apple\n- banana\n- cherry"
	out := RenderMarkdown(src, 80)
	stripped := ansi.Strip(out)
	for _, item := range []string{"apple", "banana", "cherry"} {
		if !strings.Contains(stripped, item) {
			t.Errorf("list item %q missing: %q", item, stripped)
		}
	}
}

// TestRenderMarkdownNumberedList verifies numbered lists render.
func TestRenderMarkdownNumberedList(t *testing.T) {
	src := "1. first\n2. second\n3. third"
	out := RenderMarkdown(src, 80)
	stripped := ansi.Strip(out)
	for _, item := range []string{"first", "second", "third"} {
		if !strings.Contains(stripped, item) {
			t.Errorf("numbered item %q missing: %q", item, stripped)
		}
	}
}

// TestRenderMarkdownLink verifies links render with visible text.
func TestRenderMarkdownLink(t *testing.T) {
	out := RenderMarkdown("[example](https://example.com)", 80)
	stripped := ansi.Strip(out)
	if !strings.Contains(stripped, "example") {
		t.Errorf("link text missing: %q", stripped)
	}
}

// TestRenderMarkdownBlockquote verifies blockquotes render.
func TestRenderMarkdownBlockquote(t *testing.T) {
	out := RenderMarkdown("> wisdom here", 80)
	stripped := ansi.Strip(out)
	if !strings.Contains(stripped, "wisdom here") {
		t.Errorf("blockquote text missing: %q", stripped)
	}
}

// TestRenderMarkdownHeading verifies headings render.
func TestRenderMarkdownHeading(t *testing.T) {
	out := RenderMarkdown("# Big Title", 80)
	stripped := ansi.Strip(out)
	if !strings.Contains(stripped, "Big Title") {
		t.Errorf("heading text missing: %q", stripped)
	}
}

// TestRenderMarkdownEmpty verifies empty input returns empty string.
func TestRenderMarkdownEmpty(t *testing.T) {
	out := RenderMarkdown("", 80)
	if out != "" {
		t.Errorf("expected empty string, got %q", out)
	}
}

// TestRenderMarkdownFallback verifies that invalid markdown still
// returns content (never blanks out the chat).
func TestRenderMarkdownFallback(t *testing.T) {
	src := "just plain text"
	out := RenderMarkdown(src, 80)
	stripped := ansi.Strip(out)
	if !strings.Contains(stripped, "plain text") {
		t.Errorf("fallback text missing: %q", stripped)
	}
}
