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

const executableTargets = [
  path.join(root, "host", "host.sh"),
  path.join(root, "cli", "tabctl.js"),
];

for (const executablePath of executableTargets) {
  if (fs.existsSync(executablePath)) {
    fs.chmodSync(executablePath, 0o755);
  }
}
