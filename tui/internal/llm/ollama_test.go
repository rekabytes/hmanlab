package llm

import (
	"context"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// TestStreamChatHappyPath serves a fake newline-delimited JSON stream
// and asserts the parsed events match what the cli-side parser would
// produce (cli/src/ollama.rs::stream_chat). Locks the wire format
// down so a future refactor can't silently drift.
func TestStreamChatHappyPath(t *testing.T) {
	const body = "" +
		`{"model":"glm-4.7","message":{"role":"assistant","content":"Hello"},"done":false}` + "\n" +
		`{"model":"glm-4.7","message":{"role":"assistant","content":", "},"done":false}` + "\n" +
		`{"model":"glm-4.7","message":{"role":"assistant","content":"world!"},"done":false}` + "\n" +
		`{"model":"glm-4.7","message":{"role":"assistant","content":""},"done":true,"prompt_eval_count":42,"eval_count":3}` + "\n"

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/api/chat" {
			t.Errorf("path = %q, want /api/chat", r.URL.Path)
		}
		if got := r.Header.Get("Authorization"); got != "Bearer test-key" {
			t.Errorf("auth header = %q, want Bearer test-key", got)
		}
		fmt.Fprint(w, body)
	}))
	defer srv.Close()

	// Override the provider's base URL by constructing manually with a
	// tweaked base. NewCloudOllama reads OllamaCloudBase; we bypass
	// that here by reaching into the struct.
	c := &CloudOllama{
		apiKey: "test-key",
		base:   srv.URL,
		http:   srv.Client(),
	}

	ch := c.StreamChat(context.Background(), "glm-4.7", []Message{
		{Role: RoleUser, Content: "hi"},
	})
	var text strings.Builder
	var done StreamEvent
	for ev := range ch {
		if ev.Err != nil {
			t.Fatalf("unexpected error: %v", ev.Err)
		}
		if ev.Done {
			done = ev
			break
		}
		text.WriteString(ev.Text)
	}
	if got := text.String(); got != "Hello, world!" {
		t.Errorf("accumulated text = %q, want %q", got, "Hello, world!")
	}
	if !done.Done {
		t.Error("no Done event received")
	}
	if done.PromptTokens != 42 || done.CompletionTokens != 3 {
		t.Errorf("usage = (%d, %d), want (42, 3)", done.PromptTokens, done.CompletionTokens)
	}
}

// TestStreamChatAuthFailure asserts 401 surfaces as a clean auth error
// on the channel rather than a confusing parse failure mid-stream.
func TestStreamChatAuthFailure(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusUnauthorized)
		fmt.Fprint(w, `{"error":"invalid api key"}`)
	}))
	defer srv.Close()

	c := &CloudOllama{
		apiKey: "bad-key",
		base:   srv.URL,
		http:   srv.Client(),
	}
	ch := c.StreamChat(context.Background(), "glm-4.7", nil)
	var sawErr error
	var sawDone bool
	for ev := range ch {
		if ev.Err != nil {
			sawErr = ev.Err
		}
		if ev.Done {
			sawDone = true
		}
	}
	if sawErr == nil {
		t.Fatal("expected auth error, got nil")
	}
	if !strings.Contains(sawErr.Error(), "auth failed") {
		t.Errorf("error = %q, want it to mention 'auth failed'", sawErr)
	}
	if sawDone {
		t.Error("should not have emitted Done alongside an error")
	}
}

// TestStreamChatCancel verifies that cancelling ctx makes the
// goroutine return promptly. We run the request against a server that
// never responds; without cancellation the test would hang forever.
func TestStreamChatCancel(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		// Hold the connection open without writing anything.
		select {}
	}))
	defer srv.Close()

	c := &CloudOllama{
		apiKey: "test-key",
		base:   srv.URL,
		http:   srv.Client(),
	}
	ctx, cancel := context.WithCancel(context.Background())
	ch := c.StreamChat(ctx, "glm-4.7", nil)

	// Cancel immediately — the in-flight HTTP request should be aborted
	// and the channel should close.
	cancel()
	for ev := range ch {
		// Either an error (cancelled) or no events at all is acceptable.
		// The key invariant: the channel closes, no goroutine leaks.
		if ev.Err != nil && ev.Err != context.Canceled {
			t.Errorf("error = %v, want context.Canceled or nil", ev.Err)
		}
	}
}

// TestPingAuth verifies Ping surfaces auth failures cleanly.
func TestPingAuth(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusUnauthorized)
	}))
	defer srv.Close()

	c := &CloudOllama{
		apiKey: "bad-key",
		base:   srv.URL,
		http:   srv.Client(),
	}
	err := c.Ping(context.Background())
	if err == nil {
		t.Fatal("Ping should have failed on 401")
	}
	if !strings.Contains(err.Error(), "auth failed") {
		t.Errorf("error = %q, want 'auth failed'", err)
	}
}
