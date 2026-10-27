//go:build darwin

package main

/*
#include <launch.h>
#include <stdlib.h>
*/
import "C"
import (
	"context"
	"fmt"
	"io"
	"log/slog"
	"net"
	"os"
	"os/exec"
	"os/signal"
	"strings"
	"sync"
	"syscall"
	"time"
	"unsafe"
)

type Config struct {
	Orbctl   string `json:"orbctl"`
	Machine  string `json:"machine"`
	Port     int    `json:"port"`
	OnDemand bool   `json:"on-demand"`
	TTL      int    `json:"ttl"`
}

// Machine controls only the explicitly named Orby machine. In particular, stop
// must never be called without a machine name: that would stop all of OrbStack.
type Machine struct {
	Config Config
	Exec   func(context.Context, string, ...string) ([]byte, error)
	Dial   func(context.Context, string, string) (net.Conn, error)
	mu     sync.Mutex
	addr   string
}

func command(ctx context.Context, binary string, args ...string) ([]byte, error) {
	cmd := exec.CommandContext(ctx, binary, args...)
	out, err := cmd.CombinedOutput()
	if err != nil {
		return nil, fmt.Errorf("%s %v: %w: %s", binary, args, err, strings.TrimSpace(string(out)))
	}
	return out, nil
}

func (m *Machine) call(ctx context.Context, args ...string) ([]byte, error) {
	if m.Exec == nil {
		m.Exec = command
	}
	return m.Exec(ctx, m.Config.Orbctl, args...)
}

// IP is obtained from the named guest, rather than macOS's unrelated DHCP
// leases. Do not assume that an address remains valid across machine restarts.
func (m *Machine) IP(ctx context.Context) (string, error) {
	out, err := m.call(ctx, "run", "-m", m.Config.Machine, "-u", "root", "hostname", "-I")
	if err != nil {
		return "", err
	}
	for _, field := range strings.Fields(string(out)) {
		if ip := net.ParseIP(field); ip != nil && ip.To4() != nil {
			return ip.String(), nil
		}
	}
	return "", fmt.Errorf("no IPv4 address returned by Orby machine")
}

func (m *Machine) Start(ctx context.Context) (string, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.addr != "" {
		return m.addr, nil
	}
	if _, err := m.call(ctx, "start", m.Config.Machine); err != nil {
		return "", err
	}
	deadline, cancel := context.WithTimeout(ctx, 90*time.Second)
	defer cancel()
	var lastErr error
	for deadline.Err() == nil {
		ip, err := m.IP(deadline)
		if err == nil {
			addr := net.JoinHostPort(ip, "22")
			dial := m.Dial
			if dial == nil {
				dial = (&net.Dialer{Timeout: 2 * time.Second}).DialContext
			}
			conn, err := dial(deadline, "tcp", addr)
			if err == nil {
				conn.Close()
				m.addr = addr
				return addr, nil
			}
			lastErr = err
		} else {
			lastErr = err
		}
		select {
		case <-deadline.Done():
		case <-time.After(time.Second):
		}
	}
	return "", fmt.Errorf("SSH not ready: %v: %w", lastErr, deadline.Err())
}

func (m *Machine) Stop(ctx context.Context) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.addr = ""
	_, err := m.call(ctx, "stop", m.Config.Machine)
	return err
}

type Proxy struct {
	Machine *Machine
	TTL     time.Duration
	mu      sync.Mutex
	active  int
	timer   *time.Timer
}

func (p *Proxy) accepted() {
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.timer != nil {
		p.timer.Stop()
		p.timer = nil
	}
	p.active++
}

func (p *Proxy) released() {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.active--
	if p.active == 0 && p.Machine.Config.OnDemand {
		p.timer = time.AfterFunc(p.TTL, func() {
			p.mu.Lock()
			defer p.mu.Unlock()
			if p.active == 0 {
				ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
				defer cancel()
				if err := p.Machine.Stop(ctx); err != nil {
					slog.Error("idle stop failed", "error", err)
				}
			}
		})
	}
}

func (p *Proxy) Serve(ctx context.Context, listener net.Listener) error {
	defer listener.Close()
	if !p.Machine.Config.OnDemand {
		if _, err := p.Machine.Start(ctx); err != nil {
			return err
		}
	}
	go func() { <-ctx.Done(); listener.Close() }()
	for {
		client, err := listener.Accept()
		if err != nil {
			if ctx.Err() != nil {
				return nil
			}
			return err
		}
		p.accepted()
		go func() {
			defer client.Close()
			defer p.released()
			addr, err := p.Machine.Start(ctx)
			if err != nil {
				slog.Error("start machine", "error", err)
				return
			}
			upstream, err := net.DialTimeout("tcp", addr, 5*time.Second)
			if err != nil {
				slog.Error("connect to guest", "error", err)
				return
			}
			defer upstream.Close()
			var wg sync.WaitGroup
			wg.Add(2)
			go func() { defer wg.Done(); io.Copy(upstream, client); closeWrite(upstream) }()
			go func() { defer wg.Done(); io.Copy(client, upstream); closeWrite(client) }()
			wg.Wait()
		}()
	}
}

func closeWrite(conn net.Conn) {
	if c, ok := conn.(interface{ CloseWrite() error }); ok {
		c.CloseWrite()
	}
}

func activationListener(port int) (net.Listener, error) {
	name := C.CString("Listener")
	defer C.free(unsafe.Pointer(name))
	var fds *C.int
	var count C.size_t
	if code := C.launch_activate_socket(name, &fds, &count); code != 0 || count != 1 {
		return nil, fmt.Errorf("launchd socket activation failed: %d, count %d", code, count)
	}
	defer C.free(unsafe.Pointer(fds))
	file := os.NewFile(uintptr(*fds), "Listener")
	defer file.Close()
	listener, err := net.FileListener(file)
	if err != nil {
		return nil, err
	}
	if addr, ok := listener.Addr().(*net.TCPAddr); !ok || addr.Port != port {
		listener.Close()
		return nil, fmt.Errorf("unexpected launchd listener: %s", listener.Addr())
	}
	return listener, nil
}

func Run(cfg Config) error {
	listener, err := activationListener(cfg.Port)
	if err != nil {
		return err
	}
	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGTERM, syscall.SIGINT)
	defer stop()
	m := &Machine{Config: cfg}
	p := &Proxy{Machine: m, TTL: time.Duration(cfg.TTL) * time.Second}
	err = p.Serve(ctx, listener)
	if cfg.OnDemand {
		stopCtx, cancel := context.WithTimeout(context.Background(), time.Minute)
		defer cancel()
		if stopErr := m.Stop(stopCtx); stopErr != nil {
			slog.Error("stop machine", "error", stopErr)
		}
	}
	return err
}
