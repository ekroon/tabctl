#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

const root = process.cwd();

const artifacts = [
  ["src/host/host.sh", "dist/host/host.sh"],
  ["src/host/launcher", "dist/host/launcher"],
  ["src/tests/unit/fixtures", "dist/tests/unit/fixtures"],
  ["rust/tabctl-rust-cli-readonly/target/release/tabctl-rust-cli-readonly", "dist/cli/rust/tabctl-rust-cli-readonly"],
  ["rust/tabctl-rust-cli-readonly/target/release/tabctl-rust-cli-readonly.exe", "dist/cli/rust/tabctl-rust-cli-readonly.exe"],
  ["rust/tabctl-host-mvp/target/release/tabctl-host-mvp", "dist/host/rust/tabctl-host-mvp"],
  ["rust/tabctl-host-mvp/target/release/tabctl-host-mvp.exe", "dist/host/rust/tabctl-host-mvp.exe"],
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
  path.join(root, "dist", "cli", "rust", "tabctl-rust-cli-readonly"),
  path.join(root, "dist", "host", "rust", "tabctl-host-mvp"),
];

for (const p of executables) {
  if (fs.existsSync(p) && process.platform !== "win32") fs.chmodSync(p, 0o755);
}
