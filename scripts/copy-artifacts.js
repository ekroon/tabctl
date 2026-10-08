#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

const root = process.cwd();

const executables = [
  path.join(root, "dist", "extension", "background.js"),
];

for (const p of executables) {
  if (fs.existsSync(p)) fs.chmodSync(p, 0o755);
}
