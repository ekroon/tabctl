#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");
const { execSync } = require("node:child_process");
const { bump, formatVersion, parseVersion } = require("./versioning");

const root = path.resolve(__dirname, "..");
const pkgPath = path.join(root, "package.json");
const winPkgPath = path.join(root, "packages", "win32-x64", "package.json");
const winPackageName = "tabctl-win32-x64";
const cargoPackagePaths = [
  path.join(root, "rust", "crates", "shared", "Cargo.toml"),
  path.join(root, "rust", "crates", "host", "Cargo.toml"),
  path.join(root, "rust", "crates", "graphql", "Cargo.toml"),
  path.join(root, "rust", "crates", "tabctl", "Cargo.toml"),
];

function readPackage() {
  const raw = fs.readFileSync(pkgPath, "utf8");
  return JSON.parse(raw);
}

function writePackage(pkg) {
  fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n", "utf8");
}

function syncOptionalDependencyVersion(pkg, dependencyName, version) {
  pkg.optionalDependencies = pkg.optionalDependencies || {};
  pkg.optionalDependencies[dependencyName] = version;
}

function syncJsonPackageVersion(filePath, version) {
  const raw = fs.readFileSync(filePath, "utf8");
  const parsed = JSON.parse(raw);
  parsed.version = version;
  fs.writeFileSync(filePath, JSON.stringify(parsed, null, 2) + "\n", "utf8");
}

function syncCargoPackageVersion(filePath, version) {
  const raw = fs.readFileSync(filePath, "utf8");
  const next = raw.replace(
    /(\[package\][\s\S]*?\nversion\s*=\s*")([^"]+)(")/m,
    `$1${version}$3`,
  );
  if (next === raw) {
    throw new Error(`Could not update [package].version in ${filePath}`);
  }
  fs.writeFileSync(filePath, next, "utf8");
}

const kind = process.argv[2] || "patch";
const pkg = readPackage();
const current = parseVersion(pkg.version || "0.0.0");
const next = bump(current, kind);
pkg.version = formatVersion(next);
syncOptionalDependencyVersion(pkg, winPackageName, pkg.version);
writePackage(pkg);
syncJsonPackageVersion(winPkgPath, pkg.version);
for (const cargoPath of cargoPackagePaths) {
  syncCargoPackageVersion(cargoPath, pkg.version);
}

// Sync lockfiles
execSync("npm install --package-lock-only --ignore-scripts", {
  cwd: root,
  stdio: "ignore",
});
execSync("cargo generate-lockfile --manifest-path rust/Cargo.toml", {
  cwd: root,
  stdio: "ignore",
});

process.stdout.write(`${pkg.version}\n`);
