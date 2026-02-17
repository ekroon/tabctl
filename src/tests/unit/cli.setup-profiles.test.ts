import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { runCli, runCliWithStdin, parseOutput } from "./cli-helpers";
import { startMockSocket, stopMockSocket } from "./socket";
import { canonicalizeExtensionPath } from "../../shared/extension-sync";

async function withWindowsSetupSocket<T>(
  envOverrides: Record<string, string>,
  fn: (env: Record<string, string>) => Promise<T>,
): Promise<T> {
  if (process.platform !== "win32") {
    return fn(envOverrides);
  }
  const mock = await startMockSocket((request) => ({
    ok: true,
    action: request.action,
    requestId: request.id,
    data: { now: Date.now() },
  }));
  try {
    return await fn({ ...envOverrides, TABCTL_SOCKET: mock.socketPath });
  } finally {
    await stopMockSocket(mock.server, mock.socketPath, mock.sockets);
  }
}

async function runSetupCli(args: string[], envOverrides: Record<string, string>) {
  return withWindowsSetupSocket(envOverrides, (env) => runCli(args, undefined, env));
}

async function runSetupCliWithStdin(args: string[], stdinData: string, envOverrides: Record<string, string>) {
  return withWindowsSetupSocket(envOverrides, (env) => runCliWithStdin(args, stdinData, env));
}

test("setup writes native host manifest", async () => {
  const homeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-setup-"));
  const extensionId = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
  const nodePath = process.execPath;
  const result = await runSetupCli([
    "setup",
    "--browser",
    "edge",
    "--extension-id",
    extensionId,
    "--node",
    nodePath,
  ], { HOME: homeDir, XDG_STATE_HOME: path.join(homeDir, ".local", "state"), XDG_CONFIG_HOME: path.join(homeDir, ".config") });

  assert.equal(result.status, 0);
  const output = parseOutput(result) as { ok: boolean; action?: string; data: Record<string, unknown> };
  assert.equal(output.ok, true);
  assert.equal(output.action, "setup");
  assert.equal(output.data.profileName, "edge");
  if (process.platform === "win32") {
    const verification = output.data.verification as { attempted?: boolean; ok?: boolean };
    assert.equal(verification.attempted, true);
    assert.equal(verification.ok, true);
  }

  const isWin = process.platform === "win32";
  const wrapperPath = output.data.wrapperPath as string;
  const profileDir = path.join(homeDir, ".local", "state", "tabctl", "profiles", "edge");
  assert.ok(wrapperPath.startsWith(profileDir), "wrapper should be under profile dir");
  if (isWin) {
    // .exe when platform package is available, .cmd fallback otherwise
    assert.ok(wrapperPath.endsWith(".exe") || wrapperPath.endsWith(".cmd"), "Windows wrapper should be .exe or .cmd");
  } else {
    assert.ok(wrapperPath.endsWith(".sh"), "Unix wrapper should be .sh");
  }
  assert.ok(fs.existsSync(wrapperPath));

  // Manifest path depends on platform; just verify the file exists
  const manifestPath = output.data.manifestPath as string;
  assert.ok(fs.existsSync(manifestPath));

  // Manifest uses standard host name
  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8")) as { name?: string; path?: string; allowed_origins?: string[] };
  assert.equal(manifest.name, "com.erwinkroon.tabctl");
  assert.equal(manifest.path, wrapperPath);
  assert.deepEqual(manifest.allowed_origins, [`chrome-extension://${extensionId}/`]);

  const hostPath = path.join(homeDir, ".local", "state", "tabctl", "host.bundle.js");
  if (wrapperPath.endsWith(".exe")) {
    // .exe uses a .cfg file, not embedded paths
    const cfgPath = path.join(path.dirname(wrapperPath), "host-launcher.cfg");
    assert.ok(fs.existsSync(cfgPath), "cfg file should exist for .exe wrapper");
    const cfg = fs.readFileSync(cfgPath, "utf8");
    assert.ok(cfg.includes(nodePath));
    assert.ok(cfg.includes(hostPath));
    assert.ok(cfg.includes("TABCTL_PROFILE=edge"));
  } else {
    const wrapper = fs.readFileSync(wrapperPath, "utf8");
    assert.ok(wrapper.includes(nodePath));
    assert.ok(wrapper.includes(hostPath));
    if (isWin) {
      assert.ok(wrapper.includes("set TABCTL_PROFILE=edge"));
    } else {
      assert.ok(wrapper.includes('export TABCTL_PROFILE="edge"'));
    }
  }

  // Profile registered
  assert.equal(output.data.isDefault, true);
  const profilesPath = path.join(homeDir, ".config", "tabctl", "profiles.json");
  assert.ok(fs.existsSync(profilesPath));
  const profiles = JSON.parse(fs.readFileSync(profilesPath, "utf8"));
  assert.equal(profiles.default, "edge");
  assert.ok(profiles.profiles.edge);
});

