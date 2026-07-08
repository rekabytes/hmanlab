package session

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// overrideHome redirects SessionsDir into a temp dir for the test.
func overrideHome(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)
	return dir
}

func TestDirForCWDGroupsByProjectName(t *testing.T) {
	overrideHome(t)
	a, err := DirForCWD("/home/alice/myapp")
	if err != nil {
		t.Fatal(err)
	}
	b, err := DirForCWD("/home/alice/myapp")
	if err != nil {
		t.Fatal(err)
	}
	if a != b {
		t.Errorf("same cwd should map to same dir: %q vs %q", a, b)
	}
	other, _ := DirForCWD("/home/alice/other")
	if a == other {
		t.Errorf("different projects should map to different dirs")
	}
	if got := filepath.Base(a); got != "myapp" {
		t.Errorf("leaf = %q, want 'myapp'", got)
	}
}

func TestDirForCWDSanitizesNames(t *testing.T) {
	overrideHome(t)
	// Spaces and odd chars get collapsed to dashes.
	a, _ := DirForCWD("/home/alice/My Cool Project!")
	if got := filepath.Base(a); got != "My-Cool-Project" {
		t.Errorf("sanitized = %q, want 'My-Cool-Project'", got)
	}
	// Root path falls back to "default".
	b, _ := DirForCWD("/")
	if got := filepath.Base(b); got != "default" {
		t.Errorf("root leaf = %q, want 'default'", got)
	}
}

func TestNewIDIsUUIDv4Shaped(t *testing.T) {
	for i := 0; i < 100; i++ {
		id := NewID()
		// xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx
		if len(id) != 36 {
			t.Fatalf("id len = %d, want 36: %q", len(id), id)
		}
		if id[14] != '4' {
			t.Errorf("id %q: version nibble = %c, want '4'", id, id[14])
		}
		if id[8] != '-' || id[13] != '-' || id[18] != '-' || id[23] != '-' {
			t.Errorf("id %q: dash positions wrong", id)
		}
		// variant bits: first char of 4th group is 8/9/a/b
		c := id[19]
		if c != '8' && c != '9' && c != 'a' && c != 'b' {
			t.Errorf("id %q: variant nibble = %c, want 8/9/a/b", id, c)
		}
	}
}

func TestNewIDUniqueness(t *testing.T) {
	seen := make(map[string]bool, 1000)
	for i := 0; i < 1000; i++ {
		id := NewID()
		if seen[id] {
			t.Fatalf("collision on %q after %d ids", id, i)
		}
		seen[id] = true
	}
}

