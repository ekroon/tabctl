#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");
const { execSync } = require("node:child_process");

const root = path.resolve(__dirname, "..");
const pkgPath = path.join(root, "package.json");
const winPkgPath = path.join(root, "packages", "win32-x64", "package.json");
const cargoPackagePaths = [
  path.join(root, "rust", "crates", "shared", "Cargo.toml"),
  path.join(root, "rust", "crates", "host", "Cargo.toml"),
  path.join(root, "rust", "crates", "tabctl", "Cargo.toml"),
];

function readPackage() {
  const raw = fs.readFileSync(pkgPath, "utf8");
  return JSON.parse(raw);
}

function writePackage(pkg) {
  fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n", "utf8");
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

function parseVersion(version) {
  const match = String(version).trim().match(/^(\d+)\.(\d+)\.(\d+)(?:-(alpha|rc)\.(\d+))?(?:\+.*)?$/);
  if (!match) {
    throw new Error(`Invalid version: ${version}`);
  }
  return {
    major: Number(match[1]),
    minor: Number(match[2]),
    patch: Number(match[3]),
    prereleaseTag: match[4] || null,
    prereleaseNumber: match[5] ? Number(match[5]) : null,
  };
}

function formatVersion(parts) {
  const base = `${parts.major}.${parts.minor}.${parts.patch}`;
  if (!parts.prereleaseTag) return base;
  const num = Number.isInteger(parts.prereleaseNumber) ? parts.prereleaseNumber : 1;
  return `${base}-${parts.prereleaseTag}.${num}`;
}

function bump(current, kind) {
  const next = { ...current };
  if (kind === "major") {
    next.major += 1;
    next.minor = 0;
    next.patch = 0;
    next.prereleaseTag = null;
    next.prereleaseNumber = null;
  } else if (kind === "minor") {
    next.minor += 1;
    next.patch = 0;
    next.prereleaseTag = null;
    next.prereleaseNumber = null;
  } else if (kind === "patch") {
    next.patch += 1;
    next.prereleaseTag = null;
    next.prereleaseNumber = null;
  } else if (kind === "alpha") {
    if (next.prereleaseTag === "alpha") {
      next.prereleaseNumber += 1;
    } else if (next.prereleaseTag === "rc") {
      throw new Error("Cannot bump alpha from an rc prerelease");
    } else {
      next.patch += 1;
      next.prereleaseTag = "alpha";
      next.prereleaseNumber = 1;
    }
  } else if (kind === "rc") {
    if (next.prereleaseTag === "rc") {
      next.prereleaseNumber += 1;
    } else if (next.prereleaseTag === "alpha") {
      next.prereleaseTag = "rc";
      next.prereleaseNumber = 1;
    } else {
      next.patch += 1;
      next.prereleaseTag = "rc";
      next.prereleaseNumber = 1;
    }
  } else if (kind === "stable") {
    next.prereleaseTag = null;
    next.prereleaseNumber = null;
  } else {
    throw new Error(`Unknown bump kind: ${kind}`);
  }
  return next;
}

const kind = process.argv[2] || "patch";
const pkg = readPackage();
const current = parseVersion(pkg.version || "0.0.0");
const next = bump(current, kind);
pkg.version = formatVersion(next);
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
