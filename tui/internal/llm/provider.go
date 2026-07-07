// Package llm wraps the chat-completion providers. The Provider
// interface is forward-compat for v0.1+ (z.ai, OpenRouter, etc.) —
// for v0 only Ollama Cloud has a concrete impl, but every call site
// talks to the interface so adding a second provider later is just
// another implementation file.
package llm

import "context"

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
// (role + content) — no tools at v0, no attachments, no images.
type Message struct {
	Role    Role   `json:"role"`
	Content string `json:"content"`
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
	StreamChat(ctx context.Context, model string, messages []Message) <-chan StreamEvent
}

// StreamEvent is what flows back from a StreamChat call. Either a
// chunk of content tokens (Token.Text may be empty), a terminal Done
// carrying usage counts, or an Error.
//
// Mirrors the cli's StreamItem enum (cli/src/ollama.rs:297) — same
// variant shape, just Go-flavoured.
type StreamEvent struct {
	// Text is a streamed token fragment. Empty on non-Content events.
	Text string

	// Done is true on the terminal event — last chunk in the stream.
	Done bool

	// PromptTokens / CompletionTokens are the usage counts carried on
	// the terminal chunk. 0 for intermediate events.
	PromptTokens     int
	CompletionTokens int

	// Err is set on a stream-level error (network drop, auth failure,
	// JSON parse failure, etc.). When set, the channel closes right
	// after this event.
	Err error
}
