import assert from "node:assert/strict";
import fs from "fs";
import os from "os";
import path from "path";
import test from "node:test";
import { resolveConfig, resetConfig, expandEnvVars } from "../../shared/config";

/** Save and restore env vars relevant to config resolution. */
function withCleanEnv(fn: () => void) {
  const keys = [
    "XDG_CONFIG_HOME",
    "XDG_STATE_HOME",
    "TABCTL_CONFIG_DIR",
    "TABCTL_SOCKET",
  ] as const;
  const saved: Record<string, string | undefined> = {};
  for (const k of keys) saved[k] = process.env[k];

  resetConfig();
  try {
    // Clear all config-related env vars so each test starts clean
    for (const k of keys) delete process.env[k];
    fn();
  } finally {
    for (const k of keys) {
      if (saved[k] !== undefined) {
        process.env[k] = saved[k];
      } else {
        delete process.env[k];
      }
    }
    resetConfig();
  }
}

test("default XDG paths when no env vars are set", () => {
  withCleanEnv(() => {
    const home = os.homedir();
    const cfg = resolveConfig();

    assert.equal(cfg.configDir, path.join(home, ".config", "tabctl"));
    assert.equal(cfg.dataDir, path.join(home, ".local", "state", "tabctl"));
    assert.equal(cfg.socketPath, path.join(cfg.dataDir, "tabctl.sock"));
    assert.equal(cfg.undoLog, path.join(cfg.dataDir, "undo.jsonl"));
    assert.equal(cfg.policyPath, path.join(cfg.configDir, "policy.json"));
  });
});

test("XDG_CONFIG_HOME override changes configDir only", () => {
  withCleanEnv(() => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-cfg-"));
    process.env.XDG_CONFIG_HOME = tmpDir;

    const cfg = resolveConfig();
    const home = os.homedir();

    assert.equal(cfg.configDir, path.join(tmpDir, "tabctl"));
    // dataDir should still use XDG_STATE_HOME default
    assert.equal(cfg.dataDir, path.join(home, ".local", "state", "tabctl"));
    assert.equal(cfg.policyPath, path.join(cfg.configDir, "policy.json"));

    fs.rmSync(tmpDir, { recursive: true, force: true });
  });
});

test("TABCTL_CONFIG_DIR override sets configDir and dataDir", () => {
  withCleanEnv(() => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-cfgdir-"));
    process.env.TABCTL_CONFIG_DIR = tmpDir;

    const cfg = resolveConfig();

    assert.equal(cfg.configDir, tmpDir);
    assert.equal(cfg.dataDir, path.join(tmpDir, "data"));
    assert.equal(cfg.socketPath, path.join(tmpDir, "data", "tabctl.sock"));
    assert.equal(cfg.policyPath, path.join(tmpDir, "policy.json"));

    fs.rmSync(tmpDir, { recursive: true, force: true });
  });
});

test("config.json dataDir override", () => {
  withCleanEnv(() => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-cfgjson-"));
    process.env.TABCTL_CONFIG_DIR = tmpDir;

    const customData = "/custom/data";
    fs.writeFileSync(
      path.join(tmpDir, "config.json"),
      JSON.stringify({ dataDir: customData }),
      "utf-8",
    );

    const cfg = resolveConfig();

    assert.equal(cfg.configDir, tmpDir);
    assert.equal(cfg.dataDir, customData);
    assert.equal(cfg.socketPath, path.join(customData, "tabctl.sock"));
    assert.equal(cfg.undoLog, path.join(customData, "undo.jsonl"));

    fs.rmSync(tmpDir, { recursive: true, force: true });
  });
});

test("TABCTL_SOCKET override takes precedence", () => {
  withCleanEnv(() => {
    const customSocket = "/tmp/my-tabctl.sock";
    process.env.TABCTL_SOCKET = customSocket;

    const cfg = resolveConfig();

    assert.equal(cfg.socketPath, customSocket);

    // Other paths should still use defaults
    const home = os.homedir();
    assert.equal(cfg.configDir, path.join(home, ".config", "tabctl"));
  });
});

test("missing config.json does not error", () => {
  withCleanEnv(() => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-nocfg-"));
    process.env.TABCTL_CONFIG_DIR = tmpDir;

    // No config.json exists — should not throw
    const cfg = resolveConfig();
    assert.equal(cfg.configDir, tmpDir);
    assert.equal(cfg.dataDir, path.join(tmpDir, "data"));

    fs.rmSync(tmpDir, { recursive: true, force: true });
  });
});

test("malformed config.json does not error", () => {
  withCleanEnv(() => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-badjson-"));
    process.env.TABCTL_CONFIG_DIR = tmpDir;

    fs.writeFileSync(path.join(tmpDir, "config.json"), "NOT VALID JSON{{{", "utf-8");

    const cfg = resolveConfig();
    // Falls back to default dataDir for TABCTL_CONFIG_DIR
    assert.equal(cfg.dataDir, path.join(tmpDir, "data"));

    fs.rmSync(tmpDir, { recursive: true, force: true });
  });
});

