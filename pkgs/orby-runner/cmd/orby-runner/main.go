package main

import (
	"encoding/json"
	"fmt"
	"log/slog"
	"orby-runner/internal/runner"
	"os"
)

func main() {
	path := os.Getenv("ORBY_CONFIG_FILE")
	data, err := os.ReadFile(path)
	if err != nil {
		slog.Error("read configuration", "error", err)
		os.Exit(1)
	}
	var cfg runner.Config
	if err := json.Unmarshal(data, &cfg); err != nil {
		slog.Error("parse configuration", "error", err)
		os.Exit(1)
	}
	if orbctl := os.Getenv("ORBY_ORBCTL"); orbctl != "" {
		cfg.Orbctl = orbctl
	}
	if cfg.Orbctl == "" || cfg.Machine == "" || cfg.Port < 1024 || cfg.Port > 65535 || cfg.TTL < 1 {
		slog.Error(fmt.Sprintf("invalid Orby configuration: %+v", cfg))
		os.Exit(1)
	}
	if err := runner.Run(cfg); err != nil {
		slog.Error("runner stopped", "error", err)
		os.Exit(1)
	}
}
