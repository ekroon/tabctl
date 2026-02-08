#!/usr/bin/env node
"use strict";

// Cross-compile the Windows native messaging host launcher.
// Requires Go to be installed (go build).

const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");

const root = process.cwd();
const launcherDir = path.join(root, "src", "host", "launcher");
const outPath = path.join(root, "packages", "win32-x64", "tabctl-host.exe");

fs.mkdirSync(path.dirname(outPath), { recursive: true });

try {
  execFileSync("go", ["build", "-o", outPath, "."], {
    cwd: launcherDir,
    stdio: "pipe",
    env: { ...process.env, GOOS: "windows", GOARCH: "amd64", CGO_ENABLED: "0" },
  });
  console.log("Windows launcher built:", outPath);
} catch (err) {
  console.error("Failed to build Windows launcher. Is Go installed?");
  console.error(err.stderr?.toString() || err.message);
  process.exit(1);
}
