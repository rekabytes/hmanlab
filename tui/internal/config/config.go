// Package config loads + writes the user-level hmanlab config at
// ~/.config/hmanlab/config.toml. The schema mirrors cli/src/config.rs
// field-for-field so the same file works in both clients without
// migration.
package config

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/BurntSushi/toml"
)

// Constants matching cli/src/config.rs. Keep these in lockstep with
// the Rust client so provider IDs + base URLs can't drift between
// the two implementations.

// Ollama Cloud — already defined above, kept for backward compat.
// See providers.go for all other provider constants.

// DefaultOllamaHost is the default local Ollama endpoint.
const DefaultOllamaHost = "http://localhost:11434"

// Config mirrors the top-level TOML schema in cli/src/config.rs::Config.
// Only the fields we actually read/write are exposed as typed;
// everything else is preserved through the Raw map so we don't drop
// user data on save.
type Config struct {
	// BYOK API keys (snake_case TOML keys match the cli).
	ZaiAPIKey         string `toml:"zai_api_key,omitempty"`
	ZaiUsageAPIKey    string `toml:"zai_usage_api_key,omitempty"`
	OllamaCloudAPIKey string `toml:"ollama_cloud_api_key,omitempty"`
	OpencodeAPIKey    string `toml:"opencode_api_key,omitempty"`
	OpenrouterAPIKey  string `toml:"openrouter_api_key,omitempty"`
	HmanlabAPIKey     string `toml:"hmanlab_api_key,omitempty"`
	MinimaxAPIKey     string `toml:"minimax_api_key,omitempty"`

	// Local Ollama endpoint (no key needed).
	OllamaHost string `toml:"ollama_host,omitempty"`

	// Persisted last-used selection so relaunch boots with the same
	// provider + model.
	LastModel    string `toml:"last_model,omitempty"`
	LastProvider string `toml:"last_provider,omitempty"`

	// ExtraModels holds user-added models per provider.
	ExtraModels []ExtraModel `toml:"extra_models,omitempty"`

	// Raw preserves every other TOML key the user may have set so
	// writing the config back doesn't nuke unknown fields.
	Raw map[string]any `toml:"-"`
}

// ExtraModel is a user-added model entry, mirroring cli's ExtraModel.
type ExtraModel struct {
	Provider string `toml:"provider"`
	Name     string `toml:"name"`
}

// Path returns the canonical config file path. Respects
// XDG_CONFIG_HOME if set; falls back to ~/.config/hmanlab/config.toml.
// Mirrors cli/src/config.rs::config_path so the two clients read the
// same file on disk.
func Path() (string, error) {
	if xdg := os.Getenv("XDG_CONFIG_HOME"); xdg != "" {
		return filepath.Join(xdg, "hmanlab", "config.toml"), nil
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("resolve $HOME: %w", err)
	}
	return filepath.Join(home, ".config", "hmanlab", "config.toml"), nil
}

// Dir returns the parent directory of Path() — useful for `mkdir -p`
// before writing.
func Dir() (string, error) {
	p, err := Path()
	if err != nil {
		return "", err
	}
	return filepath.Dir(p), nil
}

// Load reads the config file. Returns a zero-value Config (no error)
// if the file doesn't exist yet — first-run flow handles that case.
// Returns an error only on read or parse failure of an existing file.
func Load() (*Config, error) {
	path, err := Path()
	if err != nil {
		return nil, err
	}
	cfg := &Config{Raw: map[string]any{}}

	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return cfg, nil
		}
		return nil, fmt.Errorf("read %s: %w", path, err)
	}

	// Decode into Raw first so we preserve every key for round-trip writes.
	if err := toml.Unmarshal(data, &cfg.Raw); err != nil {
		return nil, fmt.Errorf("parse %s: %w", path, err)
	}

	// Decode again into a typed shadow struct so we get the fields we
	// care about with proper types. Using a separate struct avoids
	// touching Raw.
	var typed struct {
		ZaiAPIKey         string      `toml:"zai_api_key,omitempty"`
		ZaiUsageAPIKey    string      `toml:"zai_usage_api_key,omitempty"`
		OllamaCloudAPIKey string      `toml:"ollama_cloud_api_key,omitempty"`
		OpencodeAPIKey    string      `toml:"opencode_api_key,omitempty"`
		OpenrouterAPIKey  string      `toml:"openrouter_api_key,omitempty"`
		HmanlabAPIKey     string      `toml:"hmanlab_api_key,omitempty"`
		MinimaxAPIKey     string      `toml:"minimax_api_key,omitempty"`
		OllamaHost        string      `toml:"ollama_host,omitempty"`
		LastModel         string      `toml:"last_model,omitempty"`
		LastProvider      string      `toml:"last_provider,omitempty"`
		ExtraModels       []ExtraModel `toml:"extra_models,omitempty"`
	}
	if err := toml.Unmarshal(data, &typed); err != nil {
		return nil, fmt.Errorf("parse %s: %w", path, err)
	}
	cfg.ZaiAPIKey = typed.ZaiAPIKey
	cfg.ZaiUsageAPIKey = typed.ZaiUsageAPIKey
	cfg.OllamaCloudAPIKey = typed.OllamaCloudAPIKey
	cfg.OpencodeAPIKey = typed.OpencodeAPIKey
	cfg.OpenrouterAPIKey = typed.OpenrouterAPIKey
	cfg.HmanlabAPIKey = typed.HmanlabAPIKey
	cfg.MinimaxAPIKey = typed.MinimaxAPIKey
	cfg.OllamaHost = typed.OllamaHost
	cfg.LastModel = typed.LastModel
	cfg.LastProvider = typed.LastProvider
	cfg.ExtraModels = typed.ExtraModels

	return cfg, nil
}

