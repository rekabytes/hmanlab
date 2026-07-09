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
func (c *OpenAICompat) StreamChat(ctx context.Context, model string, messages []Message, tools []Tool) <-chan StreamEvent {
	ch := make(chan StreamEvent, 32)
	go c.stream(ctx, model, messages, tools, ch)
	return ch
}

func (c *OpenAICompat) stream(ctx context.Context, model string, messages []Message, tools []Tool, ch chan<- StreamEvent) {
	defer close(ch)

	type oaiTCall struct {
		ID       string `json:"id,omitempty"`
		Type     string `json:"type"` // "function"
		Function struct {
			Name      string `json:"name"`
			Arguments string `json:"arguments"` // JSON-encoded string
		} `json:"function"`
	}
	type oaiMessage struct {
		Role       string     `json:"role"`
		Content    string     `json:"content,omitempty"`
		ToolCalls  []oaiTCall `json:"tool_calls,omitempty"`
		Name       string     `json:"name,omitempty"`
		ToolCallID string     `json:"tool_call_id,omitempty"`
	}

	oaiMsgs := make([]oaiMessage, len(messages))
	for i, m := range messages {
		om := oaiMessage{
			Role:       m.Role,
			Content:    m.Content,
			Name:       m.Name,
			ToolCallID: m.ToolCallID,
		}
		for j, tc := range m.ToolCalls {
			var oc oaiTCall
			oc.ID = fmt.Sprintf("call_%d", j)
			oc.Type = "function"
			oc.Function.Name = tc.Function.Name
			oc.Function.Arguments = string(tc.Function.Arguments)
			om.ToolCalls = append(om.ToolCalls, oc)
		}
		oaiMsgs[i] = om
	}

	type oaiRequest struct {
		Model         string       `json:"model"`
		Messages      []oaiMessage `json:"messages"`
		Stream        bool         `json:"stream"`
		MaxTokens     int          `json:"max_tokens"`
		StreamOptions struct {
			IncludeUsage bool `json:"include_usage"`
		} `json:"stream_options"`
		Tools []Tool `json:"tools,omitempty"`
	}

	reqBody := oaiRequest{
		Model:     model,
		Messages:  oaiMsgs,
		Stream:    true,
		MaxTokens: 32000,
		Tools:     tools,
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
// Tool-call arguments arrive as fragmented deltas — accumulated by
// index, concatenated, then emitted as one batch.
func (c *OpenAICompat) parseSSE(ctx context.Context, body io.Reader, ch chan<- StreamEvent) {
	scanner := bufio.NewScanner(body)
	scanner.Buffer(make([]byte, 0, 64*1024), 1024*1024)

	var promptTokens, completionTokens int

	// Accumulator for fragmented tool calls (arguments arrive in pieces).
	type partialToolCall struct {
		Name string
		Args string
	}
	toolCallMap := make(map[int]*partialToolCall)

	flushToolCalls := func() {
		if len(toolCallMap) == 0 {
			return
		}
		// Emit in index order.
		maxIdx := -1
		for i := range toolCallMap {
			if i > maxIdx {
				maxIdx = i
			}
		}
		var calls []ToolCall
		for i := 0; i <= maxIdx; i++ {
			tc, ok := toolCallMap[i]
			if !ok {
				continue
			}
			calls = append(calls, ToolCall{
				Function: ToolCallFunction{
					Name:      tc.Name,
					Arguments: json.RawMessage(tc.Args),
				},
			})
		}
		ch <- StreamEvent{ToolCalls: calls}
		toolCallMap = make(map[int]*partialToolCall)
	}

	for scanner.Scan() {
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
		payload, ok := strings.CutPrefix(line, "data:")
		if !ok {
			continue
		}
		payload = strings.TrimSpace(payload)

		if payload == "[DONE]" {
			flushToolCalls()
			ch <- StreamEvent{
				Done:             true,
				PromptTokens:     promptTokens,
				CompletionTokens: completionTokens,
			}
			return
		}

		var chunk struct {
			Choices []struct {
				Delta struct {
					Content   string `json:"content"`
					ToolCalls []struct {
						Index    int `json:"index"`
						Function struct {
							Name      string `json:"name"`
							Arguments string `json:"arguments"`
						} `json:"function"`
					} `json:"tool_calls"`
				} `json:"delta"`
				FinishReason *string `json:"finish_reason"`
			} `json:"choices"`
			Usage *struct {
				PromptTokens     int `json:"prompt_tokens"`
				CompletionTokens int `json:"completion_tokens"`
			} `json:"usage"`
		}
		if err := json.Unmarshal([]byte(payload), &chunk); err != nil {
			continue
		}

		if chunk.Usage != nil {
			if chunk.Usage.PromptTokens > 0 || chunk.Usage.CompletionTokens > 0 {
				promptTokens = chunk.Usage.PromptTokens
				completionTokens = chunk.Usage.CompletionTokens
			}
		}

		if len(chunk.Choices) > 0 {
			choice := chunk.Choices[0]

			// Accumulate tool-call fragments by index.
			for _, tc := range choice.Delta.ToolCalls {
				slot, exists := toolCallMap[tc.Index]
				if !exists {
					slot = &partialToolCall{}
					toolCallMap[tc.Index] = slot
				}
				if tc.Function.Name != "" {
					slot.Name = tc.Function.Name
				}
				slot.Args += tc.Function.Arguments
			}

			if choice.Delta.Content != "" {
				ch <- StreamEvent{Text: choice.Delta.Content}
			}

			if choice.FinishReason != nil && *choice.FinishReason == "tool_calls" {
				flushToolCalls()
			}
		}
	}

	flushToolCalls()
	ch <- StreamEvent{
		Done:             true,
		PromptTokens:     promptTokens,
		CompletionTokens: completionTokens,
	}
}
