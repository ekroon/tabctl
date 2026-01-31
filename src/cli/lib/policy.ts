import fs from "fs";
import os from "os";
import path from "path";

export type Policy = {
  protect?: {
    pinned?: boolean;
    groupTitles?: string[];
    groupIds?: number[];
    windowIds?: number[];
    domains?: string[];
  };
};

export type PolicyContext = {
  policy: Policy | null;
  path?: string;
};

function configHome(): string {
  return process.env.XDG_CONFIG_HOME || path.join(os.homedir(), ".config");
}

export function defaultPolicyPath(): string {
  return path.join(configHome(), "tabctl", "policy.json");
}

export function defaultPolicyTemplate(): Policy {
  return {
    protect: {
      pinned: true,
      groupTitles: ["🔒"],
    },
  };
}

export function loadPolicy(): PolicyContext {
  const resolvedPath = defaultPolicyPath();

  if (!fs.existsSync(resolvedPath)) {
    return { policy: null, path: resolvedPath };
  }

  const raw = fs.readFileSync(resolvedPath, "utf8");
  const parsed = JSON.parse(raw) as Policy;
  return { policy: parsed, path: resolvedPath };
}

export function getDomain(rawUrl?: string | null): string | null {
  if (!rawUrl) {
    return null;
  }
  try {
    const url = new URL(rawUrl);
    return url.hostname;
  } catch {
    return null;
  }
}

export function evaluateTab(tab: Record<string, unknown>, policy: Policy | null) {
  const reasons: string[] = [];
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

export function annotateEntry<T extends Record<string, unknown>>(entry: T, policy: Policy | null) {
  const { eligible, protectedReasons } = evaluateTab(entry, policy);
  return {
    ...entry,
    eligible,
    protectedReasons,
  } as T & { eligible: boolean; protectedReasons: string[] };
}

export function summarizePolicy(policy: Policy | null, policyPath?: string) {
  if (!policy) {
    return { enabled: false, path: policyPath };
  }
  return {
    enabled: true,
    path: policyPath,
    protect: policy.protect || {},
  };
}
