#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

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
