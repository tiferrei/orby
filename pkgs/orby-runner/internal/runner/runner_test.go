package runner

import (
	"context"
	"fmt"
	"net"
	"reflect"
	"sync"
	"testing"
	"time"
)

func TestMachineLifecycle(t *testing.T) {
	var mu sync.Mutex
	var calls [][]string
	m := &Machine{
		Config: Config{Orbctl: "/usr/local/bin/orbctl", Machine: "orby-builder"},
		Exec: func(_ context.Context, bin string, args ...string) ([]byte, error) {
			if bin != "/usr/local/bin/orbctl" {
				t.Fatalf("wrong binary: %q", bin)
			}
			mu.Lock()
			calls = append(calls, args)
			mu.Unlock()
			if args[0] == "run" {
				return []byte("198.19.1.5  fd07::1\n"), nil
			}
			return nil, nil
		},
		Dial: func(_ context.Context, network, addr string) (net.Conn, error) {
			if addr != "198.19.1.5:22" {
				t.Fatalf("wrong SSH endpoint: %q", addr)
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
	want := [][]string{
		{"start", "orby-builder"},
		{"run", "-m", "orby-builder", "-u", "root", "hostname", "-I"},
		{"stop", "orby-builder"},
		{"start", "orby-builder"},
		{"run", "-m", "orby-builder", "-u", "root", "hostname", "-I"},
	}
	mu.Lock()
	defer mu.Unlock()
	if !reflect.DeepEqual(calls, want) {
		t.Fatalf("unexpected commands: %v", calls)
	}
}

func TestMachineStartFailure(t *testing.T) {
	m := &Machine{
		Config: Config{Orbctl: "orbctl", Machine: "orby-builder"},
		Exec: func(_ context.Context, _ string, args ...string) ([]byte, error) {
			return nil, fmt.Errorf("no machine: %s", args[0])
		},
	}
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	if _, err := m.Start(ctx); err == nil {
		t.Fatal("expected start failure")
	}
}

func TestIdleStopIsCancelledByNewConnection(t *testing.T) {
	stopped := make(chan struct{}, 2)
	m := &Machine{
		Config: Config{Orbctl: "orbctl", Machine: "orby-builder", OnDemand: true},
		Exec: func(_ context.Context, _ string, args ...string) ([]byte, error) {
			if args[0] == "stop" {
				stopped <- struct{}{}
			}
			return nil, nil
		},
	}
	p := &Proxy{Machine: m, TTL: 50 * time.Millisecond}
	p.accepted()
	p.released()
	p.accepted()
	select {
	case <-stopped:
		t.Fatal("stopped a machine while a connection was active")
	case <-time.After(100 * time.Millisecond):
	}
	p.released()
	select {
	case <-stopped:
	case <-time.After(time.Second):
		t.Fatal("did not stop idle machine")
	}
}
