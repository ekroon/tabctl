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
process.stdout.write(`${pkg.version}\n`);