test("setup writes native host manifest for chrome", async () => {
  const homeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-setup-chrome-"));
  const extensionId = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
  const nodePath = process.execPath;
  const result = await runSetupCli([
    "setup",
    "--browser",
    "chrome",
    "--extension-id",
    extensionId,
    "--node",
    nodePath,
  ], { HOME: homeDir, XDG_STATE_HOME: path.join(homeDir, ".local", "state"), XDG_CONFIG_HOME: path.join(homeDir, ".config") });

  assert.equal(result.status, 0);
  const output = parseOutput(result) as { ok: boolean; action?: string; data: Record<string, unknown> };
  assert.equal(output.ok, true);
  assert.equal(output.action, "setup");
  assert.equal(output.data.profileName, "chrome");

  const isWin = process.platform === "win32";
  const wrapperPath = output.data.wrapperPath as string;
  const profileDir = path.join(homeDir, ".local", "state", "tabctl", "profiles", "chrome");
  assert.ok(wrapperPath.startsWith(profileDir), "wrapper should be under profile dir");
  if (isWin) {
    assert.ok(wrapperPath.endsWith(".exe") || wrapperPath.endsWith(".cmd"), "Windows wrapper should be .exe or .cmd");
  } else {
    assert.ok(wrapperPath.endsWith(".sh"), "Unix wrapper should be .sh");
  }
  assert.ok(fs.existsSync(wrapperPath));

  // Manifest path depends on platform; just verify the file exists
  const manifestPath = output.data.manifestPath as string;
  assert.ok(fs.existsSync(manifestPath));

  // Manifest uses standard host name
  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8")) as { name?: string; path?: string; allowed_origins?: string[] };
  assert.equal(manifest.name, "com.erwinkroon.tabctl");
  assert.equal(manifest.path, wrapperPath);
  assert.deepEqual(manifest.allowed_origins, [`chrome-extension://${extensionId}/`]);

  const hostPath = path.join(homeDir, ".local", "state", "tabctl", "host.bundle.js");
  if (wrapperPath.endsWith(".exe")) {
    const cfgPath = path.join(path.dirname(wrapperPath), "host-launcher.cfg");
    assert.ok(fs.existsSync(cfgPath), "cfg file should exist for .exe wrapper");
    const cfg = fs.readFileSync(cfgPath, "utf8");
    assert.ok(cfg.includes(nodePath));
    assert.ok(cfg.includes(hostPath));
    assert.ok(cfg.includes("TABCTL_PROFILE=chrome"));
  } else {
    const wrapper = fs.readFileSync(wrapperPath, "utf8");
    assert.ok(wrapper.includes(nodePath));
    assert.ok(wrapper.includes(hostPath));
    if (isWin) {
      assert.ok(wrapper.includes("set TABCTL_PROFILE=chrome"));
    } else {
      assert.ok(wrapper.includes('export TABCTL_PROFILE="chrome"'));
    }
  }

  // Profile registered with browser: "chrome"
  assert.equal(output.data.isDefault, true);
  const profilesPath = path.join(homeDir, ".config", "tabctl", "profiles.json");
  assert.ok(fs.existsSync(profilesPath));
  const profiles = JSON.parse(fs.readFileSync(profilesPath, "utf8"));
  assert.equal(profiles.default, "chrome");
  assert.ok(profiles.profiles.chrome);
  assert.equal(profiles.profiles.chrome.browser, "chrome");
});

