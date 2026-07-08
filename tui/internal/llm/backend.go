package llm

import (
	"github.com/hmanlab/hmanlab/tui/internal/config"
)

// BackendFor returns the streaming client for a given provider ID, or
// nil if the provider isn't configured (no key). Mirrors
// cli/src/app/backend.rs::make_backend_for.
//
// Ollama Cloud uses the native Ollama protocol (/api/chat).
// All other BYOK providers use the OpenAI-compatible protocol
// (/v1/chat/completions, SSE).
func BackendFor(cfg *config.Config, provider string) Provider {
	key := cfg.ByokKey(provider)
	if key == "" && provider != config.OllamaCloudProvider {
		return nil
	}

	switch provider {
	case config.OllamaCloudProvider:
		return NewCloudOllama(key)

	case config.ZaiSubscriptionProvider,
		config.ZaiUsageProvider,
		config.OpencodeProvider,
		config.OpenrouterProvider,
		config.HmanlabProvider,
		config.MinimaxProvider:
		return NewOpenAICompat(config.ProviderBase(provider), key)

	default:
		return nil
	}
}
