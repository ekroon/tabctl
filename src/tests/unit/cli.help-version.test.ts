import assert from "node:assert/strict";
import fs from "fs";
import os from "os";
import path from "path";
import test from "node:test";
import { runCli, assertVersion, pkgVersion } from "./cli-helpers";

test("policy init creates default file", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-policy-init-"));
  const result = await runCli(["policy", "--init"], undefined, { XDG_CONFIG_HOME: dir });

  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
  const policyPath = path.join(dir, "tabctl", "policy.json");
  assert.ok(fs.existsSync(policyPath));
  const raw = fs.readFileSync(policyPath, "utf8");
  assert.match(raw, /"pinned"/);
});

test("help outputs plain text by default", async () => {
  const result = await runCli(["help"]);
  assert.equal(result.status, 0);
  assert.match(result.stdout, /tabctl - Edge tab management CLI/);
});

test("help supports --help flag", async () => {
  const result = await runCli(["--help"]);
  assert.equal(result.status, 0);
  assert.match(result.stdout, /tabctl - Edge tab management CLI/);
  assert.match(result.stdout, /Command Details/);
  assert.match(result.stdout, /Option Groups/);
});

test("help supports json output", async () => {
  const result = await runCli(["help", "--json"]);
  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
  assert.ok(output.data?.commands);
  assertVersion(output.data?.version as string | undefined);
  const optionGroups = output.data?.optionGroups as Array<{ name: string; options: string[] }> | undefined;
  assert.ok(optionGroups);
  const scopeGroup = optionGroups?.find((group) => group?.name === "Scope Options");
  const paginationGroup = optionGroups?.find((group) => group?.name === "Pagination Options");
  assert.ok(scopeGroup?.options?.includes("--tab <id> (repeatable)"));
  assert.ok(scopeGroup?.options?.includes("--group <name>"));
  assert.ok(scopeGroup?.options?.includes("--group-id <id>"));
  assert.ok(scopeGroup?.options?.includes("--ungrouped"));
  assert.ok(scopeGroup?.options?.includes("--window <id|active|last-focused>"));
  assert.ok(scopeGroup?.options?.includes("--all"));
  assert.ok(paginationGroup?.options?.includes("--limit <n>"));
  assert.ok(paginationGroup?.options?.includes("--offset <n>"));
  assert.ok(paginationGroup?.options?.includes("--no-page"));
  const analyze = output.data?.commands?.find((cmd: { name: string; options?: string[] }) => cmd?.name === "analyze");
  assert.ok(analyze?.options?.includes("--window-title"));
  const list = output.data?.commands?.find((cmd: { name: string; options?: string[] }) => cmd?.name === "list");
  assert.ok(list?.options?.includes("--groups"));
  const open = output.data?.commands?.find((cmd: { name: string; options?: string[] }) => cmd?.name === "open");
  assert.ok(open?.options?.includes("--url <url> (repeatable)"));
  assert.ok(open?.options?.includes("--before-tab <id>"));
  assert.ok(open?.options?.includes("--after-tab <id>"));
  assert.ok(open?.options?.includes("--new-window"));
  const report = output.data?.commands?.find((cmd: { name: string; options?: string[] }) => cmd?.name === "report");
  assert.ok(report?.options?.includes("--format json|md|csv"));
  const close = output.data?.commands?.find((cmd: { name: string; options?: string[] }) => cmd?.name === "close");
  assert.ok(close?.options?.includes("--dry-run"));
  const setup = output.data?.commands?.find((cmd: { name: string; options?: string[] }) => cmd?.name === "setup");
  assert.ok(setup?.options?.includes("--browser edge|chrome"));
});

test("command-specific help filters output", async () => {
  const result = await runCli(["help", "open", "--json"]);
  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
  assert.equal(output.data?.commands?.length, 1);
  assert.equal(output.data?.commands?.[0]?.name, "open");
});

test("version outputs cli version", async () => {
  const result = await runCli(["version"]);
  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
  assertVersion(output.data?.version as string | undefined);
  assert.equal(output.data?.component, "cli");
  assert.equal(output.data?.baseVersion, pkgVersion);
});

