//go:build darwin

package runner

/*
#include <launch.h>
#include <stdlib.h>
*/
import "C"
import (
	"fmt"
	"net"
	"os"
	"os/signal"
	"syscall"
	"unsafe"
)

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

func notifySignals(ch chan os.Signal) { signal.Notify(ch, syscall.SIGTERM, syscall.SIGINT) }
func stopSignals(ch chan os.Signal)   { signal.Stop(ch) }
