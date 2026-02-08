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
  readHostVersion,
  resolveInstalledHostPath,
  syncHost,
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

// --- readHostVersion ---

test("readHostVersion reads BASE_VERSION from bundled host", () => {
  const dir = makeTmpDir();
  try {
    const hostPath = path.join(dir, "host.bundle.js");
    fs.writeFileSync(hostPath, `
      var BASE_VERSION = "1.2.3";
      var VERSION = "1.2.3-dev.abc123.dirty";
      // rest of host code
    `, "utf-8");
    assert.equal(readHostVersion(hostPath), "1.2.3");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("readHostVersion returns null for missing file", () => {
  assert.equal(readHostVersion("/nonexistent/host.bundle.js"), null);
});

test("readHostVersion returns null when no VERSION in file", () => {
  const dir = makeTmpDir();
  try {
    const hostPath = path.join(dir, "host.bundle.js");
    fs.writeFileSync(hostPath, "console.log('hello');", "utf-8");
    assert.equal(readHostVersion(hostPath), null);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

// --- resolveInstalledHostPath ---

test("resolveInstalledHostPath returns dataDir/host.bundle.js", () => {
  const dataDir = "/tmp/my-tabctl-data";
  const result = resolveInstalledHostPath(dataDir);
  assert.equal(result, path.join(dataDir, "host.bundle.js"));
});

// --- syncHost ---

test("syncHost copies bundled host when not present", () => {
  const dir = makeTmpDir();
  try {
    const result = syncHost(dir);
    assert.equal(result.synced, true);
    assert.ok(fs.existsSync(result.hostPath));
    assert.ok(result.bundledVersion, "bundledVersion should be set");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("syncHost skips when versions match", () => {
  const dir = makeTmpDir();
  try {
    const first = syncHost(dir);
    assert.equal(first.synced, true);

    const second = syncHost(dir);
    assert.equal(second.synced, false);
    assert.equal(second.bundledVersion, second.installedVersion);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("syncHost overwrites when versions differ", () => {
  const dir = makeTmpDir();
  try {
    // First sync
    const first = syncHost(dir);
    assert.equal(first.synced, true);

    // Tamper with installed version
    const content = fs.readFileSync(first.hostPath, "utf-8");
    fs.writeFileSync(first.hostPath, content.replace(/BASE_VERSION\s*=\s*"[^"]*"/, 'BASE_VERSION = "0.0.0"'));

    // Second sync should detect mismatch and re-copy
    const second = syncHost(dir);
    assert.equal(second.synced, true);
    assert.equal(second.installedVersion, "0.0.0");
    assert.notEqual(second.bundledVersion, "0.0.0");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("synced host bundle is executable", () => {
  const dir = makeTmpDir();
  const cleanConfigHome = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-extsync-cfg-"));
  try {
    const result = syncHost(dir);
    assert.equal(result.synced, true);

    // Verify the bundle actually runs (will exit when stdin closes)
    const { spawnSync } = require("child_process");
    const proc = spawnSync(process.execPath, [result.hostPath], {
      input: "{}",
      encoding: "utf-8",
      timeout: 5000,
      env: { ...process.env, XDG_STATE_HOME: dir, XDG_CONFIG_HOME: cleanConfigHome },
    });
    // Host should start and exit cleanly when stdin closes
    assert.ok(proc.stderr.includes("Listening on") || proc.stderr.includes("Extension disconnected"),
      `Host should start and run. stderr: ${proc.stderr.slice(0, 200)}`);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
    fs.rmSync(cleanConfigHome, { recursive: true, force: true });
  }
});
