package llm

// Ollama Cloud provider implementation. Wire format mirrors
// cli/src/ollama.rs::stream_chat byte-for-byte — POST {base}/api/chat
// with Authorization: Bearer <key>, body is JSON, response is
// newline-delimited JSON chunks terminated by a `{done: true}` sentinel.

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strings"
	"time"

	"github.com/hmanlab/hmanlab/tui/internal/config"
)

// CloudOllama is the Ollama Cloud provider. Safe for concurrent use —
// the http.Client is goroutine-safe and the per-call state lives on
// the goroutine spawned by StreamChat.
type CloudOllama struct {
	apiKey string
	base   string
	http   *http.Client
}

// NewCloudOllama builds a client for the Ollama Cloud provider. The
// base URL defaults to config.OllamaCloudBase if empty.
func NewCloudOllama(apiKey string) *CloudOllama {
	return &CloudOllama{
		apiKey: strings.TrimSpace(apiKey),
		base:   strings.TrimRight(config.OllamaCloudBase, "/"),
		// Same timeout shape as the cli — generous for long generations,
		// tight enough to surface a hung connection. Per-request
		// timeouts are enforced via ctx instead.
		http: &http.Client{Timeout: 5 * time.Minute},
	}
}

// Name implements Provider.
func (c *CloudOllama) Name() string { return config.OllamaCloudProvider }

// Models implements Provider — the static cloud catalog.
func (c *CloudOllama) Models() []string {
	out := make([]string, len(config.OllamaCloudModels))
	copy(out, config.OllamaCloudModels)
	return out
}

// Ping validates the API key by calling /api/tags. Returns nil if the
// key is accepted, the upstream error otherwise. Used by the first-run
// connect modal to validate before persisting the key.
func (c *CloudOllama) Ping(ctx context.Context) error {
	url := c.base + "/api/tags"
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return fmt.Errorf("build request: %w", err)
	}
	req.Header.Set("Authorization", "Bearer "+c.apiKey)
	resp, err := c.http.Do(req)
	if err != nil {
		return fmt.Errorf("call /api/tags: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode == http.StatusUnauthorized || resp.StatusCode == http.StatusForbidden {
		return fmt.Errorf("auth failed (%d) — check the API key", resp.StatusCode)
	}
	if resp.StatusCode/100 != 2 {
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 512))
		return fmt.Errorf("/api/tags returned %d: %s", resp.StatusCode, strings.TrimSpace(string(body)))
	}
	return nil
}

// chatRequest is the JSON body for /api/chat. Mirrors
// cli/src/ollama.rs::ChatRequest. Tools/think default off at v0.
type chatRequest struct {
	Model    string    `json:"model"`
	Messages []Message `json:"messages"`
	Stream   bool      `json:"stream"`
	Think    bool      `json:"think"`
}

// chatChunk is one line of the newline-delimited streaming response.
// Only the fields we read at v0 are typed; the rest are ignored.
type chatChunk struct {
	Message struct {
		Content string `json:"content"`
	} `json:"message"`
	Done             bool `json:"done"`
	PromptEvalCount  int  `json:"prompt_eval_count,omitempty"`
	EvalCount        int  `json:"eval_count,omitempty"`
}

// StreamChat implements Provider. The returned channel receives
// StreamEvent values as the model produces them; the channel closes
// when the stream ends (Done) or errors out (Err). Cancelling ctx
// aborts the in-flight HTTP request promptly — no leaked goroutines.
func (c *CloudOllama) StreamChat(ctx context.Context, model string, messages []Message) <-chan StreamEvent {
	out := make(chan StreamEvent, 16)

	go func() {
		defer close(out)

		body, err := json.Marshal(chatRequest{
			Model:    model,
			Messages: messages,
			Stream:   true,
			Think:    false,
		})
		if err != nil {
			out <- StreamEvent{Err: fmt.Errorf("marshal request: %w", err)}
			return
		}

		url := c.base + "/api/chat"
		req, err := http.NewRequestWithContext(ctx, http.MethodPost, url, bytes.NewReader(body))
		if err != nil {
			out <- StreamEvent{Err: fmt.Errorf("build request: %w", err)}
			return
		}
		req.Header.Set("Authorization", "Bearer "+c.apiKey)
		req.Header.Set("Content-Type", "application/json")
		req.Header.Set("Accept", "application/x-ndjson")

		resp, err := c.http.Do(req)
		if err != nil {
			// If the context was cancelled, surface a clean cancellation
			// rather than a generic network error.
			if ctx.Err() != nil {
				out <- StreamEvent{Err: ctx.Err()}
			} else {
				out <- StreamEvent{Err: fmt.Errorf("POST /api/chat: %w", err)}
			}
			return
		}
		defer resp.Body.Close()

		if resp.StatusCode == http.StatusUnauthorized || resp.StatusCode == http.StatusForbidden {
			out <- StreamEvent{Err: fmt.Errorf("auth failed (%d) — check the API key", resp.StatusCode)}
			return
		}
		if resp.StatusCode/100 != 2 {
			rbody, _ := io.ReadAll(io.LimitReader(resp.Body, 512))
			out <- StreamEvent{Err: fmt.Errorf("/api/chat returned %d: %s", resp.StatusCode, strings.TrimSpace(string(rbody)))}
			return
		}

		// Newline-delimited JSON stream. Each non-empty line is one chunk.
		// A chunk with done=true is the terminal sentinel — emit Done and
		// return. Mirrors cli/src/ollama.rs::stream_chat's unfold loop.
		scanner := bufio.NewScanner(resp.Body)
		// Raise the per-line limit so a model that emits a large tool_call
		// payload (long after v0, but cheap insurance) doesn't trip the
		// default 64 KB cap.
		scanner.Buffer(make([]byte, 0, 64*1024), 1024*1024)
		for scanner.Scan() {
			line := strings.TrimSpace(scanner.Text())
			if line == "" {
				continue
			}
			var chunk chatChunk
			if err := json.Unmarshal([]byte(line), &chunk); err != nil {
				// Don't kill the stream on one bad chunk — log and keep going.
				// The cli does the same (cli/src/ollama.rs:430 "unparseable
				// chunk — skip").
				continue
			}
			if chunk.Done {
				out <- StreamEvent{
					Done:             true,
					PromptTokens:     chunk.PromptEvalCount,
					CompletionTokens: chunk.EvalCount,
				}
				return
			}
			if chunk.Message.Content != "" {
				out <- StreamEvent{Text: chunk.Message.Content}
			}
		}
		if err := scanner.Err(); err != nil {
			// Context-cancelled reads look like errors; surface cleanly.
			if ctx.Err() != nil {
				out <- StreamEvent{Err: ctx.Err()}
				return
			}
			out <- StreamEvent{Err: fmt.Errorf("read stream: %w", err)}
			return
		}
		// Stream ended without a done=true sentinel. Synthesise one so
		// the caller sees a clean finish rather than a phantom error.
		out <- StreamEvent{Done: true}
	}()

	return out
}
