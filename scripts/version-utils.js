#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

function readJson(filePath) {
  return JSON.parse(fs.readFileSync(filePath, "utf8"));
}

function writeJson(filePath, value) {
  fs.writeFileSync(filePath, JSON.stringify(value, null, 2) + "\n", "utf8");
}

function readTomlSection(raw, sectionName) {
  const header = `[${sectionName}]`;
  const start = raw.indexOf(header);
  if (start === -1) {
    throw new Error(`Missing [${sectionName}] section`);
  }
  const rest = raw.slice(start + header.length);
  const nextHeader = rest.match(/\n\[[^\]]+\]/);
  return nextHeader ? rest.slice(0, nextHeader.index) : rest;
}

function readWorkspaceVersion(rootDir) {
  const cargoPath = path.join(rootDir, "rust", "Cargo.toml");
  const raw = fs.readFileSync(cargoPath, "utf8");
  const section = readTomlSection(raw, "workspace.package");
  const match = section.match(/\nversion\s*=\s*"([^"]+)"/);
  if (!match) {
    throw new Error(`Could not find [workspace.package].version in ${cargoPath}`);
  }
  return match[1];
}

function writeWorkspaceVersion(rootDir, version) {
  const cargoPath = path.join(rootDir, "rust", "Cargo.toml");
  const raw = fs.readFileSync(cargoPath, "utf8");
  const workspacePackagePattern = /(\[workspace\.package\][\s\S]*?\nversion\s*=\s*")([^"]+)(")/m;
  let next;
  if (workspacePackagePattern.test(raw)) {
    next = raw.replace(workspacePackagePattern, `$1${version}$3`);
  } else {
    next = `${raw.trimEnd()}\n\n[workspace.package]\nversion = "${version}"\n`;
  }
  fs.writeFileSync(cargoPath, next, "utf8");
}

function readCargoPackageVersion(filePath, workspaceVersion) {
  const raw = fs.readFileSync(filePath, "utf8");
  const section = readTomlSection(raw, "package");
  const workspaceMatch = section.match(/\nversion\.workspace\s*=\s*true/);
  if (workspaceMatch) {
    if (!workspaceVersion) {
      throw new Error(`Workspace version required for ${filePath}`);
    }
    return workspaceVersion;
  }
  const explicitMatch = section.match(/\nversion\s*=\s*"([^"]+)"/);
  if (explicitMatch) {
    return explicitMatch[1];
  }
  throw new Error(`Could not find package version in ${filePath}`);
}

module.exports = {
  readCargoPackageVersion,
  readJson,
  readWorkspaceVersion,
  writeJson,
  writeWorkspaceVersion,
};
