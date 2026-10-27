package main

import (
	"os"
	"path/filepath"
	"testing"
)

func TestReadConfig(t *testing.T) {
	path := filepath.Join(t.TempDir(), "runner.json")
	if err := os.WriteFile(path, []byte(`{"orbctl":"/opt/homebrew/bin/orbctl","machine":"orby","port":31222,"on-demand":true,"ttl":900}`), 0600); err != nil {
		t.Fatal(err)
	}
	t.Setenv("ORBY_ORBCTL", "/should/not/override/config")
	cfg, err := readConfig(path)
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Orbctl != "/opt/homebrew/bin/orbctl" || cfg.Machine != "orby" || cfg.Port != 31222 || !cfg.OnDemand || cfg.TTL != 900 {
		t.Fatalf("unexpected configuration: %+v", cfg)
	}
}

func TestReadConfigRejectsMissingOrRelativeOrbctl(t *testing.T) {
	path := filepath.Join(t.TempDir(), "runner.json")
	for _, value := range []string{"", "orbctl"} {
		if err := os.WriteFile(path, []byte(`{"orbctl":"`+value+`","machine":"orby","port":31222,"ttl":900}`), 0600); err != nil {
			t.Fatal(err)
		}
		if _, err := readConfig(path); err == nil {
			t.Fatalf("accepted orbctl path %q", value)
		}
	}
}

func TestReadConfigRejectsMissingFile(t *testing.T) {
	if _, err := readConfig(filepath.Join(t.TempDir(), "missing.json")); err == nil {
		t.Fatal("accepted missing configuration")
	}
}

func TestReadConfigRejectsMalformedJSON(t *testing.T) {
	path := filepath.Join(t.TempDir(), "runner.json")
	if err := os.WriteFile(path, []byte(`{"orbctl":`), 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := readConfig(path); err == nil {
		t.Fatal("accepted malformed configuration")
	}
}
