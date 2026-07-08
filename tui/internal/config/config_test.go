package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestLoadMissingFile verifies the first-run case: no config file
// exists yet, Load returns a zero-value Config + no error. The UI's
// connect modal relies on this so it can launch cleanly on a fresh
// machine.
func TestLoadMissingFile(t *testing.T) {
	dir := t.TempDir()
	// Override the path resolution by setting XDG_CONFIG_HOME.
	t.Setenv("XDG_CONFIG_HOME", dir)
	cfg, err := Load()
	if err != nil {
		t.Fatalf("Load on missing file: %v", err)
	}
	if cfg == nil {
		t.Fatal("Load returned nil config")
	}
	if cfg.HasOllamaCloudKey() {
		t.Errorf("zero-value config should have no key, got %q", cfg.OllamaCloudAPIKey)
	}
	if cfg.EffectiveModel() != OllamaCloudDefaultModel {
		t.Errorf("default model = %q, want %q", cfg.EffectiveModel(), OllamaCloudDefaultModel)
	}
}

// TestRoundTrip writes a config, reloads it, and asserts every field
// survived the toml round-trip.
func TestRoundTrip(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)

	cfg, err := Load()
	if err != nil {
		t.Fatal(err)
	}
	cfg.OllamaCloudAPIKey = "sk-test-key"
	cfg.ZaiAPIKey = "sk-zai"
	cfg.OpenrouterAPIKey = "sk-or"
	cfg.OllamaHost = "http://localhost:11434"
	cfg.LastModel = "glm-5"
	cfg.LastProvider = "ollama-cloud"
	cfg.ExtraModels = []ExtraModel{{Provider: "openrouter", Name: "custom-model"}}
	// Stash an unrelated key in Raw to verify we preserve it on save.
	cfg.Raw["trusted_workspaces"] = []string{"/tmp/foo", "/tmp/bar"}
	if err := cfg.Save(); err != nil {
		t.Fatalf("Save: %v", err)
	}

	// File should exist now.
	path, _ := Path()
	if _, err := os.Stat(path); err != nil {
		t.Fatalf("config file not written: %v", err)
	}

	// Reload and verify.
	reloaded, err := Load()
	if err != nil {
		t.Fatalf("reload: %v", err)
	}
	if reloaded.OllamaCloudAPIKey != "sk-test-key" {
		t.Errorf("api key = %q, want sk-test-key", reloaded.OllamaCloudAPIKey)
	}
	if reloaded.ZaiAPIKey != "sk-zai" {
		t.Errorf("zai key = %q, want sk-zai", reloaded.ZaiAPIKey)
	}
	if reloaded.OpenrouterAPIKey != "sk-or" {
		t.Errorf("openrouter key = %q, want sk-or", reloaded.OpenrouterAPIKey)
	}
	if reloaded.OllamaHost != "http://localhost:11434" {
		t.Errorf("ollama_host = %q", reloaded.OllamaHost)
	}
	if reloaded.LastModel != "glm-5" {
		t.Errorf("last_model = %q, want glm-5", reloaded.LastModel)
	}
	if !reloaded.HasOllamaCloudKey() {
		t.Error("HasOllamaCloudKey = false, want true")
	}
	if len(reloaded.ExtraModels) != 1 || reloaded.ExtraModels[0].Name != "custom-model" {
		t.Errorf("extra_models = %+v", reloaded.ExtraModels)
	}
	// Legacy key preserved.
	if got := reloaded.Raw["trusted_workspaces"]; got == nil {
		t.Error("trusted_workspaces lost on round-trip")
	}
}

// TestConfiguredProviders verifies that ConfiguredProviders returns
// only providers with non-empty keys.
func TestConfiguredProviders(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)

	cfg, _ := Load()
	cfg.OllamaCloudAPIKey = "sk-1"
	cfg.OpenrouterAPIKey = "sk-2"
	// z.ai not set — should not appear.
	cfg.Save()

	reloaded, _ := Load()
	configured := reloaded.ConfiguredProviders()
	if len(configured) != 2 {
		t.Fatalf("got %d configured, want 2: %v", len(configured), configured)
	}
	// Verify ByokKey returns the right values.
	if got := reloaded.ByokKey(OllamaCloudProvider); got != "sk-1" {
		t.Errorf("ByokKey(ollama-cloud) = %q, want sk-1", got)
	}
	if got := reloaded.ByokKey(OpenrouterProvider); got != "sk-2" {
		t.Errorf("ByokKey(openrouter) = %q, want sk-2", got)
	}
	if got := reloaded.ByokKey(ZaiSubscriptionProvider); got != "" {
		t.Errorf("ByokKey(zai-subscription) = %q, want empty", got)
	}
}

// TestSetByokKey verifies round-trip through SetByokKey + Save + Load.
func TestSetByokKey(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)

	cfg, _ := Load()
	cfg.SetByokKey(HmanlabProvider, "sk-hmanlab")
	cfg.SetByokKey(MinimaxProvider, "sk-minimax")
	cfg.Save()

	reloaded, _ := Load()
	if reloaded.HmanlabAPIKey != "sk-hmanlab" {
		t.Errorf("hmanlab key = %q", reloaded.HmanlabAPIKey)
	}
	if reloaded.MinimaxAPIKey != "sk-minimax" {
		t.Errorf("minimax key = %q", reloaded.MinimaxAPIKey)
	}
}

// TestWriteAndReadExplicitPath sanity-checks that the file we write
// is plain TOML a human could edit.
func TestWriteAndReadExplicitPath(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)

	cfg, _ := Load()
	cfg.OllamaCloudAPIKey = "sk-abc"
	cfg.Save()

	data, _ := os.ReadFile(filepath.Join(dir, "hmanlab", "config.toml"))
	if !strings.Contains(string(data), "ollama_cloud_api_key = \"sk-abc\"") {
		t.Errorf("config file missing expected key. Got:\n%s", string(data))
	}
}
