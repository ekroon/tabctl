#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");
const { execSync } = require("node:child_process");

const root = path.resolve(__dirname, "..");
const pkgPath = path.join(root, "package.json");
const manifestTemplatePath = path.join(root, "src", "extension", "manifest.template.json");
const manifestPath = path.join(root, "dist", "extension", "manifest.json");

function readPackageVersion() {
  const raw = fs.readFileSync(pkgPath, "utf8");
  const pkg = JSON.parse(raw);
  return typeof pkg.version === "string" ? pkg.version : "0.0.0";
}

function readGitSha() {
  try {
    return execSync("git rev-parse --short=8 HEAD", {
      cwd: root,
      stdio: ["ignore", "pipe", "ignore"],
    }).toString().trim();
  } catch {
    return null;
  }
}

function isDirty() {
  try {
    const out = execSync("git status --porcelain -uno", {
      cwd: root,
      stdio: ["ignore", "pipe", "ignore"],
    }).toString().trim();
    return out.length > 0;
  } catch {
    return false;
  }
}

function toExtensionVersion(version) {
  const base = String(version || "0.0.0")
    .split("-")[0]
    .split("+")[0]
    .trim();
  const parts = base.split(".").map((part) => {
    const n = Number.parseInt(part, 10);
    return Number.isFinite(n) && n >= 0 ? n : 0;
  });
  while (parts.length < 3) parts.push(0);
  return parts.slice(0, 4).join(".");
}

const baseVersion = readPackageVersion();
const mode = (() => {
  if (process.env.TABCTL_VERSION_MODE) {
    return process.env.TABCTL_VERSION_MODE;
  }
  if (process.env.TABCTL_DEV_VERSION) {
    return "dev";
  }
  const gitDir = path.join(root, ".git");
  return fs.existsSync(gitDir) ? "dev" : "release";
})();

let gitSha = readGitSha();
let dirty = false;
let version = baseVersion;
const devBuild = mode === "dev";

if (devBuild && gitSha) {
  dirty = isDirty();
  version = `${baseVersion}-dev.${gitSha}${dirty ? ".dirty" : ""}`;
}

fs.mkdirSync(path.dirname(manifestPath), { recursive: true });
try {
  const templateRaw = fs.readFileSync(manifestTemplatePath, "utf8");
  const manifest = JSON.parse(templateRaw);
  manifest.version = toExtensionVersion(baseVersion);
  manifest.version_name = version;
  const nextManifest = JSON.stringify(manifest, null, 2) + "\n";
  const currentManifest = fs.existsSync(manifestPath) ? fs.readFileSync(manifestPath, "utf8") : "";
  if (currentManifest !== nextManifest) {
    fs.writeFileSync(manifestPath, nextManifest, "utf8");
  }
} catch (error) {
  process.stderr.write(`[tabctl] failed to update manifest version: ${error}\n`);
}