func TestTruncateTitle(t *testing.T) {
	cases := []struct {
		in   string
		want string
	}{
		{"hello", "hello"},
		{"  spaced  ", "spaced"},
		{"first line\nsecond line", "first line"},
		{strings.Repeat("a", 60), strings.Repeat("a", 60)},
		{strings.Repeat("a", 61), strings.Repeat("a", 59) + "…"},
		{strings.Repeat("あ", 70), strings.Repeat("あ", 59) + "…"}, // rune-safe
	}
	for _, c := range cases {
		got := TruncateTitle(c.in)
		if got != c.want {
			t.Errorf("TruncateTitle(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}

func TestRoundTrip(t *testing.T) {
	overrideHome(t)
	path, _ := PathFor("/work", "deadbeef-1111-4222-8333-444455556666")

	if err := WriteMeta(path, "deadbeef-1111-4222-8333-444455556666", "hello world", "glm-5"); err != nil {
		t.Fatal(err)
	}
	if err := WriteUser(path, "deadbeef-1111-4222-8333-444455556666", "hi there", "glm-5"); err != nil {
		t.Fatal(err)
	}
	if err := WriteAssistant(path, "deadbeef-1111-4222-8333-444455556666", "hello back", "glm-5"); err != nil {
		t.Fatal(err)
	}

	records, err := LoadRecords(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(records) != 3 {
		t.Fatalf("got %d records, want 3", len(records))
	}
	if records[0].Kind != KindMeta {
		t.Errorf("record 0 kind = %q, want meta", records[0].Kind)
	}
	if records[0].Title != "hello world" {
		t.Errorf("meta title = %q", records[0].Title)
	}
	if records[1].Kind != KindUser || records[1].Content != "hi there" {
		t.Errorf("record 1 = %+v", records[1])
	}
	if records[2].Kind != KindAssistant || records[2].Content != "hello back" {
		t.Errorf("record 2 = %+v", records[2])
	}
	// All records share the session id.
	for _, r := range records {
		if r.SessionID != "deadbeef-1111-4222-8333-444455556666" {
			t.Errorf("record session_id = %q", r.SessionID)
		}
		if r.UUID == "" {
			t.Errorf("record %q has empty uuid", r.Kind)
		}
	}
}

func TestRecordsToMessages(t *testing.T) {
	records := []Record{
		{Kind: KindMeta, Content: ""},
		{Kind: KindUser, Content: "q1"},
		{Kind: KindAssistant, Content: "a1"},
		{Kind: KindUser, Content: "q2"},
	}
	msgs := RecordsToMessages(records)
	if len(msgs) != 3 {
		t.Fatalf("got %d msgs, want 3", len(msgs))
	}
	if msgs[0].Role != "user" || msgs[0].Content != "q1" {
		t.Errorf("msg 0 = %+v", msgs[0])
	}
	if msgs[2].Content != "q2" {
		t.Errorf("msg 2 = %+v", msgs[2])
	}
}

func TestLoadRecordsSkipsMalformed(t *testing.T) {
	dir := overrideHome(t)
	path := filepath.Join(dir, "hmanlab", "sessions", "abc", "test.jsonl")
	os.MkdirAll(filepath.Dir(path), 0o755)
	// Write a mix of valid + garbage lines.
	content := `{"type":"meta","session_id":"s1","uuid":"u1","timestamp":"2026-01-01T00:00:00Z","title":"t"}
this is not json
{"type":"user","session_id":"s1","uuid":"u2","timestamp":"2026-01-01T00:00:01Z","content":"hello"}

{"broken json
`
	os.WriteFile(path, []byte(content), 0o644)

	records, err := LoadRecords(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(records) != 2 {
		t.Fatalf("got %d records, want 2 (skipping garbage)", len(records))
	}
}

func TestListSessionsSortedNewestFirst(t *testing.T) {
	overrideHome(t)
	cwd := "/proj"
	// Create three sessions with distinct mtimes.
	old := NewID()
	mid := NewID()
	new := NewID()
	for _, id := range []string{old, mid, new} {
		p, _ := PathFor(cwd, id)
		WriteMeta(p, id, "session "+id, "glm-5")
	}
	// Set mtimes: old=1h ago, mid=10m ago, new=now.
	oldPath, _ := PathFor(cwd, old)
	midPath, _ := PathFor(cwd, mid)
	newPath, _ := PathFor(cwd, new)
	now := time.Now()
	os.Chtimes(oldPath, now, now.Add(-1*time.Hour))
	os.Chtimes(midPath, now, now.Add(-10*time.Minute))
	os.Chtimes(newPath, now, now)

	summaries, err := ListSessions(cwd)
	if err != nil {
		t.Fatal(err)
	}
	if len(summaries) != 3 {
		t.Fatalf("got %d sessions, want 3", len(summaries))
	}
	if summaries[0].SessionID != new {
		t.Errorf("newest first: got %q, want %q", summaries[0].SessionID, new)
	}
	if summaries[2].SessionID != old {
		t.Errorf("oldest last: got %q, want %q", summaries[2].SessionID, old)
	}
}

func TestListSessionsEmptyDir(t *testing.T) {
	overrideHome(t)
	summaries, err := ListSessions("/never-used")
	if err != nil {
		t.Fatal(err)
	}
	if len(summaries) != 0 {
		t.Errorf("got %d sessions on empty, want 0", len(summaries))
	}
}

func TestFindByPrefix(t *testing.T) {
	summaries := []SessionSummary{
		{SessionID: "deadbeef-1111-4222-8333-444455556666"},
		{SessionID: "cafe1234-2222-4333-9444-555566667777"},
		{SessionID: "dead0000-3333-4444-a555-666677778888"},
	}
	// Unique match on "cafe".
	got, err := FindByPrefix(summaries, "cafe")
	if err != nil {
		t.Fatal(err)
	}
	if got.SessionID != "cafe1234-2222-4333-9444-555566667777" {
		t.Errorf("got %q", got.SessionID)
	}
	// Ambiguous: "dead" matches two.
	_, err = FindByPrefix(summaries, "dead")
	if err == nil {
		t.Error("expected ambiguity error for 'dead'")
	}
	// With dashes + longer prefix resolves uniquely.
	got, err = FindByPrefix(summaries, "deadbeef")
	if err != nil {
		t.Fatal(err)
	}
	if got.SessionID != "deadbeef-1111-4222-8333-444455556666" {
		t.Errorf("got %q", got.SessionID)
	}
	// Too short.
	_, err = FindByPrefix(summaries, "ab")
	if err == nil {
		t.Error("expected error for short prefix")
	}
	// No match.
	_, err = FindByPrefix(summaries, "zzzzzzzz")
	if err == nil {
		t.Error("expected no-match error")
	}
}

func TestReadSummaryFallback(t *testing.T) {
	overrideHome(t)
	path, _ := PathFor("/p", "aaaa1111-2222-4333-9444-555566667777")
	// No meta record, just a user message — title derived from content,
	// model stays empty (meta is the source of truth for model).
	WriteUser(path, "aaaa1111-2222-4333-9444-555566667777", "what is the meaning of life?", "glm-5")
	summaries, err := ListSessions("/p")
	if err != nil {
		t.Fatal(err)
	}
	if len(summaries) != 1 {
		t.Fatalf("got %d, want 1", len(summaries))
	}
	if summaries[0].Title != "what is the meaning of life?" {
		t.Errorf("title = %q", summaries[0].Title)
	}
	if summaries[0].Model != "" {
		t.Errorf("model = %q, want empty (no meta record)", summaries[0].Model)
	}
}
