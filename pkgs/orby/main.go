package main

import (
	"encoding/json"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
)

const configFile = "/var/lib/orby/runner.json"

func main() {
	cfg, err := readConfig(configFile)
	if err != nil {
		slog.Error("load configuration", "error", err)
		os.Exit(1)
	}
	if err := Run(cfg); err != nil {
		slog.Error("runner stopped", "error", err)
		os.Exit(1)
	}
}

func readConfig(path string) (Config, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return Config{}, err
	}
	var cfg Config
	if err := json.Unmarshal(data, &cfg); err != nil {
		return Config{}, err
	}
	if !filepath.IsAbs(cfg.Orbctl) || cfg.Machine == "" || cfg.Port < 1024 || cfg.Port > 65535 || cfg.TTL < 1 {
		return Config{}, fmt.Errorf("invalid Orby configuration: %+v", cfg)
	}
	return cfg, nil
}
