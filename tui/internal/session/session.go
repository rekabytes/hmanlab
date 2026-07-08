// Package session implements JSONL-based conversation persistence for the
// Go TUI. Each conversation is one append-only JSONL file under
// ~/.config/hmanlab/sessions/<project>/<uuid>.jsonl, grouped by the working
// directory's folder name so each project keeps its own session list.
//
// The TUI owns its own session store.
package session

import (
	"crypto/rand"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"time"
)

// RecordKind labels the type of a JSONL line.
type RecordKind string

const (
	KindMeta      RecordKind = "meta"
	KindUser      RecordKind = "user"
	KindAssistant RecordKind = "assistant"
	KindTool      RecordKind = "tool"
)

// Record is one line in a session JSONL file. Mirrors the shape the TUI
// writes/reads; unknown fields on disk are ignored on load.
type Record struct {
	Kind      RecordKind `json:"type"`
	SessionID string     `json:"session_id"`
	UUID      string     `json:"uuid"`
	Timestamp string     `json:"timestamp"`
	Content   string     `json:"content,omitempty"`
	Name      string     `json:"name,omitempty"`
	Model     string     `json:"model,omitempty"`
	Title     string     `json:"title,omitempty"`
}

// SessionSummary is one row in the /sessions picker.
type SessionSummary struct {
	SessionID string
	Title     string
	Model     string
	UpdatedAt time.Time
	Path      string
}

// Active tracks the currently open session so subsequent writes append to
// the same file. nil when no session is loaded yet (created lazily on the
// first user message).
type Active struct {
	SessionID string
	Path      string
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

// SessionsDir returns the base directory for all session files, honoring
// XDG_CONFIG_HOME.
func SessionsDir() (string, error) {
	if xdg := os.Getenv("XDG_CONFIG_HOME"); xdg != "" {
		return filepath.Join(xdg, "hmanlab", "sessions"), nil
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("resolve $HOME: %w", err)
	}
	return filepath.Join(home, ".config", "hmanlab", "sessions"), nil
}

// DirForCWD returns the session directory for a working directory. Groups
// sessions by the cwd's last folder name (e.g. ~/dev/myapp → "myapp") so
// each project keeps its own session list. Slashes/odd chars are scrubbed.
func DirForCWD(cwd string) (string, error) {
	base, err := SessionsDir()
	if err != nil {
		return "", err
	}
	project := sanitizeProject(filepath.Base(cwd))
	if project == "" || project == "." || project == string(os.PathSeparator) {
		project = "default"
	}
	return filepath.Join(base, project), nil
}

// PathFor returns the full JSONL path for a session in a given cwd.
func PathFor(cwd, sessionID string) (string, error) {
	dir, err := DirForCWD(cwd)
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, sessionID+".jsonl"), nil
}

// unsafeProjectChars matches anything that's not a safe filename char.
var unsafeProjectChars = regexp.MustCompile(`[^A-Za-z0-9._-]+`)

// sanitizeProject collapses unsafe chars in a folder name to a single
// dash so it's always a valid, readable directory component.
func sanitizeProject(name string) string {
	name = strings.TrimSpace(name)
	name = unsafeProjectChars.ReplaceAllString(name, "-")
	name = strings.Trim(name, "-")
	return name
}

// ---------------------------------------------------------------------------
// IDs + timestamps
// ---------------------------------------------------------------------------

// NewID generates a cryptographically random UUID v4.
func NewID() string {
	var b [16]byte
	if _, err := rand.Read(b[:]); err != nil {
		// crypto/rand should never fail on a healthy system; fall back to
		// time-based so the app doesn't crash.
		panic("crypto/rand: " + err.Error())
	}
	b[6] = (b[6] & 0x0f) | 0x40 // version 4
	b[8] = (b[8] & 0x3f) | 0x80 // variant 10
	return fmt.Sprintf("%x-%x-%x-%x-%x", b[0:4], b[4:6], b[6:8], b[8:10], b[10:16])
}

func nowISO() string {
	return time.Now().UTC().Format("2006-01-02T15:04:05Z")
}

