import test from "node:test";
import assert from "node:assert/strict";
import os from "os";
import path from "path";
import fs from "fs";
import {
  readExtensionVersion,
  resolveInstalledExtensionDir,
  syncExtension,
  checkExtensionSync,
  deriveExtensionId,
} from "../../shared/extension-sync";

function makeTmpDir(): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-extsync-"));
}

// --- readExtensionVersion ---

test("readExtensionVersion reads version from manifest", () => {
  const dir = makeTmpDir();
  try {
    fs.writeFileSync(
      path.join(dir, "manifest.json"),
      JSON.stringify({ version: "1.2.3" }),
      "utf-8",
    );
    assert.equal(readExtensionVersion(dir), "1.2.3");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("readExtensionVersion returns null for missing manifest", () => {
  const dir = path.join(os.tmpdir(), "tabctl-extsync-nonexistent-" + Date.now());
  assert.equal(readExtensionVersion(dir), null);
});

test("readExtensionVersion returns null for invalid JSON", () => {
  const dir = makeTmpDir();
  try {
    fs.writeFileSync(path.join(dir, "manifest.json"), "NOT VALID JSON{{{", "utf-8");
    assert.equal(readExtensionVersion(dir), null);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

// --- resolveInstalledExtensionDir ---

test("resolveInstalledExtensionDir returns dataDir/extension", () => {
  const dataDir = "/tmp/my-tabctl-data";
  const result = resolveInstalledExtensionDir(dataDir);
  assert.equal(result, path.join(dataDir, "extension"));
});

// --- syncExtension ---

test("syncExtension copies bundled to installed when not present", () => {
  const dir = makeTmpDir();
  try {
    const result = syncExtension(dir);
    assert.equal(result.synced, true);
    assert.ok(fs.existsSync(result.extensionDir));
    assert.ok(fs.existsSync(path.join(result.extensionDir, "manifest.json")));
    assert.ok(result.bundledVersion, "bundledVersion should be set");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("syncExtension skips when versions match", () => {
  const dir = makeTmpDir();
  try {
    const first = syncExtension(dir);
    assert.equal(first.synced, true);

    const second = syncExtension(dir);
    assert.equal(second.synced, false);
    assert.equal(second.bundledVersion, second.installedVersion);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

// --- checkExtensionSync ---

test("checkExtensionSync detects missing extension", () => {
  const dir = makeTmpDir();
  try {
    const result = checkExtensionSync(dir);
    assert.equal(result.needsSync, true);
    assert.equal(result.needsReload, false);
    assert.equal(result.installedVersion, null);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("checkExtensionSync detects matching versions", () => {
  const dir = makeTmpDir();
  try {
    syncExtension(dir);
    const result = checkExtensionSync(dir);
    assert.equal(result.needsSync, false);
    assert.equal(result.needsReload, false);
    assert.equal(result.bundledVersion, result.installedVersion);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

// --- deriveExtensionId ---

test("deriveExtensionId matches Chromium algorithm", () => {
  // Known test vector: SHA256 of path, first 32 hex chars, 0-f → a-p
  const id = deriveExtensionId("/Users/erwin/.local/state/tabctl/extension");
  assert.equal(id.length, 32);
  assert.match(id, /^[a-p]{32}$/);
  assert.equal(id, "mpglnmehddpkinfhheeahiicfieegcon");
});

test("deriveExtensionId produces different IDs for different paths", () => {
  const id1 = deriveExtensionId("/path/one");
  const id2 = deriveExtensionId("/path/two");
  assert.notEqual(id1, id2);
  assert.match(id1, /^[a-p]{32}$/);
  assert.match(id2, /^[a-p]{32}$/);
});
