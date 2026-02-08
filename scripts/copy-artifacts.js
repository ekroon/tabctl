#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");

const root = process.cwd();

const artifacts = [
  ["src/host/host.sh", "dist/host/host.sh"],
  ["src/host/launcher", "dist/host/launcher"],
  ["src/tests/unit/fixtures", "dist/tests/unit/fixtures"],
];

for (const [src, dest] of artifacts) {
  const srcPath = path.join(root, src);
  const destPath = path.join(root, dest);
  if (!fs.existsSync(srcPath)) continue;
  fs.mkdirSync(path.dirname(destPath), { recursive: true });
  if (fs.statSync(srcPath).isDirectory()) {
    fs.cpSync(srcPath, destPath, { recursive: true });
  } else {
    fs.copyFileSync(srcPath, destPath);
  }
}

const executables = [
  path.join(root, "dist", "host", "host.sh"),
  path.join(root, "dist", "cli", "tabctl.js"),
];

for (const p of executables) {
  if (fs.existsSync(p) && process.platform !== "win32") fs.chmodSync(p, 0o755);
}

// Cross-compile Windows launcher (requires Go)
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