// Save writes the config back to disk. Fields the user already had
// (preserved in Raw) are re-serialised; the typed fields on Config are
// overlaid on top. Atomic-ish: writes to a temp file in the same dir,
// then renames over the target so a partial write never lands.
func (c *Config) Save() error {
	dir, err := Dir()
	if err != nil {
		return err
	}
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return fmt.Errorf("mkdir %s: %w", dir, err)
	}

	// Overlay typed fields onto Raw so both stay in sync.
	if c.OllamaCloudAPIKey != "" {
		c.Raw["ollama_cloud_api_key"] = c.OllamaCloudAPIKey
	}
	if c.ZaiAPIKey != "" {
		c.Raw["zai_api_key"] = c.ZaiAPIKey
	}
	if c.ZaiUsageAPIKey != "" {
		c.Raw["zai_usage_api_key"] = c.ZaiUsageAPIKey
	}
	if c.OpencodeAPIKey != "" {
		c.Raw["opencode_api_key"] = c.OpencodeAPIKey
	}
	if c.OpenrouterAPIKey != "" {
		c.Raw["openrouter_api_key"] = c.OpenrouterAPIKey
	}
	if c.HmanlabAPIKey != "" {
		c.Raw["hmanlab_api_key"] = c.HmanlabAPIKey
	}
	if c.MinimaxAPIKey != "" {
		c.Raw["minimax_api_key"] = c.MinimaxAPIKey
	}
	if c.OllamaHost != "" {
		c.Raw["ollama_host"] = c.OllamaHost
	}
	if c.LastModel != "" {
		c.Raw["last_model"] = c.LastModel
	}
	if c.LastProvider != "" {
		c.Raw["last_provider"] = c.LastProvider
	}
	if len(c.ExtraModels) > 0 {
		c.Raw["extra_models"] = c.ExtraModels
	}

	path, err := Path()
	if err != nil {
		return err
	}
	tmp, err := os.CreateTemp(dir, ".config.toml.*")
	if err != nil {
		return fmt.Errorf("create temp: %w", err)
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName) // no-op if rename succeeds

	enc := toml.NewEncoder(tmp)
	if err := enc.Encode(c.Raw); err != nil {
		tmp.Close()
		return fmt.Errorf("encode: %w", err)
	}
	if err := tmp.Close(); err != nil {
		return fmt.Errorf("close temp: %w", err)
	}
	if err := os.Rename(tmpName, path); err != nil {
		return fmt.Errorf("rename: %w", err)
	}
	return nil
}

// HasOllamaCloudKey returns true iff a non-empty Ollama Cloud API key
// is configured. Drives the first-run modal in the UI.
func (c *Config) HasOllamaCloudKey() bool {
	return c.OllamaCloudAPIKey != ""
}

// ConfiguredProviders returns the IDs of all BYOK providers that have
// a non-empty API key configured.
func (c *Config) ConfiguredProviders() []string {
	var configured []string
	for _, p := range BYOKProviderIDs {
		if c.ByokKey(p) != "" {
			configured = append(configured, p)
		}
	}
	return configured
}

// ByokKey returns the API key for a provider ID, or "" if not configured.
func (c *Config) ByokKey(provider string) string {
	switch provider {
	case ZaiSubscriptionProvider:
		return c.ZaiAPIKey
	case ZaiUsageProvider:
		return c.ZaiUsageAPIKey
	case OllamaCloudProvider:
		return c.OllamaCloudAPIKey
	case OpencodeProvider:
		return c.OpencodeAPIKey
	case OpenrouterProvider:
		return c.OpenrouterAPIKey
	case HmanlabProvider:
		return c.HmanlabAPIKey
	case MinimaxProvider:
		return c.MinimaxAPIKey
	default:
		return ""
	}
}

// SetByokKey sets the API key for a provider ID.
func (c *Config) SetByokKey(provider, key string) {
	switch provider {
	case ZaiSubscriptionProvider:
		c.ZaiAPIKey = key
	case ZaiUsageProvider:
		c.ZaiUsageAPIKey = key
	case OllamaCloudProvider:
		c.OllamaCloudAPIKey = key
	case OpencodeProvider:
		c.OpencodeAPIKey = key
	case OpenrouterProvider:
		c.OpenrouterAPIKey = key
	case HmanlabProvider:
		c.HmanlabAPIKey = key
	case MinimaxProvider:
		c.MinimaxAPIKey = key
	}
}

// EffectiveModel returns the model the TUI should boot with: the
// last-used model if set, else the default model for the last-used
// provider, else OllamaCloudDefaultModel.
func (c *Config) EffectiveModel() string {
	if c.LastModel != "" {
		return c.LastModel
	}
	if c.LastProvider != "" {
		if dm := ProviderDefaultModel(c.LastProvider); dm != "" {
			return dm
		}
	}
	return OllamaCloudDefaultModel
}
