// Tiny native messaging host launcher for Windows.
//
// Chrome launches native messaging hosts as sub-processes. On Windows, if the
// host is a .cmd/.bat file, Chrome invokes it through cmd.exe which opens
// stdin/stdout in text mode — corrupting the binary 4-byte length-prefixed
// native messaging protocol.
//
// This launcher reads a config file (host-launcher.cfg) next to the exe,
// sets environment variables, and proxies stdin/stdout between Chrome and
// Node.js in binary mode.
//
// Build: GOOS=windows GOARCH=amd64 go build -o tabctl-host.exe .

package main

import (
	"bufio"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
)

func main() {
	exePath, err := os.Executable()
	if err != nil {
		fmt.Fprintf(os.Stderr, "tabctl-host: cannot resolve exe path: %v\n", err)
		os.Exit(1)
	}
	exeDir := filepath.Dir(exePath)
	cfgPath := filepath.Join(exeDir, "host-launcher.cfg")

	f, err := os.Open(cfgPath)
	if err != nil {
		fmt.Fprintf(os.Stderr, "tabctl-host: cannot open %s: %v\n", cfgPath, err)
		os.Exit(1)
	}

	scanner := bufio.NewScanner(f)

	if !scanner.Scan() {
		fmt.Fprintf(os.Stderr, "tabctl-host: missing node path in %s\n", cfgPath)
		os.Exit(1)
	}
	nodePath := strings.TrimSpace(scanner.Text())

	if !scanner.Scan() {
		fmt.Fprintf(os.Stderr, "tabctl-host: missing host path in %s\n", cfgPath)
		os.Exit(1)
	}
	hostPath := strings.TrimSpace(scanner.Text())

	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" {
			continue
		}
		if eq := strings.IndexByte(line, '='); eq > 0 {
			os.Setenv(line[:eq], line[eq+1:])
		}
	}
	f.Close()

	// Proxy stdin/stdout via pipes to preserve binary mode.
	cmd := exec.Command(nodePath, hostPath)
	cmd.Stderr = os.Stderr

	nodeIn, err := cmd.StdinPipe()
	if err != nil {
		fmt.Fprintf(os.Stderr, "tabctl-host: stdin pipe: %v\n", err)
		os.Exit(1)
	}
	nodeOut, err := cmd.StdoutPipe()
	if err != nil {
		fmt.Fprintf(os.Stderr, "tabctl-host: stdout pipe: %v\n", err)
		os.Exit(1)
	}

	if err := cmd.Start(); err != nil {
		fmt.Fprintf(os.Stderr, "tabctl-host: failed to start: %v\n", err)
		os.Exit(1)
	}

	var wg sync.WaitGroup
	wg.Add(2)

	go func() {
		defer wg.Done()
		io.Copy(nodeIn, os.Stdin)
		nodeIn.Close()
	}()

	go func() {
		defer wg.Done()
		io.Copy(os.Stdout, nodeOut)
	}()

	wg.Wait()

	if err := cmd.Wait(); err != nil {
		if exitErr, ok := err.(*exec.ExitError); ok {
			os.Exit(exitErr.ExitCode())
		}
		os.Exit(1)
	}
}
