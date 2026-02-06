import assert from "node:assert/strict";
import fs from "fs";
import os from "os";
import path from "path";
import test from "node:test";
import { configPath, loadConfig, resolveBrowser, resolveSocketPath } from "../../shared/config";

function withConfigHome(dir: string, fn: () => void) {
  const previous = process.env.XDG_CONFIG_HOME;
  process.env.XDG_CONFIG_HOME = dir;
  try {
    fn();
  } finally {
    if (previous) {
      process.env.XDG_CONFIG_HOME = previous;
    } else {
      delete process.env.XDG_CONFIG_HOME;
    }
  }
}

test("config defaults to chrome when missing", () => {
  const configHomeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-config-"));
  withConfigHome(configHomeDir, () => {
    const config = loadConfig();
    assert.equal(config, null);
    assert.equal(resolveBrowser(config), "chrome");
    const stateHome = path.join(configHomeDir, "state");
    assert.equal(
      resolveSocketPath(stateHome, resolveBrowser(config), config),
      path.join(stateHome, "tabctl", "tabctl-chrome.sock"),
    );
  });
});

test("config uses chrome socket", () => {
  const configHomeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-config-"));
  withConfigHome(configHomeDir, () => {
    const resolvedPath = configPath();
    fs.mkdirSync(path.dirname(resolvedPath), { recursive: true });
    fs.writeFileSync(resolvedPath, JSON.stringify({ browser: "chrome" }));
    const config = loadConfig();
    assert.equal(resolveBrowser(config), "chrome");
    const stateHome = path.join(configHomeDir, "state");
    assert.equal(
      resolveSocketPath(stateHome, resolveBrowser(config), config),
      path.join(stateHome, "tabctl", "tabctl-chrome.sock"),
    );
  });
});

test("config uses edge socket", () => {
  const configHomeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-config-"));
  withConfigHome(configHomeDir, () => {
    const resolvedPath = configPath();
    fs.mkdirSync(path.dirname(resolvedPath), { recursive: true });
    fs.writeFileSync(resolvedPath, JSON.stringify({ browser: "edge" }));
    const config = loadConfig();
    assert.equal(resolveBrowser(config), "edge");
    const stateHome = path.join(configHomeDir, "state");
    assert.equal(
      resolveSocketPath(stateHome, resolveBrowser(config), config),
      path.join(stateHome, "tabctl", "tabctl-edge.sock"),
    );
  });
});

test("config uses custom socket name", () => {
  const configHomeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-config-"));
  withConfigHome(configHomeDir, () => {
    const resolvedPath = configPath();
    fs.mkdirSync(path.dirname(resolvedPath), { recursive: true });
    fs.writeFileSync(resolvedPath, JSON.stringify({ socketName: "tabctl-test" }));
    const config = loadConfig();
    const stateHome = path.join(configHomeDir, "state");
    assert.equal(
      resolveSocketPath(stateHome, resolveBrowser(config), config),
      path.join(stateHome, "tabctl", "tabctl-test.sock"),
    );
  });
});
