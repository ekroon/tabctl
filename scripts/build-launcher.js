#!/usr/bin/env node
"use strict";

// Build the Windows native messaging host launcher from the Rust workspace.

const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");

const root = process.cwd();
const rustDir = path.join(root, "rust");
const manifestPath = path.join(rustDir, "Cargo.toml");
const target = process.env.TABCTL_WINDOWS_RUST_TARGET
  || (process.platform === "win32" ? "x86_64-pc-windows-msvc" : "x86_64-pc-windows-gnu");
const outPath = path.join(root, "packages", "win32-x64", "tabctl-host.exe");
const builtPath = path.join(rustDir, "target", target, "release", "tabctl.exe");

fs.mkdirSync(path.dirname(outPath), { recursive: true });

try {
  execFileSync("cargo", ["build", "--manifest-path", manifestPath, "-p", "tabctl", "--release", "--target", target], {
    cwd: root,
    stdio: "pipe",
    env: { ...process.env },
  });
  fs.copyFileSync(builtPath, outPath);
  console.log("Windows launcher built:", outPath);
} catch (err) {
  console.error("Failed to build Windows launcher with Rust.");
  console.error(err.stderr?.toString() || err.message);
  process.exit(1);
}
