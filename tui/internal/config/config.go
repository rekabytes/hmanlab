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
const (
	OllamaCloudProvider     = "ollama-cloud"
	OllamaCloudBase         = "https://ollama.com"
	OllamaCloudDefaultModel = "glm-4.7"
)

// OllamaCloudModels is the catalog the cli advertises for the cloud
// provider. Mirror of OLLAMA_CLOUD_MODELS in cli/src/config.rs.
var OllamaCloudModels = []string{
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

// Config mirrors the top-level TOML schema in cli/src/config.rs::Config.
// Only the fields we actually read/write at v0 are exposed as typed;
// everything else is preserved through the Raw map so we don't drop
// user data on save.
type Config struct {
	// Fields we actively use. Snake_case TOML keys match the cli.
	OllamaCloudAPIKey string `toml:"ollama_cloud_api_key,omitempty"`
	LastModel         string `toml:"last_model,omitempty"`
	LastProvider      string `toml:"last_provider,omitempty"`

	// Raw preserves every other TOML key the user may have set so
	// writing the config back doesn't nuke z.ai keys, Telegram tokens,
	// trusted_workspaces, etc. We deserialise into Raw first, then
	// re-serialise back alongside any updates.
	Raw map[string]any `toml:"-"`
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
		OllamaCloudAPIKey string `toml:"ollama_cloud_api_key,omitempty"`
		LastModel         string `toml:"last_model,omitempty"`
		LastProvider      string `toml:"last_provider,omitempty"`
	}
	if err := toml.Unmarshal(data, &typed); err != nil {
		return nil, fmt.Errorf("parse %s: %w", path, err)
	}
	cfg.OllamaCloudAPIKey = typed.OllamaCloudAPIKey
	cfg.LastModel = typed.LastModel
	cfg.LastProvider = typed.LastProvider

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
	if c.LastModel != "" {
		c.Raw["last_model"] = c.LastModel
	}
	if c.LastProvider != "" {
		c.Raw["last_provider"] = c.LastProvider
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

// EffectiveModel returns the model the TUI should boot with: the
// last-used model if set, else OllamaCloudDefaultModel.
func (c *Config) EffectiveModel() string {
	if c.LastModel != "" {
		return c.LastModel
	}
	return OllamaCloudDefaultModel
}
