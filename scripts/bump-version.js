#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const pkgPath = path.join(root, "package.json");

function readPackage() {
  const raw = fs.readFileSync(pkgPath, "utf8");
  return JSON.parse(raw);
}

function writePackage(pkg) {
  fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n", "utf8");
}

function parseVersion(version) {
  const match = String(version).trim().match(/^(\d+)\.(\d+)\.(\d+)(?:[-+].*)?$/);
  if (!match) {
    throw new Error(`Invalid version: ${version}`);
  }
  return { major: Number(match[1]), minor: Number(match[2]), patch: Number(match[3]) };
}

function formatVersion(parts) {
  return `${parts.major}.${parts.minor}.${parts.patch}`;
}

function bump(current, kind) {
  const next = { ...current };
  if (kind === "major") {
    next.major += 1;
    next.minor = 0;
    next.patch = 0;
  } else if (kind === "minor") {
    next.minor += 1;
    next.patch = 0;
  } else if (kind === "patch") {
    next.patch += 1;
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
process.stdout.write(`${pkg.version}\n`);
