"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.defaultPolicyPath = defaultPolicyPath;
exports.defaultPolicyTemplate = defaultPolicyTemplate;
exports.loadPolicy = loadPolicy;
exports.getDomain = getDomain;
exports.evaluateTab = evaluateTab;
exports.annotateEntry = annotateEntry;
exports.summarizePolicy = summarizePolicy;
const fs_1 = __importDefault(require("fs"));
const os_1 = __importDefault(require("os"));
const path_1 = __importDefault(require("path"));
function configHome() {
    return process.env.XDG_CONFIG_HOME || path_1.default.join(os_1.default.homedir(), ".config");
}
function defaultPolicyPath() {
    return path_1.default.join(configHome(), "tabctl", "policy.json");
}
function defaultPolicyTemplate() {
    return {
        protect: {
            pinned: true,
            groupTitles: ["🔒"],
        },
    };
}
function loadPolicy() {
    const resolvedPath = defaultPolicyPath();
    if (!fs_1.default.existsSync(resolvedPath)) {
        return { policy: null, path: resolvedPath };
    }
    const raw = fs_1.default.readFileSync(resolvedPath, "utf8");
    const parsed = JSON.parse(raw);
    return { policy: parsed, path: resolvedPath };
}
function getDomain(rawUrl) {
    if (!rawUrl) {
        return null;
    }
    try {
        const url = new URL(rawUrl);
        return url.hostname;
    }
    catch {
        return null;
    }
}
function evaluateTab(tab, policy) {
    const reasons = [];
    if (!policy?.protect) {
        return { eligible: true, protectedReasons: reasons };
    }
    const protect = policy.protect;
    if (protect.pinned && tab.pinned === true) {
        reasons.push("pinned");
    }
    const groupTitle = typeof tab.groupTitle === "string" ? tab.groupTitle : null;
    if (groupTitle && Array.isArray(protect.groupTitles) && protect.groupTitles.includes(groupTitle)) {
        reasons.push("groupTitle");
    }
    const groupId = Number(tab.groupId);
    if (Number.isFinite(groupId) && Array.isArray(protect.groupIds) && protect.groupIds.includes(groupId)) {
        reasons.push("groupId");
    }
    const windowId = Number(tab.windowId);
    if (Number.isFinite(windowId) && Array.isArray(protect.windowIds) && protect.windowIds.includes(windowId)) {
        reasons.push("windowId");
    }
    const domain = getDomain(typeof tab.url === "string" ? tab.url : null);
    if (domain && Array.isArray(protect.domains) && protect.domains.includes(domain)) {
        reasons.push("domain");
    }
    return { eligible: reasons.length === 0, protectedReasons: reasons };
}
function annotateEntry(entry, policy) {
    const { eligible, protectedReasons } = evaluateTab(entry, policy);
    return {
        ...entry,
        eligible,
        protectedReasons,
    };
}
function summarizePolicy(policy, policyPath) {
    if (!policy) {
        return { enabled: false, path: policyPath };
    }
    return {
        enabled: true,
        path: policyPath,
        protect: policy.protect || {},
    };
}
