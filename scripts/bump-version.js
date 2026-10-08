#!/usr/bin/env node
"use strict";

const path = require("path");
const { execSync } = require("node:child_process");
const { bump, formatVersion, parseVersion } = require("./versioning");
const {
  readJson,
  readWorkspaceVersion,
  writeJson,
  writeWorkspaceVersion,
} = require("./version-utils");

const root = path.resolve(__dirname, "..");
const pkgPath = path.join(root, "package.json");

function syncJsonPackageVersion(filePath, version) {
  const parsed = readJson(filePath);
  parsed.version = version;
  writeJson(filePath, parsed);
}

const kind = process.argv[2] || "patch";
const current = parseVersion(readWorkspaceVersion(root));
const next = bump(current, kind);
const nextVersion = formatVersion(next);
syncJsonPackageVersion(pkgPath, nextVersion);
writeWorkspaceVersion(root, nextVersion);

// Sync lockfiles
execSync("npm install --package-lock-only --ignore-scripts", {
  cwd: root,
  stdio: "ignore",
});
execSync("cargo generate-lockfile --manifest-path rust/Cargo.toml", {
  cwd: root,
  stdio: "ignore",
});

process.stdout.write(`${nextVersion}\n`);
