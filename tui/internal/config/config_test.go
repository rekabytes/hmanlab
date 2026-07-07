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
	cfg.LastModel = "glm-5"
	cfg.LastProvider = "ollama-cloud"
	// Stash an unrelated key in Raw to verify we preserve it on save.
	cfg.Raw["zai_api_key"] = "sk-zai-legacy"
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
	if reloaded.LastModel != "glm-5" {
		t.Errorf("last_model = %q, want glm-5", reloaded.LastModel)
	}
	if !reloaded.HasOllamaCloudKey() {
		t.Error("HasOllamaCloudKey = false, want true")
	}
	// Legacy key preserved.
	if got := reloaded.Raw["zai_api_key"]; got != "sk-zai-legacy" {
		t.Errorf("zai_api_key lost on round-trip: got %v", got)
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
