//go:build darwin && orbctl_integration

package main

import (
	"context"
	"crypto/rand"
	"fmt"
	"net"
	"os/exec"
	"strings"
	"testing"
	"time"
)

// This opt-in test creates and deletes only its own randomly named, isolated
// OrbStack machine. It never controls the production orby machine.
func TestOrbctlLifecycle(t *testing.T) {
	bin, err := exec.LookPath("orbctl")
	if err != nil {
		t.Fatalf("orbctl must be in PATH for this opt-in test: %v", err)
	}
	var nonce [16]byte
	if _, err := rand.Read(nonce[:]); err != nil {
		t.Fatal(err)
	}
	name := fmt.Sprintf("orby-test-%x", nonce[:])
	identity := fmt.Sprintf("%x", nonce[:])
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Minute)
	defer cancel()
	if _, err := command(ctx, bin, "info", name); err == nil {
		t.Fatalf("refusing to adopt existing machine %s", name)
	}
	t.Logf("creating disposable isolated machine %s; the first run may download NixOS", name)
	if _, err := command(ctx, bin, "create", "--isolated", "--cpus", "1", "--memory", "2G", "--disk", "20G", "nixos", name); err != nil {
		t.Fatalf("create %s: %v; if it was partly created, inspect it manually", name, err)
	}
	marked := false
	t.Cleanup(func() {
		if !marked {
			t.Errorf("refusing to delete unverified test machine %s; inspect it manually", name)
			return
		}
		cleanupCtx, cleanupCancel := context.WithTimeout(context.Background(), 2*time.Minute)
		defer cleanupCancel()
		out, err := command(cleanupCtx, bin, "run", "-m", name, "-u", "root", "cat", "/etc/orby-test-identity")
		if err != nil || strings.TrimSpace(string(out)) != identity {
			t.Errorf("refusing to delete test machine %s: identity mismatch or unavailable: %v", name, err)
			return
		}
		if _, err := command(cleanupCtx, bin, "delete", "-f", name); err != nil {
			t.Errorf("delete test machine %s: %v", name, err)
		}
	})
	if _, err := command(ctx, bin, "start", name); err != nil {
		t.Fatal(err)
	}
	if _, err := command(ctx, bin, "run", "-m", name, "-u", "root", "sh", "-c",
		`printf '%s\n' "$1" > /etc/orby-test-identity`, "sh", identity); err != nil {
		t.Fatal(err)
	}
	marked = true
	if _, err := command(ctx, bin, "stop", name); err != nil {
		t.Fatal(err)
	}
	m := &Machine{
		Config: Config{Orbctl: bin, Machine: name},
		// The stock OrbStack NixOS distribution does not expose Orby's sshd.
		// Verify the runner's address selection without requiring provisioning.
		Dial: func(_ context.Context, network, addr string) (net.Conn, error) {
			host, port, err := net.SplitHostPort(addr)
			if err != nil || network != "tcp" || port != "22" || net.ParseIP(host) == nil {
				t.Errorf("unexpected guest SSH address: %s %s: %v", network, addr, err)
			}
			client, server := net.Pipe()
			server.Close()
			return client, nil
		},
	}
	if _, err := m.Start(ctx); err != nil {
		t.Fatal(err)
	}
	if err := m.Stop(ctx); err != nil {
		t.Fatal(err)
	}
	if _, err := m.Start(ctx); err != nil {
		t.Fatal(err)
	}
	if err := m.Stop(ctx); err != nil {
		t.Fatal(err)
	}
}
