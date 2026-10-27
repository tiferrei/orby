package runner

import (
	"context"
	"net"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

// Exercise the real exec.CommandContext boundary without requiring OrbStack.
func TestOrbctlSubprocess(t *testing.T) {
	dir := t.TempDir()
	bin := filepath.Join(dir, "orbctl")
	script := `#!/bin/sh
printf '%s\n' "$*" >> "$(dirname "$0")/calls"
case "$*" in
  'start orby-test-fixture')
    if test -f "$(dirname "$0")/fail-start"; then
      echo 'machine failed to start' >&2
      exit 7
    fi
    ;;
  'stop orby-test-fixture')
    if test -f "$(dirname "$0")/fail-stop"; then
      echo 'machine failed to stop' >&2
      exit 8
    fi
    ;;
  'run -m orby-test-fixture -u root hostname -I')
    if test -f "$(dirname "$0")/no-ip"; then
      echo 'fd07::1'
    else
      echo '198.19.1.5 fd07::1'
    fi
    ;;
  *) echo "unexpected orbctl command: $*" >&2; exit 2 ;;
esac
`
	if err := os.WriteFile(bin, []byte(script), 0700); err != nil {
		t.Fatal(err)
	}
	m := &Machine{
		Config: Config{Orbctl: bin, Machine: "orby-test-fixture"},
		Dial: func(_ context.Context, network, addr string) (net.Conn, error) {
			if network != "tcp" || addr != "198.19.1.5:22" {
				t.Errorf("unexpected SSH endpoint: %s %s", network, addr)
			}
			client, server := net.Pipe()
			server.Close()
			return client, nil
		},
	}
	ctx := context.Background()
	if addr, err := m.Start(ctx); err != nil || addr != "198.19.1.5:22" {
		t.Fatalf("start: %q %v", addr, err)
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
	data, err := os.ReadFile(filepath.Join(dir, "calls"))
	if err != nil {
		t.Fatal(err)
	}
	got := strings.Split(strings.TrimSpace(string(data)), "\n")
	want := []string{
		"start orby-test-fixture",
		"run -m orby-test-fixture -u root hostname -I",
		"stop orby-test-fixture",
		"start orby-test-fixture",
		"run -m orby-test-fixture -u root hostname -I",
	}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("orbctl commands: got %q, want %q", got, want)
	}

	if err := os.WriteFile(filepath.Join(dir, "no-ip"), nil, 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := m.IP(ctx); err == nil || !strings.Contains(err.Error(), "no IPv4 address") {
		t.Fatalf("expected IPv4 error, got %v", err)
	}
	if err := os.WriteFile(filepath.Join(dir, "fail-start"), nil, 0600); err != nil {
		t.Fatal(err)
	}
	if err := m.Stop(ctx); err != nil {
		t.Fatal(err)
	}
	if _, err := m.Start(ctx); err == nil || !strings.Contains(err.Error(), "machine failed to start") {
		t.Fatalf("expected orbctl stderr on start failure, got %v", err)
	}
	if err := os.WriteFile(filepath.Join(dir, "fail-stop"), nil, 0600); err != nil {
		t.Fatal(err)
	}
	if err := m.Stop(ctx); err == nil || !strings.Contains(err.Error(), "machine failed to stop") {
		t.Fatalf("expected orbctl stderr on stop failure, got %v", err)
	}
}
