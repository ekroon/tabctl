#!/usr/bin/env node
"use strict";

// Offline, browser-free verification of the actual npm tarball and native bin link.
const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const { verifyDistribution } = require("./package-npm");
const { readWorkspaceVersion } = require("./version-utils");

const root = path.resolve(__dirname, "..");
const destination = path.join(root, "build", "npm-package");
fs.mkdirSync(destination, { recursive: true });
verifyDistribution(root);
const packed = JSON.parse(execFileSync("npm", [
  "pack", "--json", "--ignore-scripts", "--pack-destination", destination,
], { cwd: root, encoding: "utf8" }))[0];
for (const file of ["dist/npm/tabctl", "dist/extension/manifest.json", "dist/extension/background.js"]) {
  assert.ok(packed.files.some((entry) => entry.path === file && entry.size > 0), `Missing packed ${file}`);
}
assert.ok(packed.files.find((entry) => entry.path === "dist/npm/tabctl").mode & 0o111);
const archive = path.join(destination, packed.filename);
const scratch = fs.mkdtempSync(path.join(destination, "verify-"));

function confined(file) {
  const resolved = fs.realpathSync(file);
  assert.ok(resolved.startsWith(`${fs.realpathSync(scratch)}${path.sep}`), `Path escaped sandbox: ${resolved}`);
  return resolved;
}

try {
  execFileSync("tar", ["-xzf", archive, "-C", scratch]);
  const extracted = path.join(scratch, "package");
  verifyDistribution(extracted);
  const manifest = JSON.parse(fs.readFileSync(path.join(extracted, "package.json"), "utf8"));
  const version = readWorkspaceVersion(root);
  assert.equal(manifest.version, version);
  assert.notEqual(manifest.private, true);
  assert.deepEqual(manifest.os, ["darwin"]);
  assert.deepEqual([...manifest.cpu].sort(), ["arm64", "x64"]);
  assert.deepEqual(manifest.bin, { tabctl: "dist/npm/tabctl" });
  assert.equal(manifest.optionalDependencies, undefined);
  assert.equal(manifest.scripts.postinstall, undefined);
  const extensionManifest = JSON.parse(fs.readFileSync(path.join(extracted, "dist", "extension", "manifest.json"), "utf8"));
  assert.equal(extensionManifest.background.service_worker, "background.js");

  const env = { ...process.env };
  for (const key of Object.keys(env)) {
    if (key.startsWith("TABCTL_") || key.startsWith("XDG_")) delete env[key];
  }
  env.HOME = path.join(scratch, "home");
  fs.mkdirSync(env.HOME);
  const install = path.join(scratch, "install");
  execFileSync("npm", [
    "install", "--prefix", install, "--offline", "--ignore-scripts", "--no-audit", "--no-fund", archive,
  ], { cwd: scratch, env, stdio: "pipe" });
  const bin = path.join(install, "node_modules", ".bin", "tabctl");
  const native = confined(bin);
  assert.equal(path.basename(native), "tabctl");
  assert.ok(execFileSync(bin, ["--version"], { cwd: scratch, env, encoding: "utf8" }).trim().endsWith(version));

  for (const browser of ["edge", "chrome"]) {
    const sandbox = path.join(scratch, browser);
    const config = path.join(sandbox, "config");
    const state = path.join(sandbox, "state");
    const browserData = path.join(sandbox, "browser");
    const setup = JSON.parse(execFileSync(bin, [
      "setup", "--browser", browser, "--name", `npm-check-${browser}`,
      "--user-data-dir", browserData, "--skip-extension-download", "--json",
    ], {
      cwd: scratch,
      env: { ...env, TABCTL_CONFIG_DIR: config, TABCTL_DATA_DIR: state, TABCTL_AUTO_SYNC_MODE: "off" },
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    })).data;
    assert.equal(setup.extensionReleaseAsset.reason, "local-source");
    assert.equal(setup.extensionSync.ok, true);
    assert.deepEqual(setup.warnings, []);
    const active = confined(setup.extensionSync.activePath);
    assert.ok(fs.existsSync(path.join(active, "background.js")));
    const wrapper = confined(setup.wrapperPath);
    assert.ok(fs.readFileSync(wrapper, "utf8").includes(`exec "${native}" host`));
    const hostManifestPath = confined(setup.manifestPath);
    assert.equal(hostManifestPath, path.join(fs.realpathSync(browserData), "NativeMessagingHosts", "com.erwinkroon.tabctl.json"));
    const hostManifest = JSON.parse(fs.readFileSync(hostManifestPath, "utf8"));
    assert.equal(hostManifest.path, wrapper);
    assert.deepEqual(hostManifest.allowed_origins, [`chrome-extension://${setup.extensionId}/`]);
    const profiles = JSON.parse(fs.readFileSync(path.join(config, "profiles.json"), "utf8"));
    assert.equal(profiles.profiles[`npm-check-${browser}`].nodePath, native);
  }
  console.log(`Verified packed ${packed.name}@${version}: universal slices, offline npm native bin, --version, Edge/Chrome sandbox setup`);
  console.log(`Package ready: ${archive}`);
} finally {
  fs.rmSync(scratch, { recursive: true, force: true });
}
