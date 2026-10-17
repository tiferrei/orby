//go:build !darwin

package runner

import (
	"fmt"
	"net"
	"os"
	"os/signal"
)

func activationListener(int) (net.Listener, error) { return nil, fmt.Errorf("requires macOS launchd") }
func notifySignals(ch chan os.Signal)              { signal.Notify(ch, os.Interrupt) }
func stopSignals(ch chan os.Signal)                { signal.Stop(ch) }
