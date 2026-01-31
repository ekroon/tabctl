#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

const root = process.cwd();
const buildRoot = path.join(root, "build");

const targets = ["extension", "cli", "host", "tests"];

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
