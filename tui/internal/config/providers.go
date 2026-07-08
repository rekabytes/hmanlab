package config

// Provider catalog — all BYOK provider IDs, base URLs, model lists, and
// labels. Mirrors cli/src/config.rs constants. Keep in lockstep so the
// wire format + model names can't drift between the two implementations.

// Provider IDs.
const (
	ZaiSubscriptionProvider = "zai-subscription"
	ZaiUsageProvider        = "zai-usage"
	OllamaCloudProvider     = "ollama-cloud"
	OpencodeProvider        = "opencode"
	OpenrouterProvider      = "openrouter"
	HmanlabProvider         = "hmanlab"
	MinimaxProvider         = "minimax"
)

// Base URLs.
const (
	ZaiSubscriptionBase = "https://api.z.ai/api/coding/paas/v4"
	ZaiUsageBase        = "https://api.z.ai/api/paas/v4"
	OllamaCloudBase     = "https://ollama.com"
	OpencodeBase        = "https://opencode.ai/zen/go/v1"
	OpenrouterBase      = "https://openrouter.ai/api/v1"
	HmanlabBase         = "https://ai.hmanlab.pro/v1"
	MinimaxBase         = "https://api.minimax.io/v1"
)

// Default models.
const (
	ZaiDefaultModel        = "glm-4.7"
	OllamaCloudDefaultModel = "glm-4.7"
	OpencodeDefaultModel   = "glm-5.1"
	OpenrouterDefaultModel = "anthropic/claude-sonnet-4.6"
	HmanlabDefaultModel    = "claude-sonnet-4-6"
	MinimaxDefaultModel    = "MiniMax-M3"
)

// Model catalogs — static seed lists. Users can add more via extra_models.
var (
	ZaiModels = []string{"glm-4.7", "glm-4.6", "glm-5.1"}

	OllamaCloudModels = []string{
		"deepseek-v3.2",
		"deepseek-v4-flash",
		"deepseek-v4-pro",
		"devstral-small-2",
		"gemini-3-flash-preview",
		"gemma4",
		"glm-4.7",
		"glm-5",
		"glm-5.1",
		"gpt-oss:120b-cloud",
		"kimi-k2.6",
		"minimax-m2.1",
		"minimax-m2.5",
		"minimax-m2.7",
		"ministral-3",
		"nemotron-3-nano",
		"nemotron-3-super",
		"qwen3.5",
		"qwen3-coder-next",
		"qwen3-next",
		"rnj-1",
	}

	OpencodeModels = []string{
		"glm-5.1",
		"glm-5",
		"qwen3.6-plus",
		"qwen3.5-plus",
		"kimi-k2.6",
		"kimi-k2.5",
		"minimax-m2.7",
		"minimax-m2.5",
	}

	OpenrouterModels = []string{
		"anthropic/claude-opus-4.7",
		"anthropic/claude-sonnet-4.6",
		"anthropic/claude-haiku-4.5",
		"openai/gpt-5.5",
		"openai/gpt-5.5-pro",
		"openai/gpt-5.4-mini",
		"google/gemini-3.1-flash-lite",
		"google/gemini-2.5-pro",
		"deepseek/deepseek-v4-pro",
		"qwen/qwen3.6-plus",
		"x-ai/grok-4.3",
		"moonshotai/kimi-k2.6",
		"meta-llama/llama-3.3-70b-instruct",
	}

	HmanlabModels = []string{
		"claude-sonnet-4-6",
		"claude-opus-4-8",
		"claude-opus-4-7",
		"claude-haiku-4-5",
		"gpt-5.5",
		"gpt-5.4",
	}

	MinimaxModels = []string{"MiniMax-M3", "M2.7", "M2.6"}
)

// BYOKProviderIDs is the ordered list of all BYOK providers.
var BYOKProviderIDs = []string{
	ZaiSubscriptionProvider,
	ZaiUsageProvider,
	OllamaCloudProvider,
	OpencodeProvider,
	OpenrouterProvider,
	HmanlabProvider,
	MinimaxProvider,
}

// ProviderLabel returns a human-readable name for a provider ID.
func ProviderLabel(provider string) string {
	switch provider {
	case ZaiSubscriptionProvider:
		return "z.ai (subscription)"
	case ZaiUsageProvider:
		return "z.ai (usage-based)"
	case OllamaCloudProvider:
		return "Ollama Cloud"
	case OpencodeProvider:
		return "OpenCode Go"
	case OpenrouterProvider:
		return "OpenRouter"
	case HmanlabProvider:
		return "hmanlab"
	case MinimaxProvider:
		return "MiniMax"
	default:
		return provider
	}
}

// ProviderBase returns the API base URL for a provider ID.
func ProviderBase(provider string) string {
	switch provider {
	case ZaiSubscriptionProvider:
		return ZaiSubscriptionBase
	case ZaiUsageProvider:
		return ZaiUsageBase
	case OllamaCloudProvider:
		return OllamaCloudBase
	case OpencodeProvider:
		return OpencodeBase
	case OpenrouterProvider:
		return OpenrouterBase
	case HmanlabProvider:
		return HmanlabBase
	case MinimaxProvider:
		return MinimaxBase
	default:
		return ""
	}
}

// ProviderModels returns the static model catalog for a provider ID.
func ProviderModels(provider string) []string {
	switch provider {
	case ZaiSubscriptionProvider, ZaiUsageProvider:
		return ZaiModels
	case OllamaCloudProvider:
		return OllamaCloudModels
	case OpencodeProvider:
		return OpencodeModels
	case OpenrouterProvider:
		return OpenrouterModels
	case HmanlabProvider:
		return HmanlabModels
	case MinimaxProvider:
		return MinimaxModels
	default:
		return nil
	}
}

// ProviderDefaultModel returns the default model for a provider ID.
func ProviderDefaultModel(provider string) string {
	switch provider {
	case ZaiSubscriptionProvider, ZaiUsageProvider:
		return ZaiDefaultModel
	case OllamaCloudProvider:
		return OllamaCloudDefaultModel
	case OpencodeProvider:
		return OpencodeDefaultModel
	case OpenrouterProvider:
		return OpenrouterDefaultModel
	case HmanlabProvider:
		return HmanlabDefaultModel
	case MinimaxProvider:
		return MinimaxDefaultModel
	default:
		return ""
	}
}

// ProviderKeyField returns the TOML config key name that stores the
// API key for a provider ID. Returns "" for providers without a key
// (local Ollama).
func ProviderKeyField(provider string) string {
	switch provider {
	case ZaiSubscriptionProvider:
		return "zai_api_key"
	case ZaiUsageProvider:
		return "zai_usage_api_key"
	case OllamaCloudProvider:
		return "ollama_cloud_api_key"
	case OpencodeProvider:
		return "opencode_api_key"
	case OpenrouterProvider:
		return "openrouter_api_key"
	case HmanlabProvider:
		return "hmanlab_api_key"
	case MinimaxProvider:
		return "minimax_api_key"
	default:
		return ""
	}
}

// ProviderGetURL returns the "get a key" hint URL for a provider.
func ProviderGetURL(provider string) string {
	switch provider {
	case ZaiSubscriptionProvider, ZaiUsageProvider:
		return "https://z.ai"
	case OllamaCloudProvider:
		return "https://ollama.com/settings/keys"
	case OpencodeProvider:
		return "https://opencode.ai"
	case OpenrouterProvider:
		return "https://openrouter.ai/keys"
	case HmanlabProvider:
		return "https://ai.hmanlab.pro"
	case MinimaxProvider:
		return "https://api.minimax.io"
	default:
		return ""
	}
}