test("setup --user-data-dir writes manifest to custom path", async () => {
  const homeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-setup-udd-"));
  const userDataDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-udd-chrome-"));
  const extensionId = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
  const nodePath = process.execPath;
  const result = await runSetupCli([
    "setup",
    "--browser",
    "chrome",
    "--extension-id",
    extensionId,
    "--node",
    nodePath,
    "--user-data-dir",
    userDataDir,
  ], { HOME: homeDir, XDG_STATE_HOME: path.join(homeDir, ".local", "state"), XDG_CONFIG_HOME: path.join(homeDir, ".config") });

  assert.equal(result.status, 0);
  const output = parseOutput(result) as { ok: boolean; action?: string; data: Record<string, unknown> };
  assert.equal(output.ok, true);
  assert.equal(output.action, "setup");
  assert.equal(output.data.profileName, "chrome");

  // Manifest written to userDataDir, NOT the system-wide path
  const manifestPath = path.join(userDataDir, "NativeMessagingHosts", "com.erwinkroon.tabctl.json");
  assert.equal(output.data.manifestPath, manifestPath);
  assert.ok(fs.existsSync(manifestPath), "manifest should exist in userDataDir");
  assert.ok(manifestPath.startsWith(userDataDir), "manifest should be under userDataDir, not system-wide");

  // Manifest has correct allowed_origins
  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8")) as { name?: string; path?: string; allowed_origins?: string[] };
  assert.equal(manifest.name, "com.erwinkroon.tabctl");
  assert.deepEqual(manifest.allowed_origins, [`chrome-extension://${extensionId}/`]);

  // Profile registered with browser "chrome"
  const profilesPath = path.join(homeDir, ".config", "tabctl", "profiles.json");
  assert.ok(fs.existsSync(profilesPath));
  const profiles = JSON.parse(fs.readFileSync(profilesPath, "utf8"));
  assert.ok(profiles.profiles.chrome);
  assert.equal(profiles.profiles.chrome.browser, "chrome");

  // Output JSON includes userDataDir
  assert.equal(output.data.userDataDir, userDataDir);
});

// --- Profile integration tests ---

test("profile-list shows empty when no profiles", async () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-profile-test-"));
  const result = await runCli(["profile-list", "--json"], undefined, {
    TABCTL_CONFIG_DIR: tmpDir,
  });
  fs.rmSync(tmpDir, { recursive: true, force: true });

  assert.equal(result.status, 0);
  const output = parseOutput(result);
  assert.equal(output.ok, true);
  assert.equal(output.action, "profile-list");
  assert.deepEqual(output.data.profiles, []);
});

test("profile-show shows legacy mode when no profiles", async () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-profile-test-"));
  const result = await runCli(["profile-show", "--json"], undefined, {
    TABCTL_CONFIG_DIR: tmpDir,
  });
  fs.rmSync(tmpDir, { recursive: true, force: true });

  assert.equal(result.status, 0);
  const output = parseOutput(result);
  assert.equal(output.ok, true);
  assert.equal(output.action, "profile-show");
  assert.equal(output.data.mode, "legacy");
});

test("profile-switch fails for unknown profile", async () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-profile-test-"));
  const result = await runCli(["profile-switch", "nonexistent"], undefined, {
    TABCTL_CONFIG_DIR: tmpDir,
  });
  fs.rmSync(tmpDir, { recursive: true, force: true });

  assert.notEqual(result.status, 0);
});

