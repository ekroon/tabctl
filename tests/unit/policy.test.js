"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
const strict_1 = __importDefault(require("node:assert/strict"));
const fs_1 = __importDefault(require("fs"));
const os_1 = __importDefault(require("os"));
const path_1 = __importDefault(require("path"));
const node_test_1 = __importDefault(require("node:test"));
const policy_1 = require("../../cli/lib/policy");
(0, node_test_1.default)("loadPolicy returns null when file missing", () => {
    const dir = fs_1.default.mkdtempSync(path_1.default.join(os_1.default.tmpdir(), "tabarchive-policy-"));
    const previous = process.env.XDG_CONFIG_HOME;
    process.env.XDG_CONFIG_HOME = dir;
    const policyPath = path_1.default.join(dir, "tabctl", "policy.json");
    const context = (0, policy_1.loadPolicy)();
    strict_1.default.equal(context.policy, null);
    strict_1.default.equal(context.path, policyPath);
    if (previous) {
        process.env.XDG_CONFIG_HOME = previous;
    }
    else {
        delete process.env.XDG_CONFIG_HOME;
    }
});
(0, node_test_1.default)("loadPolicy parses policy file", () => {
    const dir = fs_1.default.mkdtempSync(path_1.default.join(os_1.default.tmpdir(), "tabarchive-policy-"));
    const previous = process.env.XDG_CONFIG_HOME;
    process.env.XDG_CONFIG_HOME = dir;
    const policyDir = path_1.default.join(dir, "tabctl");
    fs_1.default.mkdirSync(policyDir, { recursive: true });
    const policyPath = path_1.default.join(policyDir, "policy.json");
    fs_1.default.writeFileSync(policyPath, JSON.stringify({ protect: { pinned: true, groupTitles: ["🔒"] } }), "utf8");
    const context = (0, policy_1.loadPolicy)();
    strict_1.default.ok(context.policy);
    strict_1.default.equal(context.policy?.protect?.pinned, true);
    strict_1.default.deepEqual(context.policy?.protect?.groupTitles, ["🔒"]);
    if (previous) {
        process.env.XDG_CONFIG_HOME = previous;
    }
    else {
        delete process.env.XDG_CONFIG_HOME;
    }
});
(0, node_test_1.default)("evaluateTab protects pinned and group titles", () => {
    const policy = { protect: { pinned: true, groupTitles: ["🔒"] } };
    const pinnedTab = { pinned: true, groupTitle: "Work" };
    const groupTab = { pinned: false, groupTitle: "🔒" };
    const normalTab = { pinned: false, groupTitle: "Other" };
    strict_1.default.equal((0, policy_1.evaluateTab)(pinnedTab, policy).eligible, false);
    strict_1.default.equal((0, policy_1.evaluateTab)(groupTab, policy).eligible, false);
    strict_1.default.equal((0, policy_1.evaluateTab)(normalTab, policy).eligible, true);
});
