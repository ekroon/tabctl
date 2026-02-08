#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

const root = process.cwd();
const buildRoot = path.join(root, "build");

const targets = ["extension", "cli", "host", "tests", "shared"];

function copyDir(src, dest) {
  if (!fs.existsSync(src)) {
    return;
  }

  fs.mkdirSync(dest, { recursive: true });
  fs.cpSync(src, dest, { recursive: true });
}

for (const target of targets) {
  const src = path.join(buildRoot, target);
  const dest = path.join(root, target);
  copyDir(src, dest);
}

// Copy non-TS artifacts from src/ to output dirs
const artifacts = [
  ["src/host/host.sh", "host/host.sh"],
  ["src/tests/unit/fixtures", "tests/unit/fixtures"],
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

const executableTargets = [
  path.join(root, "host", "host.sh"),
  path.join(root, "cli", "tabctl.js"),
];

for (const executablePath of executableTargets) {
  if (fs.existsSync(executablePath)) {
    fs.chmodSync(executablePath, 0o755);
  }
}
