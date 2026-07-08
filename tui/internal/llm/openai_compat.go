package llm

// OpenAI-compatible streaming chat client. Works with any endpoint that
// speaks the OpenAI chat/completions API — z.ai, OpenCode, OpenRouter,
// hmanlab, MiniMax. Mirrors cli/src/openai_compat.rs.

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
)

// OpenAICompat is the OpenAI-compatible streaming client.
type OpenAICompat struct {
	base   string
	apiKey string
	http   *http.Client
}

// NewOpenAICompat builds a client for an OpenAI-compatible provider.
func NewOpenAICompat(base, apiKey string) *OpenAICompat {
	return &OpenAICompat{
		base:   strings.TrimRight(base, "/"),
		apiKey: strings.TrimSpace(apiKey),
		http:   &http.Client{Timeout: 5 * time.Minute},
	}
}

// Name implements Provider — returns the base URL as identity.
func (c *OpenAICompat) Name() string { return c.base }

// Models implements Provider — empty (models come from the provider
// catalog in config, not from the client).
func (c *OpenAICompat) Models() []string { return nil }

// Ping is a no-op for OpenAI-compat (no universal validation endpoint).
func (c *OpenAICompat) Ping(ctx context.Context) error { return nil }

// StreamChat implements Provider. POSTs to {base}/chat/completions with
// stream:true and parses the SSE response.
func (c *OpenAICompat) StreamChat(ctx context.Context, model string, messages []Message) <-chan StreamEvent {
	ch := make(chan StreamEvent, 32)
	go c.stream(ctx, model, messages, ch)
	return ch
}

func (c *OpenAICompat) stream(ctx context.Context, model string, messages []Message, ch chan<- StreamEvent) {
	defer close(ch)

	type oaiMessage struct {
		Role    string `json:"role"`
		Content string `json:"content"`
	}
	type oaiRequest struct {
		Model         string       `json:"model"`
		Messages      []oaiMessage `json:"messages"`
		Stream        bool         `json:"stream"`
		MaxTokens     int          `json:"max_tokens"`
		StreamOptions struct {
			IncludeUsage bool `json:"include_usage"`
		} `json:"stream_options"`
	}

	oaiMsgs := make([]oaiMessage, len(messages))
	for i, m := range messages {
		oaiMsgs[i] = oaiMessage{Role: string(m.Role), Content: m.Content}
	}

	reqBody := oaiRequest{
		Model:     model,
		Messages:  oaiMsgs,
		Stream:    true,
		MaxTokens: 32000,
	}
	reqBody.StreamOptions.IncludeUsage = true

	bodyBytes, err := json.Marshal(reqBody)
	if err != nil {
		ch <- StreamEvent{Err: fmt.Errorf("encode request: %w", err), Done: true}
		return
	}

	url := c.base + "/chat/completions"

	const maxAttempts = 4
	const baseDelay = 800 * time.Millisecond

	var lastErr string
	for attempt := 0; attempt < maxAttempts; attempt++ {
		req, err := http.NewRequestWithContext(ctx, http.MethodPost, url, bytes.NewReader(bodyBytes))
		if err != nil {
			ch <- StreamEvent{Err: fmt.Errorf("build request: %w", err), Done: true}
			return
		}
		req.Header.Set("Content-Type", "application/json")
		req.Header.Set("Authorization", "Bearer "+c.apiKey)

		resp, err := c.http.Do(req)
		if err != nil {
			ch <- StreamEvent{Err: fmt.Errorf("POST %s: %w", url, err), Done: true}
			return
		}

		if resp.StatusCode == http.StatusOK {
			c.parseSSE(ctx, resp.Body, ch)
			resp.Body.Close()
			return
		}

		// Non-2xx — read body for error message.
		respBody, _ := io.ReadAll(io.LimitReader(resp.Body, 4096))
		resp.Body.Close()
		preview := strings.TrimSpace(string(respBody))
		if len(preview) > 1000 {
			preview = preview[:1000]
		}
		lastErr = fmt.Sprintf("HTTP %d — %s", resp.StatusCode, preview)

		// Retry on transient errors.
		retryable := resp.StatusCode == 429 ||
			resp.StatusCode == 502 ||
			resp.StatusCode == 503 ||
			resp.StatusCode == 504
		if !retryable || attempt+1 == maxAttempts {
			ch <- StreamEvent{Err: fmt.Errorf("POST %s: %s", url, lastErr), Done: true}
			return
		}

		// Exponential backoff: 800ms, 1.6s, 3.2s.
		delay := baseDelay << attempt
		select {
		case <-ctx.Done():
			ch <- StreamEvent{Err: ctx.Err(), Done: true}
			return
		case <-time.After(delay):
		}
	}

	ch <- StreamEvent{Err: fmt.Errorf("POST %s: %s", url, lastErr), Done: true}
}

// parseSSE reads the SSE response stream and emits StreamEvents.
func (c *OpenAICompat) parseSSE(ctx context.Context, body io.Reader, ch chan<- StreamEvent) {
	scanner := bufio.NewScanner(body)
	scanner.Buffer(make([]byte, 0, 64*1024), 1024*1024)

	var promptTokens, completionTokens int

	for scanner.Scan() {
		// Check for cancellation.
		select {
		case <-ctx.Done():
			ch <- StreamEvent{Err: ctx.Err(), Done: true}
			return
		default:
		}

		line := strings.TrimSpace(scanner.Text())
		if line == "" {
			continue
		}
		// SSE frames start with "data: ". Skip keep-alive comments.
		payload, ok := strings.CutPrefix(line, "data:")
		if !ok {
			continue
		}
		payload = strings.TrimSpace(payload)

		if payload == "[DONE]" {
			ch <- StreamEvent{
				Done:              true,
				PromptTokens:      promptTokens,
				CompletionTokens:  completionTokens,
			}
			return
		}

		// Parse the JSON chunk.
		var chunk struct {
			Choices []struct {
				Delta struct {
					Content string `json:"content"`
				} `json:"delta"`
				FinishReason *string `json:"finish_reason"`
			} `json:"choices"`
			Usage *struct {
				PromptTokens     int `json:"prompt_tokens"`
				CompletionTokens int `json:"completion_tokens"`
			} `json:"usage"`
		}
		if err := json.Unmarshal([]byte(payload), &chunk); err != nil {
			continue // skip malformed
		}

		// Stash usage (may arrive on finish_reason chunk or [DONE]).
		if chunk.Usage != nil {
			if chunk.Usage.PromptTokens > 0 || chunk.Usage.CompletionTokens > 0 {
				promptTokens = chunk.Usage.PromptTokens
				completionTokens = chunk.Usage.CompletionTokens
			}
		}

		// Extract content delta.
		if len(chunk.Choices) > 0 {
			content := chunk.Choices[0].Delta.Content
			if content != "" {
				ch <- StreamEvent{Text: content}
			}
		}
	}

	// Stream ended without [DONE] — emit done with whatever usage we have.
	ch <- StreamEvent{
		Done:             true,
		PromptTokens:     promptTokens,
		CompletionTokens: completionTokens,
	}
}
