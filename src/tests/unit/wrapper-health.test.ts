import test from "node:test";
import assert from "node:assert/strict";
import os from "node:os";
import path from "node:path";
import fs from "node:fs";
import { parseWrapper, checkWrapper, resolveWrapperPath } from "../../shared/wrapper-health";

function makeTmpDir(): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-wrapper-"));
}

function writeShWrapper(dir: string, nodePath: string, hostPath: string, profileName: string | null): string {
  const wrapperPath = path.join(dir, "tabctl-host.sh");
  const lines = [
    "#!/usr/bin/env bash",
    "set -euo pipefail",
  ];
  if (profileName) {
    lines.push(`export TABCTL_PROFILE="${profileName}"`);
  }
  lines.push(`exec "${nodePath}" "${hostPath}"`);
  lines.push("");
  fs.writeFileSync(wrapperPath, lines.join("\n"), { mode: 0o700 });
  return wrapperPath;
}

function writeCmdWrapper(dir: string, nodePath: string, hostPath: string, profileName: string | null): string {
  const wrapperPath = path.join(dir, "tabctl-host.cmd");
  const lines = ["@echo off"];
  if (profileName) {
    lines.push(`set TABCTL_PROFILE=${profileName}`);
  }
  lines.push(`"${nodePath}" "${hostPath}" %*`);
  lines.push("");
  fs.writeFileSync(wrapperPath, lines.join("\r\n"), "utf8");
  return wrapperPath;
}

function writeCfgWrapper(dir: string, nodePath: string, hostPath: string, profileName: string | null): string {
  const cfgPath = path.join(dir, "host-launcher.cfg");
  const lines = [nodePath, hostPath];
  if (profileName) {
    lines.push(`TABCTL_PROFILE=${profileName}`);
  }
  lines.push("");
  fs.writeFileSync(cfgPath, lines.join("\r\n"), "utf8");
  return cfgPath;
}

// --- parseWrapper: .sh format ---