test("profile-show with configured profile shows profile name", async () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-profile-test-"));
  const profiles = {
    default: "test-profile",
    profiles: {
      "test-profile": {
        browser: "edge",
        extensionId: "test-ext-id",
        nodePath: process.execPath,
        hostPath: "/tmp/fake-host.js",
        dataDir: path.join(tmpDir, "data"),
      },
    },
  };
  fs.writeFileSync(path.join(tmpDir, "profiles.json"), JSON.stringify(profiles, null, 2));

  const result = await runCli(["profile-show", "--json"], undefined, {
    TABCTL_CONFIG_DIR: tmpDir,
  });
  fs.rmSync(tmpDir, { recursive: true, force: true });

  assert.equal(result.status, 0);
  const output = parseOutput(result);
  assert.equal(output.ok, true);
  assert.equal(output.action, "profile-show");
  assert.equal(output.data.name, "test-profile");
  assert.equal(output.data.browser, "edge");
});

// --- Additional profile integration tests ---

function makeTwoProfileConfig(tmpDir: string) {
  const profiles = {
    default: "edge",
    profiles: {
      edge: {
        browser: "edge",
        extensionId: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        nodePath: "/usr/bin/node",
        hostPath: "/tmp/fake-host.js",
        dataDir: path.join(tmpDir, "data-edge"),
      },
      chrome: {
        browser: "chrome",
        extensionId: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        nodePath: "/usr/bin/node",
        hostPath: "/tmp/fake-host.js",
        dataDir: path.join(tmpDir, "data-chrome"),
      },
    },
  };
  fs.writeFileSync(path.join(tmpDir, "profiles.json"), JSON.stringify(profiles, null, 2));
  return profiles;
}