test("resolveConfig caches and resetConfig clears cache", () => {
  withCleanEnv(() => {
    const a = resolveConfig();
    const b = resolveConfig();
    assert.equal(a, b, "repeated calls should return same object");

    resetConfig();
    const c = resolveConfig();
    assert.notEqual(a, c, "after resetConfig a new object should be returned");
    // Values should still be equal (same env), but different object identity
    assert.deepEqual(a, c);
  });
});

test("config.json dataDir expands $HOME", () => {
  withCleanEnv(() => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-config-"));
    fs.writeFileSync(path.join(dir, "config.json"), JSON.stringify({ dataDir: "$HOME" }), "utf-8");
    process.env.TABCTL_CONFIG_DIR = dir;

    const config = resolveConfig();
    assert.equal(config.dataDir, os.homedir());

    fs.rmSync(dir, { recursive: true, force: true });
  });
});

test("config.json dataDir expands ${HOME}", () => {
  withCleanEnv(() => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-config-"));
    fs.writeFileSync(path.join(dir, "config.json"), JSON.stringify({ dataDir: "${HOME}/my-data" }), "utf-8");
    process.env.TABCTL_CONFIG_DIR = dir;

    const config = resolveConfig();
    assert.equal(config.dataDir, path.join(os.homedir(), "my-data"));

    fs.rmSync(dir, { recursive: true, force: true });
  });
});

test("expandEnvVars leaves unknown vars unexpanded", () => {
  const result = expandEnvVars("$TABCTL_NONEXISTENT_VAR_12345/data");
  assert.equal(result, "$TABCTL_NONEXISTENT_VAR_12345/data");
});

test("config.json relative dataDir throws error", () => {
  withCleanEnv(() => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-config-"));
    fs.writeFileSync(path.join(dir, "config.json"), JSON.stringify({ dataDir: "relative/path" }), "utf-8");
    process.env.TABCTL_CONFIG_DIR = dir;

    assert.throws(() => resolveConfig(), /must be an absolute path/);

    fs.rmSync(dir, { recursive: true, force: true });
  });
});

// --- Profile-aware config tests ---

function writeProfiles(dir: string, registry: Record<string, unknown>): void {
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "profiles.json"), JSON.stringify(registry), "utf-8");
}

test("resolveConfig with profile uses profile dataDir", () => {
  withCleanEnv(() => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-profcfg-"));
    const profileDataDir = path.join(dir, "profile-data");
    writeProfiles(dir, {
      default: "myprofile",
      profiles: { myprofile: { dataDir: profileDataDir } },
    });
    process.env.TABCTL_CONFIG_DIR = dir;

    const cfg = resolveConfig("myprofile");
    assert.equal(cfg.dataDir, profileDataDir);
    assert.equal(cfg.socketPath, path.join(profileDataDir, "tabctl.sock"));
    assert.equal(cfg.undoLog, path.join(profileDataDir, "undo.jsonl"));

    fs.rmSync(dir, { recursive: true, force: true });
  });
});

test("resolveConfig with TABCTL_PROFILE env uses profile", () => {
  withCleanEnv(() => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-profenv-"));
    const profileDataDir = path.join(dir, "env-profile-data");
    writeProfiles(dir, {
      default: null,
      profiles: { envprof: { dataDir: profileDataDir } },
    });
    process.env.TABCTL_CONFIG_DIR = dir;
    process.env.TABCTL_PROFILE = "envprof";

    const cfg = resolveConfig();
    assert.equal(cfg.dataDir, profileDataDir);
    assert.equal(cfg.activeProfileName, "envprof");

    delete process.env.TABCTL_PROFILE;
    fs.rmSync(dir, { recursive: true, force: true });
  });
});

test("resolveConfig without profiles falls back to legacy", () => {
  withCleanEnv(() => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-noprof-"));
    process.env.TABCTL_CONFIG_DIR = dir;
    // No profiles.json exists

    const cfg = resolveConfig();
    assert.equal(cfg.configDir, dir);
    assert.equal(cfg.dataDir, path.join(dir, "data"));
    assert.equal(cfg.activeProfileName, undefined);

    fs.rmSync(dir, { recursive: true, force: true });
  });
});

test("resolveConfig with unknown profile name throws", () => {
  withCleanEnv(() => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-unkprof-"));
    writeProfiles(dir, {
      default: null,
      profiles: { existing: { dataDir: "/tmp/existing" } },
    });
    process.env.TABCTL_CONFIG_DIR = dir;

    assert.throws(() => resolveConfig("nonexistent"), /not found/);

    fs.rmSync(dir, { recursive: true, force: true });
  });
});

test("resolveConfig activeProfileName is set when profile active", () => {
  withCleanEnv(() => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-actprof-"));
    const profileDataDir = path.join(dir, "active-data");
    writeProfiles(dir, {
      default: "myprof",
      profiles: { myprof: { dataDir: profileDataDir } },
    });
    process.env.TABCTL_CONFIG_DIR = dir;

    const cfg = resolveConfig("myprof");
    assert.equal(cfg.activeProfileName, "myprof");

    fs.rmSync(dir, { recursive: true, force: true });
  });
});