test("parseWrapper parses Unix wrapper with profile", () => {
  const dir = makeTmpDir();
  try {
    const wrapperPath = writeShWrapper(dir, "/usr/bin/node", "/opt/tabctl/host.js", "edge");
    const info = parseWrapper(wrapperPath);
    assert.ok(info);
    assert.equal(info.nodePath, "/usr/bin/node");
    assert.equal(info.hostPath, "/opt/tabctl/host.js");
    assert.equal(info.profileName, "edge");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("parseWrapper parses Unix wrapper without profile", () => {
  const dir = makeTmpDir();
  try {
    const wrapperPath = writeShWrapper(dir, "/usr/bin/node", "/opt/tabctl/host.js", null);
    const info = parseWrapper(wrapperPath);
    assert.ok(info);
    assert.equal(info.nodePath, "/usr/bin/node");
    assert.equal(info.hostPath, "/opt/tabctl/host.js");
    assert.equal(info.profileName, null);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

// --- parseWrapper: .cmd format ---

test("parseWrapper parses Windows .cmd wrapper with profile", () => {
  const dir = makeTmpDir();
  try {
    const wrapperPath = writeCmdWrapper(dir, "C:\\node\\node.exe", "C:\\tabctl\\host.js", "chrome");
    const info = parseWrapper(wrapperPath);
    assert.ok(info);
    assert.equal(info.nodePath, "C:\\node\\node.exe");
    assert.equal(info.hostPath, "C:\\tabctl\\host.js");
    assert.equal(info.profileName, "chrome");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

// --- parseWrapper: .cfg format ---

test("parseWrapper parses .cfg launcher config", () => {
  const dir = makeTmpDir();
  try {
    const cfgPath = writeCfgWrapper(dir, "C:\\node\\node.exe", "C:\\tabctl\\host.js", "edge");
    const info = parseWrapper(cfgPath);
    assert.ok(info);
    assert.equal(info.nodePath, "C:\\node\\node.exe");
    assert.equal(info.hostPath, "C:\\tabctl\\host.js");
    assert.equal(info.profileName, "edge");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

// --- parseWrapper: .exe format (looks for adjacent .cfg) ---

test("parseWrapper reads .exe by looking for host-launcher.cfg", () => {
  const dir = makeTmpDir();
  try {
    writeCfgWrapper(dir, "/usr/bin/node", "/opt/host.js", "chrome");
    // Create a fake .exe file
    const exePath = path.join(dir, "tabctl-host.exe");
    fs.writeFileSync(exePath, "FAKE EXE", "utf8");

    const info = parseWrapper(exePath);
    assert.ok(info);
    assert.equal(info.nodePath, "/usr/bin/node");
    assert.equal(info.hostPath, "/opt/host.js");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

// --- parseWrapper: error cases ---

test("parseWrapper returns null for nonexistent file", () => {
  assert.equal(parseWrapper("/nonexistent/wrapper.sh"), null);
});

test("parseWrapper returns null for unparseable content", () => {
  const dir = makeTmpDir();
  try {
    const wrapperPath = path.join(dir, "tabctl-host.sh");
    fs.writeFileSync(wrapperPath, "#!/usr/bin/env bash\necho hello\n", "utf8");
    assert.equal(parseWrapper(wrapperPath), null);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

// --- checkWrapper ---

test("checkWrapper reports ok for valid paths", () => {
  const dir = makeTmpDir();
  try {
    // Create real files so path validation passes
    const nodePath = path.join(dir, "node");
    const hostPath = path.join(dir, "host.js");
    fs.writeFileSync(nodePath, "#!/usr/bin/env node", { mode: 0o755 });
    fs.writeFileSync(hostPath, "// host", "utf8");

    const wrapperPath = writeShWrapper(dir, nodePath, hostPath, "edge");
    const result = checkWrapper(wrapperPath);
    assert.equal(result.ok, true);
    assert.deepEqual(result.issues, []);
    assert.ok(result.info);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("checkWrapper reports broken Node path", () => {
  const dir = makeTmpDir();
  try {
    const hostPath = path.join(dir, "host.js");
    fs.writeFileSync(hostPath, "// host", "utf8");

    const wrapperPath = writeShWrapper(dir, "/nonexistent/node", hostPath, "edge");
    const result = checkWrapper(wrapperPath);
    assert.equal(result.ok, false);
    assert.equal(result.issues.length, 1);
    assert.ok(result.issues[0].includes("Node path not found"));
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("checkWrapper reports broken host path", () => {
  const dir = makeTmpDir();
  try {
    const nodePath = path.join(dir, "node");
    fs.writeFileSync(nodePath, "#!/usr/bin/env node", { mode: 0o755 });

    const wrapperPath = writeShWrapper(dir, nodePath, "/nonexistent/host.js", "edge");
    const result = checkWrapper(wrapperPath);
    assert.equal(result.ok, false);
    assert.equal(result.issues.length, 1);
    assert.ok(result.issues[0].includes("Host path not found"));
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("checkWrapper reports both broken paths", () => {
  const dir = makeTmpDir();
  try {
    const wrapperPath = writeShWrapper(dir, "/nonexistent/node", "/nonexistent/host.js", "edge");
    const result = checkWrapper(wrapperPath);
    assert.equal(result.ok, false);
    assert.equal(result.issues.length, 2);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("checkWrapper reports missing wrapper file", () => {
  const result = checkWrapper("/nonexistent/tabctl-host.sh");
  assert.equal(result.ok, false);
  assert.ok(result.issues[0].includes("Wrapper not found"));
});

// --- resolveWrapperPath ---

test("resolveWrapperPath returns .sh on non-Windows", () => {
  if (process.platform === "win32") return;
  const result = resolveWrapperPath("/some/profile/dir");
  assert.equal(result, "/some/profile/dir/tabctl-host.sh");
});

// --- parseWrapper with escaped quotes ---

test("parseWrapper handles paths with special characters", () => {
  const dir = makeTmpDir();
  try {
    const wrapperPath = writeShWrapper(
      dir,
      "/Users/john doe/.local/node",
      "/opt/tab ctl/host.js",
      null,
    );
    const info = parseWrapper(wrapperPath);
    assert.ok(info);
    assert.equal(info.nodePath, "/Users/john doe/.local/node");
    assert.equal(info.hostPath, "/opt/tab ctl/host.js");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