test("profile-switch success updates default", async () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-profile-test-"));
  try {
    makeTwoProfileConfig(tmpDir);
    const result = await runCli(["profile-switch", "chrome"], undefined, {
      TABCTL_CONFIG_DIR: tmpDir,
    });

    assert.equal(result.status, 0);
    const output = parseOutput(result);
    assert.equal(output.ok, true);
    assert.equal(output.action, "profile-switch");
    assert.equal(output.data.name, "chrome");

    const updated = JSON.parse(fs.readFileSync(path.join(tmpDir, "profiles.json"), "utf8"));
    assert.equal(updated.default, "chrome");
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

test("profile-remove success removes profile", async () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-profile-test-"));
  try {
    makeTwoProfileConfig(tmpDir);
    const result = await runCli(["profile-remove", "chrome"], undefined, {
      TABCTL_CONFIG_DIR: tmpDir,
    });

    assert.equal(result.status, 0);
    const output = parseOutput(result);
    assert.equal(output.ok, true);
    assert.equal(output.action, "profile-remove");

    const updated = JSON.parse(fs.readFileSync(path.join(tmpDir, "profiles.json"), "utf8"));
    assert.equal(updated.default, "edge");
    assert.ok(updated.profiles.edge);
    assert.equal(updated.profiles.chrome, undefined);
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

test("--profile flag overrides active profile", async () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-profile-test-"));
  try {
    makeTwoProfileConfig(tmpDir);
    const result = await runCli(["profile-show", "--profile", "chrome", "--json"], undefined, {
      TABCTL_CONFIG_DIR: tmpDir,
    });

    assert.equal(result.status, 0);
    const output = parseOutput(result);
    assert.equal(output.data.name, "chrome");
    assert.equal(output.data.browser, "chrome");
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

test("TABCTL_PROFILE env overrides active profile", async () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-profile-test-"));
  try {
    makeTwoProfileConfig(tmpDir);
    const result = await runCli(["profile-show", "--json"], undefined, {
      TABCTL_CONFIG_DIR: tmpDir,
      TABCTL_PROFILE: "chrome",
    });

    assert.equal(result.status, 0);
    const output = parseOutput(result);
    assert.equal(output.data.name, "chrome");
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

test("setup --name creates custom-named profile", async () => {
  const homeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-setup-name-"));
  try {
    const extensionId = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const result = await runSetupCli([
      "setup",
      "--browser", "edge",
      "--extension-id", extensionId,
      "--name", "my-edge",
      "--node", process.execPath,
    ], {
      HOME: homeDir,
      XDG_STATE_HOME: path.join(homeDir, ".local", "state"),
      XDG_CONFIG_HOME: path.join(homeDir, ".config"),
    });

    assert.equal(result.status, 0);
    const output = parseOutput(result);
    assert.equal(output.ok, true);
    assert.equal(output.data.profileName, "my-edge");

    const profilesPath = path.join(homeDir, ".config", "tabctl", "profiles.json");
    const profiles = JSON.parse(fs.readFileSync(profilesPath, "utf8"));
    assert.ok(profiles.profiles["my-edge"]);
  } finally {
    fs.rmSync(homeDir, { recursive: true, force: true });
  }
});

test("output includes profile and browser fields", async () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-profile-test-"));
  try {
    const profiles = {
      default: "edge",
      profiles: {
        edge: {
          browser: "edge",
          extensionId: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
          nodePath: "/usr/bin/node",
          hostPath: "/tmp/fake-host.js",
          dataDir: path.join(tmpDir, "data"),
        },
      },
    };
    fs.writeFileSync(path.join(tmpDir, "profiles.json"), JSON.stringify(profiles, null, 2));

    const result = await runCli(["profile-list", "--json"], undefined, {
      TABCTL_CONFIG_DIR: tmpDir,
    });

    assert.equal(result.status, 0);
    const output = parseOutput(result);
    assert.equal(output.profile, "edge");
    assert.equal(output.browser, "edge");
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

test("profile-list with multiple profiles shows all", async () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-profile-test-"));
  try {
    const profiles = {
      default: "edge",
      profiles: {
        edge: {
          browser: "edge",
          extensionId: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
          nodePath: "/usr/bin/node",
          hostPath: "/tmp/fake-host.js",
          dataDir: path.join(tmpDir, "data-edge"),
        },
        chrome: {
          browser: "chrome",
          extensionId: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
          nodePath: "/usr/bin/node",
          hostPath: "/tmp/fake-host.js",
          dataDir: path.join(tmpDir, "data-chrome"),
        },
        "chrome-work": {
          browser: "chrome",
          extensionId: "cccccccccccccccccccccccccccccccc",
          nodePath: "/usr/bin/node",
          hostPath: "/tmp/fake-host.js",
          dataDir: path.join(tmpDir, "data-chrome-work"),
        },
      },
    };
    fs.writeFileSync(path.join(tmpDir, "profiles.json"), JSON.stringify(profiles, null, 2));

    const result = await runCli(["profile-list", "--json"], undefined, {
      TABCTL_CONFIG_DIR: tmpDir,
    });

    assert.equal(result.status, 0);
    const output = parseOutput(result);
    assert.equal(output.data.profiles.length, 3);

    const defaultProfile = output.data.profiles.find((p: { name: string }) => p.name === "edge");
    assert.ok(defaultProfile);
    assert.equal(defaultProfile.isDefault, true);

    const nonDefaults = output.data.profiles.filter((p: { isDefault: boolean }) => !p.isDefault);
    assert.equal(nonDefaults.length, 2);
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

test("profile-show isDefault is false when using --profile override", async () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-profile-test-"));
  try {
    makeTwoProfileConfig(tmpDir);
    const result = await runCli(["profile-show", "--profile", "chrome", "--json"], undefined, {
      TABCTL_CONFIG_DIR: tmpDir,
    });

    assert.equal(result.status, 0);
    const output = parseOutput(result);
    assert.equal(output.data.name, "chrome");
    assert.equal(output.data.isDefault, false);
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

// Basic setup is covered by "setup writes native host manifest" and
// "setup writes native host manifest for chrome" tests above (both pass --extension-id).

test("setup explicit --extension-id overrides auto-derived ID", async () => {
  const homeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-setup-interactive-"));
  try {
    const extensionId = "cccccccccccccccccccccccccccccccc";
    const envOverrides = {
      HOME: homeDir,
      XDG_STATE_HOME: path.join(homeDir, ".local", "state"),
      XDG_CONFIG_HOME: path.join(homeDir, ".config"),
    };
    const result = await runSetupCliWithStdin(
      ["setup", "--browser", "chrome", "--node", process.execPath, "--extension-id", extensionId],
      "",
      envOverrides,
    );

    assert.equal(result.status, 0, `expected exit 0, stderr: ${result.stderr}`);
    const output = parseOutput(result) as { ok: boolean; action?: string; data: Record<string, unknown> };
    assert.equal(output.ok, true);
    assert.equal(output.action, "setup");
    assert.equal(output.data.extensionId, extensionId);
    assert.equal(output.data.profileName, "chrome");

    // Manifest written with correct allowed_origins
    const manifestPath = output.data.manifestPath as string;
    assert.ok(fs.existsSync(manifestPath));
    const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8")) as { allowed_origins?: string[] };
    assert.deepEqual(manifest.allowed_origins, [`chrome-extension://${extensionId}/`]);

    // Profile registered
    const profilesPath = path.join(homeDir, ".config", "tabctl", "profiles.json");
    assert.ok(fs.existsSync(profilesPath));
    const profiles = JSON.parse(fs.readFileSync(profilesPath, "utf8"));
    assert.ok(profiles.profiles.chrome);
    assert.equal(profiles.profiles.chrome.extensionId, extensionId);
  } finally {
    fs.rmSync(homeDir, { recursive: true, force: true });
  }
});

test("setup auto-derived extension ID matches Chromium algorithm", async () => {
  const homeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-setup-derive-algo-"));
  try {
    const envOverrides = {
      HOME: homeDir,
      XDG_STATE_HOME: path.join(homeDir, ".local", "state"),
      XDG_CONFIG_HOME: path.join(homeDir, ".config"),
    };
    const result = await runSetupCliWithStdin(
      ["setup", "--browser", "chrome", "--node", process.execPath],
      "",
      envOverrides,
    );

    assert.equal(result.status, 0, `expected exit 0, stderr: ${result.stderr}`);
    const output = parseOutput(result) as { ok: boolean; data: Record<string, unknown> };
    assert.equal(output.ok, true);

    // Verify the derived ID matches the Chromium SHA256-based algorithm
    const extensionDir = path.join(homeDir, ".local", "state", "tabctl", "extension");
    const crypto = require("node:crypto");
    const hashInput = canonicalizeExtensionPath(extensionDir);
    const hash = crypto.createHash("sha256").update(hashInput).digest("hex").slice(0, 32);
    const expectedId = hash.split("").map((c: string) => String.fromCharCode("a".charCodeAt(0) + parseInt(c, 16))).join("");
    assert.equal(output.data.extensionId, expectedId);
  } finally {
    fs.rmSync(homeDir, { recursive: true, force: true });
  }
});

test("setup without --extension-id falls back to auto-derived extension ID", async () => {
  const homeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-setup-derive-"));
  try {
    const envOverrides = {
      HOME: homeDir,
      XDG_STATE_HOME: path.join(homeDir, ".local", "state"),
      XDG_CONFIG_HOME: path.join(homeDir, ".config"),
    };
    // No --extension-id: auto-derive from synced extension path
    const result = await runSetupCliWithStdin(
      ["setup", "--browser", "chrome", "--node", process.execPath],
      "",
      envOverrides,
    );

    assert.equal(result.status, 0, "expected success via auto-derived ID");
    assert.ok(
      result.stderr.includes("Extension ID derived from:"),
      "expected auto-derive message on stderr",
    );
    assert.ok(
      result.stderr.includes("Derived extension ID:"),
      "expected derived ID value on stderr",
    );
    const output = parseOutput(result);
    assert.ok(output.ok);
    assert.equal(output.data.extensionId.length, 32, "extension ID should be 32 chars");
  } finally {
    fs.rmSync(homeDir, { recursive: true, force: true });
  }
});

test("setup second profile does not nest under first profile dataDir", async () => {
  const homeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-setup-nest-"));
  try {
    const envOverrides = {
      HOME: homeDir,
      XDG_STATE_HOME: path.join(homeDir, ".local", "state"),
      XDG_CONFIG_HOME: path.join(homeDir, ".config"),
    };
    const baseStateDir = path.join(homeDir, ".local", "state", "tabctl");

    // First setup creates "edge" as default
    await runSetupCli([
      "setup", "--browser", "edge",
      "--extension-id", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "--node", process.execPath,
    ], envOverrides);

    // Second setup creates "chrome"
    const result = await runSetupCli([
      "setup", "--browser", "chrome",
      "--extension-id", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
      "--node", process.execPath,
    ], envOverrides);

    assert.equal(result.status, 0);
    const output = parseOutput(result);
    const chromeDataDir = output.data.dataDir as string;

    // Must be baseStateDir/profiles/chrome, NOT nested under edge's dataDir
    assert.equal(chromeDataDir, path.join(baseStateDir, "profiles", "chrome"));
    assert.ok(fs.existsSync(chromeDataDir));
  } finally {
    fs.rmSync(homeDir, { recursive: true, force: true });
  }
});

test("setup JSON output reflects newly-created profile in footer", async () => {
  const homeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-setup-profile-ctx-"));
  try {
    const extensionId = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const result = await runSetupCli([
      "setup", "--browser", "edge",
      "--extension-id", extensionId,
      "--node", process.execPath,
    ], {
      HOME: homeDir,
      XDG_STATE_HOME: path.join(homeDir, ".local", "state"),
      XDG_CONFIG_HOME: path.join(homeDir, ".config"),
    });

    assert.equal(result.status, 0);
    const output = parseOutput(result);
    assert.equal(output.ok, true);
    // The profile/browser footer fields should reflect the newly-created profile
    assert.equal(output.profile, "edge");
    assert.equal(output.browser, "edge");
  } finally {
    fs.rmSync(homeDir, { recursive: true, force: true });
  }
});

test("setup prints verify-connection hint on stderr", async () => {
  const homeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-setup-hints-"));
  try {
    const extensionId = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const result = await runSetupCli([
      "setup", "--browser", "edge",
      "--extension-id", extensionId,
      "--node", process.execPath,
    ], {
      HOME: homeDir,
      XDG_STATE_HOME: path.join(homeDir, ".local", "state"),
      XDG_CONFIG_HOME: path.join(homeDir, ".config"),
    });

    assert.equal(result.status, 0);
    // Should print verify-connection hint
    assert.ok(result.stderr.includes("Verify connection: tabctl --profile edge ping"), "expected verify hint on stderr");
    assert.ok(result.stderr.includes("extension is active"), "expected extension hint on stderr");
  } finally {
    fs.rmSync(homeDir, { recursive: true, force: true });
  }
});

test("setup non-default profile prints usage hints on stderr", async () => {
  const homeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-setup-nondefault-"));
  try {
    const envOverrides = {
      HOME: homeDir,
      XDG_STATE_HOME: path.join(homeDir, ".local", "state"),
      XDG_CONFIG_HOME: path.join(homeDir, ".config"),
    };

    // First setup creates "edge" as default
    await runSetupCli([
      "setup", "--browser", "edge",
      "--extension-id", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "--node", process.execPath,
    ], envOverrides);

    // Second setup creates "chrome" (non-default)
    const result = await runSetupCli([
      "setup", "--browser", "chrome",
      "--extension-id", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
      "--node", process.execPath,
    ], envOverrides);

    assert.equal(result.status, 0);
    const output = parseOutput(result);
    assert.equal(output.data.isDefault, false);

    // Should show non-default profile hints
    assert.ok(result.stderr.includes('Profile "chrome" created (current default: "edge")'), "expected non-default profile message");
    assert.ok(result.stderr.includes("tabctl --profile chrome"), "expected usage hint");
    assert.ok(result.stderr.includes("tabctl profile-switch chrome"), "expected switch hint");
    // Should also show verify-connection hint
    assert.ok(result.stderr.includes("Verify connection: tabctl --profile chrome ping"), "expected verify hint");
  } finally {
    fs.rmSync(homeDir, { recursive: true, force: true });
  }
});

test("setup fails with manual guidance when Windows verification fails", { skip: process.platform !== "win32" && "windows only" }, async () => {
  const homeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-setup-verify-fail-"));
  try {
    const extensionId = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const missingSocket = `\\\\.\\pipe\\tabctl-missing-${Date.now()}`;
    const result = await runCli([
      "setup", "--browser", "chrome",
      "--extension-id", extensionId,
      "--node", process.execPath,
    ], undefined, {
      HOME: homeDir,
      XDG_STATE_HOME: path.join(homeDir, ".local", "state"),
      XDG_CONFIG_HOME: path.join(homeDir, ".config"),
      TABCTL_SOCKET: missingSocket,
    });

    assert.notEqual(result.status, 0, "expected non-zero setup status on failed verification");
    const output = parseOutput(result);
    assert.equal(output.ok, false);
    assert.equal(output.action, "setup");
    assert.equal(output.error?.message, "Windows setup verification failed");
    assert.equal(output.data?.verification?.attempted, true);
    assert.equal(output.data?.verification?.ok, false);
    assert.ok(Array.isArray(output.data?.verification?.manualSteps));
    assert.ok(output.data.verification.manualSteps.length > 0, "expected manual remediation steps");
    assert.ok(result.stderr.includes("Manual installation steps"), "expected manual guidance in stderr");
    assert.ok(result.stderr.includes("Verify connection: tabctl --profile"), "expected verify command hint");
  } finally {
    fs.rmSync(homeDir, { recursive: true, force: true });
  }
});

test("setup warns when Windows runtime extension ID mismatches expected ID", { skip: process.platform !== "win32" && "windows only" }, async () => {
  const homeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-setup-verify-id-mismatch-"));
  const expectedExtensionId = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
  const runtimeExtensionId = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
  const mock = await startMockSocket((request) => ({
    ok: true,
    action: request.action,
    requestId: request.id,
    data: { now: Date.now(), runtimeId: runtimeExtensionId },
  }));
  try {
    const result = await runCli([
      "setup", "--browser", "chrome",
      "--extension-id", expectedExtensionId,
      "--node", process.execPath,
    ], undefined, {
      HOME: homeDir,
      XDG_STATE_HOME: path.join(homeDir, ".local", "state"),
      XDG_CONFIG_HOME: path.join(homeDir, ".config"),
      TABCTL_SOCKET: mock.socketPath,
    });

    assert.equal(result.status, 0, "expected successful setup status on runtime extension ID mismatch");
    const output = parseOutput(result);
    assert.equal(output.ok, true);
    assert.equal(output.data?.verification?.reason, "extension-id-mismatch");
    assert.equal(output.data?.verification?.expectedExtensionId, expectedExtensionId);
    assert.equal(output.data?.verification?.runtimeExtensionId, runtimeExtensionId);
    assert.ok(result.stderr.includes("runtime extension ID mismatch"), "expected mismatch warning");
    assert.ok(result.stderr.includes("Expected extension ID"), "expected expected-id diagnostics");
    assert.ok(result.stderr.includes("Runtime extension ID"), "expected runtime-id diagnostics");
  } finally {
    await stopMockSocket(mock.server, mock.socketPath, mock.sockets);
    fs.rmSync(homeDir, { recursive: true, force: true });
  }
});
