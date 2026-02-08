// Tiny native messaging host launcher for Windows.
//
// Chrome launches native messaging hosts as sub-processes. On Windows, if the
// host is a .cmd/.bat file, Chrome invokes it through cmd.exe which opens
// stdin/stdout in text mode — corrupting the binary 4-byte length-prefixed
// native messaging protocol.
//
// This launcher reads a config file (host-launcher.cfg) next to the exe,
// sets environment variables, then replaces itself with node using the CRT
// _execv function, which on Windows terminates the current process and starts
// the new one with the same handles.
//
// Build: GOOS=windows GOARCH=amd64 go build -o tabctl-host.exe .

package main

import (
	"bufio"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
)

func main() {
	// Resolve config file next to this executable
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

	// Line 1: path to node executable
	if !scanner.Scan() {
		fmt.Fprintf(os.Stderr, "tabctl-host: missing node path in %s\n", cfgPath)
		os.Exit(1)
	}
	nodePath := strings.TrimSpace(scanner.Text())

	// Line 2: path to host.js
	if !scanner.Scan() {
		fmt.Fprintf(os.Stderr, "tabctl-host: missing host path in %s\n", cfgPath)
		os.Exit(1)
	}
	hostPath := strings.TrimSpace(scanner.Text())

	// Remaining lines: NAME=VALUE environment variables
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

	// Open a log file for diagnostics (stderr goes to Chrome which may not capture it)
	logPath := filepath.Join(exeDir, "launcher.log")
	logFile, err := os.OpenFile(logPath, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0644)
	if err != nil {
		fmt.Fprintf(os.Stderr, "tabctl-host: cannot open log: %v\n", err)
		logFile = os.Stderr
	}
	defer logFile.Close()

	// Launch node with pipes for stdin/stdout so we can proxy binary data
	// between Chrome and the Node host process without any text mode corruption.
	cmd := exec.Command(nodePath, hostPath)

	fmt.Fprintf(logFile, "tabctl-host: starting %s %s\n", nodePath, hostPath)

	nodeIn, err := cmd.StdinPipe()
	if err != nil {
		fmt.Fprintf(logFile, "tabctl-host: stdin pipe: %v\n", err)
		os.Exit(1)
	}
	nodeOut, err := cmd.StdoutPipe()
	if err != nil {
		fmt.Fprintf(logFile, "tabctl-host: stdout pipe: %v\n", err)
		os.Exit(1)
	}
	nodeErr, err := cmd.StderrPipe()
	if err != nil {
		fmt.Fprintf(logFile, "tabctl-host: stderr pipe: %v\n", err)
		os.Exit(1)
	}

	if err := cmd.Start(); err != nil {
		fmt.Fprintf(logFile, "tabctl-host: failed to start: %v\n", err)
		os.Exit(1)
	}
	fmt.Fprintf(logFile, "tabctl-host: node started pid=%d\n", cmd.Process.Pid)

	logFile.Sync()

	// Bidirectional proxy: Chrome stdin → Node stdin, Node stdout → Chrome stdout
	var mu sync.Mutex
	logf := func(format string, args ...interface{}) {
		mu.Lock()
		fmt.Fprintf(logFile, format+"\n", args...)
		logFile.Sync()
		mu.Unlock()
	}

	var wg sync.WaitGroup
	wg.Add(3)

	go func() {
		defer wg.Done()
		buf := make([]byte, 4096)
		for {
			n, err := os.Stdin.Read(buf)
			if n > 0 {
				logf("stdin→node %d bytes: %x", n, buf[:min(n, 32)])
				if _, werr := nodeIn.Write(buf[:n]); werr != nil {
					logf("write to node failed: %v", werr)
					break
				}
			}
			if err != nil {
				logf("stdin: %v", err)
				break
			}
		}
		nodeIn.Close()
	}()

	go func() {
		defer wg.Done()
		buf := make([]byte, 4096)
		for {
			n, err := nodeOut.Read(buf)
			if n > 0 {
				logf("node→stdout %d bytes: %x", n, buf[:min(n, 32)])
				if _, werr := os.Stdout.Write(buf[:n]); werr != nil {
					logf("stdout write failed: %v", werr)
					break
				}
			}
			if err != nil {
				logf("nodeout: %v", err)
				break
			}
		}
	}()

	go func() {
		defer wg.Done()
		scanner := bufio.NewScanner(nodeErr)
		for scanner.Scan() {
			logf("node-stderr: %s", scanner.Text())
		}
	}()

	wg.Wait()
	logf("proxy finished")

	if err := cmd.Wait(); err != nil {
		logf("node exited with error: %v", err)
		if exitErr, ok := err.(*exec.ExitError); ok {
			os.Exit(exitErr.ExitCode())
		}
		os.Exit(1)
	}
	logf("node exited cleanly")
}
