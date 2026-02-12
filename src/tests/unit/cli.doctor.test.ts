import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { runCli, parseOutput } from "./cli-helpers";
import { resolveWrapperPath, resolveWrapperTextPath } from "../../shared/wrapper-health";

function makeTmpHome(): { homeDir: string; stateHome: string; configHome: string; env: Record<string, string> } {
  const homeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-doctor-"));
  const stateHome = path.join(homeDir, ".local", "state");
  const configHome = path.join(homeDir, ".config");
  return {
    homeDir,
    stateHome,
    configHome,
    env: { HOME: homeDir, XDG_STATE_HOME: stateHome, XDG_CONFIG_HOME: configHome },
  };
}

async function setupProfile(env: Record<string, string>, browser: string, extensionId?: string): Promise<Record<string, unknown>> {
  const result = await runCli([
    "setup",
    "--browser", browser,
    "--extension-id", extensionId || "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "--node", process.execPath,
    "--json",
  ], undefined, env);
  assert.equal(result.status, 0, `setup failed: ${result.stderr}`);
  return parseOutput(result);
}

test("doctor reports healthy profile", async () => {
  const { env } = makeTmpHome();
  await setupProfile(env, "edge");

  const result = await runCli(["doctor", "--json"], undefined, env);
  assert.equal(result.status, 0, `doctor failed: ${result.stderr}`);

  const output = parseOutput(result);
  assert.equal(output.ok, true);
  assert.equal(output.action, "doctor");
  assert.ok(output.data.profiles.edge);
  assert.equal(output.data.profiles.edge.ok, true);
  assert.deepEqual(output.data.profiles.edge.issues, []);
  assert.equal(output.data.summary.total, 1);
  assert.equal(output.data.summary.healthy, 1);
  assert.equal(output.data.summary.broken, 0);
});

test("doctor detects broken Node path", async () => {
  const { env, stateHome } = makeTmpHome();
  await setupProfile(env, "chrome");

  // Corrupt the wrapper by replacing Node path with a bad one
  const profileDir = path.join(stateHome, "tabctl", "profiles", "chrome");
  const wrapperPath = resolveWrapperPath(profileDir);
  const textPath = resolveWrapperTextPath(wrapperPath);
  let content = fs.readFileSync(textPath, "utf-8");
  content = content.replace(process.execPath, "/nonexistent/node-v99/bin/node");
  fs.writeFileSync(textPath, content, "utf8");

  const result = await runCli(["doctor", "--json"], undefined, env);
  // doctor exits 1 on broken profiles (errorOut is not called; printJson + exit code depends on ok)
  const output = parseOutput(result);
  assert.equal(output.ok, false);
  assert.equal(output.data.profiles.chrome.ok, false);
  assert.ok(output.data.profiles.chrome.issues.some((i: string) => i.includes("Node path not found")));
  assert.equal(output.data.summary.broken, 1);
});

test("doctor --fix repairs broken Node path", async () => {
  const { env, stateHome } = makeTmpHome();
  await setupProfile(env, "edge");

  // Break the wrapper
  const profileDir = path.join(stateHome, "tabctl", "profiles", "edge");
  const wrapperPath = resolveWrapperPath(profileDir);
  const textPath = resolveWrapperTextPath(wrapperPath);
  let content = fs.readFileSync(textPath, "utf-8");
  content = content.replace(process.execPath, "/nonexistent/node-v99/bin/node");
  fs.writeFileSync(textPath, content, "utf8");

  // Verify it's broken
  const broken = await runCli(["doctor", "--json"], undefined, env);
  const brokenOutput = parseOutput(broken);
  assert.equal(brokenOutput.ok, false);

  // Fix it
  const fixed = await runCli(["doctor", "--fix", "--json"], undefined, env);
  const fixedOutput = parseOutput(fixed);
  assert.equal(fixedOutput.ok, true);
  assert.equal(fixedOutput.data.profiles.edge.fixed, true);
  assert.equal(fixedOutput.data.summary.fixed, 1);

  // Verify wrapper now has current Node path
  const newContent = fs.readFileSync(textPath, "utf-8");
  assert.ok(newContent.includes(process.execPath), "wrapper should contain current Node path");
});

test("doctor --fix repairs broken host path", async () => {
  const { env, stateHome } = makeTmpHome();
  await setupProfile(env, "edge");

  // Break the wrapper by replacing host path
  const profileDir = path.join(stateHome, "tabctl", "profiles", "edge");
  const wrapperPath = resolveWrapperPath(profileDir);
  const textPath = resolveWrapperTextPath(wrapperPath);
  let content = fs.readFileSync(textPath, "utf-8");
  const hostBundlePath = path.join(stateHome, "tabctl", "host.bundle.js");
  content = content.replace(hostBundlePath, "/nonexistent/host.bundle.js");
  fs.writeFileSync(textPath, content, "utf8");

  // Fix it
  const fixed = await runCli(["doctor", "--fix", "--json"], undefined, env);
  const fixedOutput = parseOutput(fixed);
  assert.equal(fixedOutput.ok, true);
  assert.equal(fixedOutput.data.profiles.edge.fixed, true);

  // Verify wrapper now has correct host path
  const newContent = fs.readFileSync(textPath, "utf-8");
  assert.ok(newContent.includes(hostBundlePath), "wrapper should contain stable host path");
});

test("doctor with multiple profiles reports each", async () => {
  const { env, stateHome } = makeTmpHome();
  await setupProfile(env, "edge");
  await setupProfile(env, "chrome");

  // Break chrome only
  const profileDir = path.join(stateHome, "tabctl", "profiles", "chrome");
  const wrapperPath = resolveWrapperPath(profileDir);
  const textPath = resolveWrapperTextPath(wrapperPath);
  let content = fs.readFileSync(textPath, "utf-8");
  content = content.replace(process.execPath, "/nonexistent/node");
  fs.writeFileSync(textPath, content, "utf8");

  const result = await runCli(["doctor", "--json"], undefined, env);
  const output = parseOutput(result);
  assert.equal(output.ok, false);
  assert.equal(output.data.profiles.edge.ok, true);
  assert.equal(output.data.profiles.chrome.ok, false);
  assert.equal(output.data.summary.total, 2);
  assert.equal(output.data.summary.healthy, 1);
  assert.equal(output.data.summary.broken, 1);
});

test("doctor exits with error when no profiles configured", async () => {
  const { env } = makeTmpHome();
  const result = await runCli(["doctor", "--json"], undefined, env);
  assert.notEqual(result.status, 0);
  const output = parseOutput(result);
  assert.equal(output.ok, false);
});
