import test from "node:test";
import assert from "node:assert/strict";
import os from "node:os";
import path from "node:path";
import fs from "node:fs";
import {
  loadProfiles,
  saveProfiles,
  addProfile,
  removeProfile,
  getActiveProfile,
  listProfiles,
  validateProfileName,
  ProfileEntry,
} from "../../shared/profiles";
import { resetConfig } from "../../shared/config";

function makeTmpDir(): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-prof-"));
}

function makeEntry(overrides?: Partial<ProfileEntry>): ProfileEntry {
  return {
    browser: "edge",
    extensionId: "abc123",
    nodePath: "/usr/local/bin/node",
    hostPath: "/usr/local/lib/tabctl-host.js",
    dataDir: "/tmp/tabctl-data",
    ...overrides,
  };
}

// --- validateProfileName ---

test("validateProfileName accepts valid names", () => {
  for (const name of ["edge", "chrome-work", "my-profile-1"]) {
    validateProfileName(name); // should not throw
  }
});

test("validateProfileName rejects invalid names", () => {
  for (const name of ["Edge", "my profile", "a_b", "hi!"]) {
    assert.throws(() => validateProfileName(name), /Invalid profile name/);
  }
});

// --- loadProfiles ---

test("loadProfiles returns empty when no file", () => {
  const dir = makeTmpDir();
  try {
    const reg = loadProfiles(dir);
    assert.equal(reg.default, null);
    assert.deepEqual(reg.profiles, {});
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

// --- addProfile ---

test("addProfile creates first profile as default", () => {
  const dir = makeTmpDir();
  try {
    const reg = addProfile("edge", makeEntry(), dir);
    assert.equal(reg.default, "edge");
    assert.ok("edge" in reg.profiles);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("addProfile does not change default for subsequent profiles", () => {
  const dir = makeTmpDir();
  try {
    addProfile("edge", makeEntry(), dir);
    const reg = addProfile("chrome", makeEntry({ browser: "chrome" }), dir);
    assert.equal(reg.default, "edge");
    assert.ok("chrome" in reg.profiles);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

// --- removeProfile ---

test("removeProfile removes profile", () => {
  const dir = makeTmpDir();
  try {
    addProfile("edge", makeEntry(), dir);
    addProfile("chrome", makeEntry({ browser: "chrome" }), dir);
    const reg = removeProfile("edge", dir);
    assert.equal("edge" in reg.profiles, false);
    assert.ok("chrome" in reg.profiles);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("removeProfile reassigns default when removing default", () => {
  const dir = makeTmpDir();
  try {
    addProfile("edge", makeEntry(), dir);
    addProfile("chrome", makeEntry({ browser: "chrome" }), dir);
    const reg = removeProfile("edge", dir);
    assert.equal(reg.default, "chrome");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("removeProfile throws for unknown name", () => {
  const dir = makeTmpDir();
  try {
    assert.throws(() => removeProfile("nonexistent", dir), /does not exist/);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

// --- getActiveProfile ---

test("getActiveProfile returns default", () => {
  const dir = makeTmpDir();
  try {
    addProfile("edge", makeEntry(), dir);
    addProfile("chrome", makeEntry({ browser: "chrome" }), dir);
    const active = getActiveProfile(undefined, dir);
    assert.ok(active);
    assert.equal(active.name, "edge");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("getActiveProfile uses override", () => {
  const dir = makeTmpDir();
  try {
    addProfile("edge", makeEntry(), dir);
    addProfile("chrome", makeEntry({ browser: "chrome" }), dir);
    const active = getActiveProfile("chrome", dir);
    assert.ok(active);
    assert.equal(active.name, "chrome");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("getActiveProfile returns null when no profiles", () => {
  const dir = makeTmpDir();
  try {
    const active = getActiveProfile(undefined, dir);
    assert.equal(active, null);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("getActiveProfile reads TABCTL_PROFILE env", () => {
  const dir = makeTmpDir();
  const saved = process.env.TABCTL_PROFILE;
  try {
    addProfile("edge", makeEntry(), dir);
    addProfile("chrome", makeEntry({ browser: "chrome" }), dir);
    process.env.TABCTL_PROFILE = "chrome";
    const active = getActiveProfile(undefined, dir);
    assert.ok(active);
    assert.equal(active.name, "chrome");
  } finally {
    if (saved !== undefined) {
      process.env.TABCTL_PROFILE = saved;
    } else {
      delete process.env.TABCTL_PROFILE;
    }
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

// --- listProfiles ---

test("listProfiles returns all with default indicator", () => {
  const dir = makeTmpDir();
  try {
    addProfile("edge", makeEntry(), dir);
    addProfile("chrome", makeEntry({ browser: "chrome" }), dir);
    addProfile("dev", makeEntry({ browser: "edge" }), dir);
    const list = listProfiles(dir);
    assert.equal(list.length, 3);
    const defaultEntry = list.find((e) => e.isDefault);
    assert.ok(defaultEntry);
    assert.equal(defaultEntry.name, "edge");
    const nonDefaults = list.filter((e) => !e.isDefault);
    assert.equal(nonDefaults.length, 2);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

// --- saveProfiles / loadProfiles roundtrip ---

test("saveProfiles and loadProfiles roundtrip", () => {
  const dir = makeTmpDir();
  try {
    const registry = {
      default: "edge",
      profiles: {
        edge: makeEntry(),
        chrome: makeEntry({ browser: "chrome" as const, dataDir: "/tmp/chrome-data" }),
      },
    };
    saveProfiles(registry, dir);
    const loaded = loadProfiles(dir);
    assert.deepEqual(loaded, registry);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
