import assert from "node:assert/strict";
import fs from "fs";
import os from "os";
import path from "path";
import test from "node:test";
import { evaluateTab, loadPolicy } from "../../cli/lib/policy";
import { resetConfig } from "../../shared/config";

test("loadPolicy returns null when file missing", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-policy-"));
  const previous = process.env.XDG_CONFIG_HOME;
  process.env.XDG_CONFIG_HOME = dir;
  resetConfig();
  const policyPath = path.join(dir, "tabctl", "policy.json");
  const context = loadPolicy();
  assert.equal(context.policy, null);
  assert.equal(context.path, policyPath);
  if (previous) {
    process.env.XDG_CONFIG_HOME = previous;
  } else {
    delete process.env.XDG_CONFIG_HOME;
  }
  resetConfig();
});

test("loadPolicy parses policy file", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-policy-"));
  const previous = process.env.XDG_CONFIG_HOME;
  process.env.XDG_CONFIG_HOME = dir;
  resetConfig();
  const policyDir = path.join(dir, "tabctl");
  fs.mkdirSync(policyDir, { recursive: true });
  const policyPath = path.join(policyDir, "policy.json");
  fs.writeFileSync(policyPath, JSON.stringify({ protect: { pinned: true, groupTitles: ["🔒"] } }), "utf8");

  const context = loadPolicy();
  assert.ok(context.policy);
  assert.equal(context.policy?.protect?.pinned, true);
  assert.deepEqual(context.policy?.protect?.groupTitles, ["🔒"]);
  if (previous) {
    process.env.XDG_CONFIG_HOME = previous;
  } else {
    delete process.env.XDG_CONFIG_HOME;
  }
  resetConfig();
});

test("evaluateTab protects pinned and group titles", () => {
  const policy = { protect: { pinned: true, groupTitles: ["🔒"] } };
  const pinnedTab = { pinned: true, groupTitle: "Work" };
  const groupTab = { pinned: false, groupTitle: "🔒" };
  const normalTab = { pinned: false, groupTitle: "Other" };

  assert.equal(evaluateTab(pinnedTab, policy).eligible, false);
  assert.equal(evaluateTab(groupTab, policy).eligible, false);
  assert.equal(evaluateTab(normalTab, policy).eligible, true);
});