test("skill install creates project skill link", async () => {
  const testRoot = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-skill-"));
  const originalCwd = process.cwd();
  const installRoot = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-install-"));
  const repoRoot = path.resolve(__dirname, "../../..");
  const fakeBin = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-fakebin-"));
  const npxFixture = path.join(__dirname, "fixtures", "npx");
  const fakeNpx = path.join(fakeBin, "npx");
  fs.copyFileSync(npxFixture, fakeNpx);
  fs.chmodSync(fakeNpx, 0o755);
  const npxCapture = path.join(testRoot, "npx-args.json");
  fs.cpSync(path.join(repoRoot, "dist", "cli"), path.join(installRoot, "cli"), { recursive: true });
  fs.cpSync(path.join(repoRoot, "dist", "shared"), path.join(installRoot, "shared"), { recursive: true });
  const cliTarget = path.join(installRoot, "cli", "tabctl.js");
  process.chdir(testRoot);
  try {
    const result = await runCli([
      "skill",
    ], undefined, {
      XDG_CONFIG_HOME: path.join(testRoot, ".config"),
      NPX_CAPTURE_PATH: npxCapture,
    }, cliTarget, fakeNpx);
    assert.equal(result.status, 0);
    const output = JSON.parse(result.stdout.trim());
    assert.equal(output.ok, true);
    const targetDir = output.data?.targetDir as string;
    assert.ok(targetDir);
    assert.equal(output.data?.scope, "project");
    assert.ok(targetDir.includes(path.join(testRoot, ".opencode", "skills", "tabctl")));
    const captured = JSON.parse(fs.readFileSync(npxCapture, "utf8"));
    assert.deepEqual(captured.args, [
      "skills",
      "add",
      "https://github.com/ekroon/tabctl",
      "--skill",
      "tabctl",
    ]);
  } finally {
    process.chdir(originalCwd);
  }
});

test("skill install supports global scope", async () => {
  const testRoot = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-skill-global-"));
  const configHome = path.join(testRoot, "config");
  const originalCwd = process.cwd();
  const installRoot = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-install-"));
  const repoRoot = path.resolve(__dirname, "../../..");
  const fakeBin = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-fakebin-"));
  const npxFixture = path.join(__dirname, "fixtures", "npx");
  const fakeNpx = path.join(fakeBin, "npx");
  fs.copyFileSync(npxFixture, fakeNpx);
  fs.chmodSync(fakeNpx, 0o755);
  const npxCapture = path.join(testRoot, "npx-args.json");
  fs.cpSync(path.join(repoRoot, "dist", "cli"), path.join(installRoot, "cli"), { recursive: true });
  fs.cpSync(path.join(repoRoot, "dist", "shared"), path.join(installRoot, "shared"), { recursive: true });
  const cliTarget = path.join(installRoot, "cli", "tabctl.js");
  process.chdir(testRoot);

  try {
    const result = await runCli(["skill", "--global"], undefined, {
      XDG_CONFIG_HOME: configHome,
      NPX_CAPTURE_PATH: npxCapture,
    }, cliTarget, fakeNpx);
    assert.equal(result.status, 0);
    const output = JSON.parse(result.stdout.trim());
    const targetDir = output.data?.targetDir as string;
    assert.ok(targetDir);
    assert.equal(output.data?.scope, "global");
    assert.ok(targetDir.includes(path.join(configHome, "opencode", "skills", "tabctl")));
    const captured = JSON.parse(fs.readFileSync(npxCapture, "utf8"));
    assert.deepEqual(captured.args, [
      "skills",
      "add",
      "https://github.com/ekroon/tabctl",
      "--skill",
      "tabctl",
      "-g",
    ]);
  } finally {
    process.chdir(originalCwd);
  }
});


test("version includes dev sha when built", async () => {
  const result = await runCli(["version"]);
  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
  const version = output.data?.version as string | undefined;
  assert.ok(version);
  if (version && version.includes("-dev.")) {
    const re = new RegExp(`^${pkgVersion.replace(/\./g, "\\.")}-dev\\.[0-9a-f]{8}(\\.dirty)?$`);
    assert.match(version, re);
  } else {
    assert.equal(version, pkgVersion);
  }
  assert.equal(output.data?.baseVersion, pkgVersion);
});