// TruncateTitle derives a display title from the first user message: first
// line only, trimmed, capped at 60 chars with a trailing ellipsis.
func TruncateTitle(s string) string {
	firstLine := s
	if i := strings.IndexByte(s, '\n'); i >= 0 {
		firstLine = s[:i]
	}
	firstLine = strings.TrimSpace(firstLine)
	if len([]rune(firstLine)) <= 60 {
		return firstLine
	}
	r := []rune(firstLine)
	return string(r[:59]) + "…"
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

func appendRecord(path string, rec Record) error {
	if dir := filepath.Dir(path); dir != "" {
		if err := os.MkdirAll(dir, 0o755); err != nil {
			return fmt.Errorf("mkdir %s: %w", dir, err)
		}
	}
	f, err := os.OpenFile(path, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o644)
	if err != nil {
		return fmt.Errorf("open %s: %w", path, err)
	}
	defer f.Close()
	line, err := json.Marshal(rec)
	if err != nil {
		return fmt.Errorf("encode record: %w", err)
	}
	line = append(line, '\n')
	if _, err := f.Write(line); err != nil {
		return fmt.Errorf("write %s: %w", path, err)
	}
	return nil
}

// WriteMeta creates the file (if needed) and writes the opening meta record.
// Call once per session, on first user message.
func WriteMeta(path, sessionID, title, model string) error {
	return appendRecord(path, Record{
		Kind:      KindMeta,
		SessionID: sessionID,
		UUID:      NewID(),
		Timestamp: nowISO(),
		Model:     model,
		Title:     title,
	})
}

// WriteUser appends a user message.
func WriteUser(path, sessionID, content, model string) error {
	return appendRecord(path, Record{
		Kind:      KindUser,
		SessionID: sessionID,
		UUID:      NewID(),
		Timestamp: nowISO(),
		Content:   content,
		Model:     model,
	})
}

// WriteAssistant appends a completed assistant reply.
func WriteAssistant(path, sessionID, content, model string) error {
	return appendRecord(path, Record{
		Kind:      KindAssistant,
		SessionID: sessionID,
		UUID:      NewID(),
		Timestamp: nowISO(),
		Content:   content,
		Model:     model,
	})
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

// LoadRecords reads every line of a session file, skipping blank/malformed
// lines (matching the CLI's lenient load behavior).
func LoadRecords(path string) ([]Record, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read %s: %w", path, err)
	}
	var records []Record
	for _, line := range strings.Split(string(data), "\n") {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		var rec Record
		if err := json.Unmarshal([]byte(line), &rec); err != nil {
			continue // skip malformed
		}
		records = append(records, rec)
	}
	return records, nil
}

// Message is a chat turn extracted from session records, ready to replay
// into the LLM request.
type Message struct {
	Role    string // "user" | "assistant"
	Content string
}

// RecordsToMessages filters meta records and converts user/assistant turns
// into a Message slice for chat-history replay.
func RecordsToMessages(records []Record) []Message {
	var msgs []Message
	for _, r := range records {
		switch r.Kind {
		case KindUser:
			msgs = append(msgs, Message{Role: "user", Content: r.Content})
		case KindAssistant:
			msgs = append(msgs, Message{Role: "assistant", Content: r.Content})
		}
	}
	return msgs
}

// ---------------------------------------------------------------------------
// Listing + lookup
// ---------------------------------------------------------------------------

// ListSessions scans the cwd's session directory and returns summaries,
// newest first.
func ListSessions(cwd string) ([]SessionSummary, error) {
	dir, err := DirForCWD(cwd)
	if err != nil {
		return nil, err
	}
	entries, err := os.ReadDir(dir)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return nil, nil
		}
		return nil, fmt.Errorf("read %s: %w", dir, err)
	}
	var summaries []SessionSummary
	for _, entry := range entries {
		if entry.IsDir() || !strings.HasSuffix(entry.Name(), ".jsonl") {
			continue
		}
		sessionID := strings.TrimSuffix(entry.Name(), ".jsonl")
		if sessionID == "" {
			continue
		}
		path := filepath.Join(dir, entry.Name())
		info, err := entry.Info()
		var updatedAt time.Time
		if err == nil {
			updatedAt = info.ModTime()
		}
		title, model := readSummary(path)
		summaries = append(summaries, SessionSummary{
			SessionID: sessionID,
			Title:     title,
			Model:     model,
			UpdatedAt: updatedAt,
			Path:      path,
		})
	}
	sort.Slice(summaries, func(i, j int) bool {
		return summaries[i].UpdatedAt.After(summaries[j].UpdatedAt)
	})
	return summaries, nil
}

// readSummary reads the first ~20 lines for title + model, mirroring the
// CLI's read_summary.
func readSummary(path string) (title, model string) {
	f, err := os.Open(path)
	if err != nil {
		return "(unreadable)", ""
	}
	defer f.Close()
	dec := json.NewDecoder(f)
	for i := 0; i < 20; i++ {
		var rec Record
		if err := dec.Decode(&rec); err != nil {
			break
		}
		if rec.Kind == KindMeta {
			if rec.Title != "" {
				title = rec.Title
			}
			if model == "" && rec.Model != "" {
				model = rec.Model
			}
		}
		if rec.Kind == KindUser && title == "" {
			title = TruncateTitle(rec.Content)
		}
		if title != "" && model != "" {
			break
		}
	}
	if title == "" {
		title = "(untitled)"
	}
	return title, model
}

// FindByPrefix resolves a ≥4-hex-char prefix to a unique session.
// Returns an error on 0 or 2+ matches.
func FindByPrefix(summaries []SessionSummary, prefix string) (*SessionSummary, error) {
	clean := strings.ToLower(strings.NewReplacer("-", "").Replace(prefix))
	var hexOnly strings.Builder
	for _, c := range clean {
		if strings.ContainsRune("0123456789abcdef", c) {
			hexOnly.WriteRune(c)
		}
	}
	clean = hexOnly.String()
	if len(clean) < 4 {
		return nil, fmt.Errorf("prefix too short (need ≥4 hex chars)")
	}
	var matches []*SessionSummary
	for i := range summaries {
		id := strings.ToLower(strings.ReplaceAll(summaries[i].SessionID, "-", ""))
		if strings.HasPrefix(id, clean) {
			matches = append(matches, &summaries[i])
		}
	}
	switch len(matches) {
	case 0:
		return nil, fmt.Errorf("no session matches '%s'", prefix)
	case 1:
		return matches[0], nil
	default:
		return nil, fmt.Errorf("%d sessions match '%s' — use a longer prefix", len(matches), prefix)
	}
}
