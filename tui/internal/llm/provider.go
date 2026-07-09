// Package llm wraps the chat-completion providers. The Provider
// interface is forward-compat for v0.1+ (z.ai, OpenRouter, etc.) —
// for v0 only Ollama Cloud has a concrete impl, but every call site
// talks to the interface so adding a second provider later is just
// another implementation file.
package llm

import (
	"context"
	"encoding/json"
)

// Role labels a message's author. Match these strings against the cli's
// wire format (cli/src/ollama.rs::ApiMessage::role) so the same JSON
// bytes round-trip between the two clients.
type Role string

const (
	RoleSystem    Role = "system"
	RoleUser      Role = "user"
	RoleAssistant Role = "assistant"
)

// Message is a single chat-turn. Mirrors Ollama's chat-message shape
// (role + content) — extended with tool_calls and tool fields for the
// agent loop.
type Message struct {
	Role       string     `json:"role"`
	Content    string     `json:"content"`
	ToolCalls  []ToolCall `json:"tool_calls,omitempty"`
	Name       string     `json:"name,omitempty"`
	ToolCallID string     `json:"tool_call_id,omitempty"`
}

// Tool is a function-definition sent to the model so it knows what
// tools are available. Mirrors the OpenAI tool shape (also accepted
// by Ollama).
type Tool struct {
	Type     string       `json:"type"` // always "function"
	Function ToolFunction `json:"function"`
}

// ToolFunction is the schema + description for a single tool.
type ToolFunction struct {
	Name        string         `json:"name"`
	Description string         `json:"description"`
	Parameters  map[string]any `json:"parameters"` // JSON Schema object
}

// ToolCall is what the model emits back when it wants to call a tool.
type ToolCall struct {
	Function ToolCallFunction `json:"function"`
}

// ToolCallFunction carries the tool name + parsed arguments.
type ToolCallFunction struct {
	Name      string          `json:"name"`
	Arguments json.RawMessage `json:"arguments"`
}

// Provider is the chat-completion surface. Implementations must be
// safe for concurrent use (Bubble Tea's Update runs single-threaded
// but the streaming goroutine fires alongside it).
type Provider interface {
	// Name returns the provider ID (e.g. "ollama-cloud"). Used in the
	// status bar / header to show what's connected.
	Name() string

	// Models lists the models this provider exposes. For Ollama Cloud
	// this is the static catalog (config.OllamaCloudModels). For
	// future providers it could be a live API call.
	Models() []string

	// StreamChat opens a streaming chat-completion request and returns
	// a channel of StreamEvent values. The channel closes when the
	// stream finishes (either naturally on the model's `[DONE]`
	// sentinel, or because ctx was cancelled).
	//
	// Implementations should respect ctx.Done() promptly — Bubble Tea
	// cancels on Ctrl+C and expects the goroutine to return without
	// leaking.
	StreamChat(ctx context.Context, model string, messages []Message, tools []Tool) <-chan StreamEvent
}

// StreamEvent is what flows back from a StreamChat call. Either a
// chunk of content tokens, a batch of tool calls, a terminal Done
// carrying usage counts, or an Error.
type StreamEvent struct {
	// Text is a streamed token fragment. Empty on non-Content events.
	Text string

	// ToolCalls is non-nil when the model wants to call tools.
	// Accumulated across the stream and emitted as one batch.
	ToolCalls []ToolCall

	// Done is true on the terminal event — last chunk in the stream.
	Done bool

	// PromptTokens / CompletionTokens are the usage counts carried on
	// the terminal chunk.
	PromptTokens     int
	CompletionTokens int

	// Err is set on a stream-level error.
	Err error
}
