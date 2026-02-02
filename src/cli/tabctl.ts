#!/usr/bin/env node
import fs from "fs";
import net from "net";
import os from "os";
import path from "path";
import { spawnSync } from "node:child_process";
import { renderCsv, renderMarkdown } from "./lib/report";
import { annotateEntry, defaultPolicyPath, defaultPolicyTemplate, evaluateTab, loadPolicy, summarizePolicy, type Policy } from "./lib/policy";
import { VERSION, BASE_VERSION, GIT_SHA, DIRTY } from "../shared/version";

const STATE_HOME = process.env.XDG_STATE_HOME || path.join(os.homedir(), ".local", "state");
const SOCKET_PATH = process.env.TABCTL_SOCKET || path.join(STATE_HOME, "tabctl", "tabctl.sock");
const LEGACY_SOCKET_PATH = path.join(os.homedir(), ".tabarchive", "tabarchive.sock");
const HOST_NAME = "com.erwinkroon.tabctl";
const HOST_DESCRIPTION = "Tab archive native host";
const EXTENSION_ID_PATTERN = /^[a-p]{32}$/;
const GROUP_COLORS = new Set([
  "grey",
  "blue",
  "red",
  "yellow",
  "green",
  "pink",
  "purple",
  "cyan",
  "orange",
]);
const DEFAULT_PAGE_LIMIT = 100;
const SKILL_NAME = "tabctl";
const SKILL_REPO = process.env.TABCTL_SKILL_REPO || "https://github.com/ekroon/tabctl";
const SUPPORTED_SIGNALS = ["page-meta", "github-state", "selector"] as const;
const SUPPORTED_SIGNAL_SET = new Set<string>(SUPPORTED_SIGNALS);

type Options = {
  _: string[];
  [key: string]: unknown;
};

function createId() {
  return `req-${Date.now()}-${Math.random().toString(16).slice(2, 10)}`;
}

function normalizeGroupColor(value: unknown) {
  if (typeof value !== "string") {
    return undefined;
  }
  const trimmed = value.trim().toLowerCase();
  if (!trimmed) {
    return undefined;
  }
  if (!GROUP_COLORS.has(trimmed)) {
    errorOut(`Invalid color: ${value}. Use one of: ${Array.from(GROUP_COLORS).join(", ")}`);
  }
  return trimmed;
}

function normalizeSignals(value: unknown) {
  if (!Array.isArray(value)) {
    return [] as string[];
  }
  return value.map((signal) => String(signal).trim()).filter(Boolean);
}

function validateSignals(signals: string[]) {
  for (const signal of signals) {
    if (!SUPPORTED_SIGNAL_SET.has(signal)) {
      errorOut(`Unknown signal: ${signal}. Use one of: ${SUPPORTED_SIGNALS.join(", ")}`);
    }
  }
}

function parseArgs(argv: string[]) {
  const args = [...argv];
  let command: string | undefined;
  const options: Options = { _: [] };
  const allowedFlags = new Set([
    "all",
    "pretty",
    "confirm",
    "dry-run",
    "github",
    "progress",
    "init",
    "help",
    "json",
    "window-title",
    "create",
    "collapsed",
    "expanded",
    "new-window",
    "close-source",
    "include-stale",
    "groups",
    "global",
    "stale-days",
    "github-concurrency",
    "github-timeout-ms",
    "tab",
    "group",
    "group-id",
    "window",
    "agent",
    "signal",
    "signal-config",
    "signal-concurrency",
    "signal-timeout-ms",
    "selector",
    "txid",
    "latest",
    "url",
    "after-group",
    "before-group",
    "after-tab",
    "before-tab",
    "window-group",
    "window-tab",
    "window-url",
    "title",
    "color",
    "from",
    "to",
    "browser",
    "extension-id",
    "node",
    "apply",
    "format",
    "out",
    "limit",
    "offset",
    "no-page",
    "ungrouped",
  ]);

  while (args.length > 0) {
    const arg = args.shift() as string;
    if (!arg.startsWith("--")) {
      if (!command) {
        command = arg;
        continue;
      }
      options._.push(arg);
      continue;
    }

    const key = arg.slice(2);
    if (!allowedFlags.has(key)) {
      if (key === "format") {
        errorOut("Unknown option: --format");
      }
      errorOut(`Unknown option: --${key}`);
    }
    if (["all", "pretty", "confirm", "dry-run", "github", "progress", "init", "help", "json", "window-title", "create", "collapsed", "expanded", "new-window", "close-source", "include-stale", "groups", "global", "latest", "no-page", "ungrouped"].includes(key)) {
      options[key] = true;
      continue;
    }


    const value = args.shift();
    if (value == null) {
      errorOut(`Missing value for --${key}`);
    }
    if (key === "signal") {
      if (!options.signal) {
        options.signal = [];
      }
      (options.signal as string[]).push(value as string);
      continue;
    }
    if (key === "tab") {
      if (!options.tab) {
        options.tab = [];
      }
      (options.tab as string[]).push(value as string);
      continue;
    }
    if (key === "agent") {
      if (!options.agent) {
        options.agent = [];
      }
      (options.agent as string[]).push(value as string);
      continue;
    }
    if (key === "url") {
      if (!options.url) {
        options.url = [];
      }
      (options.url as string[]).push(value as string);
      continue;
    }
    if (key === "selector") {
      if (!options.selector) {
        options.selector = [];
      }
      (options.selector as string[]).push(value as string);
      continue;
    }
    options[key] = value;
  }

  return { command, options };
}

function sendRequest(payload: Record<string, unknown>, onProgress?: (data: Record<string, unknown>) => void) {
  return new Promise<Record<string, unknown>>((resolve, reject) => {
    const socketPath = process.env.TABCTL_SOCKET
      || (fs.existsSync(SOCKET_PATH) ? SOCKET_PATH : LEGACY_SOCKET_PATH);
    const client = net.createConnection(socketPath);
    let buffer = "";

    client.on("connect", () => {
      client.write(`${JSON.stringify(payload)}\n`);
    });

    client.on("data", (data) => {
      buffer += data;
      let index;
      while ((index = buffer.indexOf("\n")) >= 0) {
        const line = buffer.slice(0, index).trim();
        buffer = buffer.slice(index + 1);
        if (!line) {
          continue;
        }
        let response: Record<string, unknown>;
        try {
          response = JSON.parse(line);
        } catch (error) {
          client.end();
          client.destroy();
          reject(error);
          return;
        }

        if (response.progress && onProgress) {
          onProgress(response);
          continue;
        }

        client.end();
        client.destroy();
        resolve(response);
        return;
      }
    });

    client.on("error", (error) => {
      reject(error);
    });
  });
}

async function fetchSnapshot() {
  const response = await sendRequest({ id: createId(), action: "list", params: {} });
  if (!response.ok) {
    return null;
  }
  return response.data as Record<string, unknown> | null;
}

function buildTabIndex(snapshot: Record<string, unknown>) {
  const tabIndex = new Map<number, Record<string, unknown>>();
  const windows = (snapshot.windows as Array<Record<string, unknown>>) || [];
  for (const window of windows) {
    const tabs = (window.tabs as Array<Record<string, unknown>>) || [];
    for (const tab of tabs) {
      if (typeof tab.tabId === "number") {
        tabIndex.set(tab.tabId, tab);
      }
    }
  }
  return tabIndex;
}

function buildWindowTitleIndex(snapshot: Record<string, unknown>, policy: Policy | null) {
  const windowTitleIndex = new Map<number, string | null>();
  const windows = (snapshot.windows as Array<Record<string, unknown>>) || [];
  for (const window of windows) {
    const windowId = window.windowId as number;
    if (typeof windowId !== "number") {
      continue;
    }
    const tabs = (window.tabs as Array<Record<string, unknown>>) || [];
    const activeTab = tabs.find((tab) => tab.active === true);
    if (!activeTab) {
      windowTitleIndex.set(windowId, null);
      continue;
    }
    const { eligible } = evaluateTab(activeTab, policy);
    if (!eligible) {
      windowTitleIndex.set(windowId, null);
      continue;
    }
    const title = typeof activeTab.title === "string" ? activeTab.title : null;
    windowTitleIndex.set(windowId, title);
  }
  return windowTitleIndex;
}

function buildWindowLabelIndex(snapshot: Record<string, unknown>) {
  const windowLabels = new Map<number, string>();
  const windows = (snapshot.windows as Array<Record<string, unknown>>) || [];
  windows.forEach((win, index) => {
    const windowId = win.windowId as number;
    if (typeof windowId === "number") {
      windowLabels.set(windowId, `W${index + 1}`);
    }
  });
  return windowLabels;
}

function buildGroupsFromSnapshot(snapshot: Record<string, unknown>, windowId: number | null) {
  const windows = (snapshot.windows as Array<Record<string, unknown>>) || [];
  const windowLabels = buildWindowLabelIndex(snapshot);
  const groups: Array<Record<string, unknown>> = [];
  for (const win of windows) {
    const winId = win.windowId as number;
    if (!Number.isFinite(winId)) {
      continue;
    }
    if (Number.isFinite(windowId) && winId !== windowId) {
      continue;
    }
    const counts = new Map<number, number>();
    const tabs = (win.tabs as Array<Record<string, unknown>>) || [];
    for (const tab of tabs) {
      const groupId = tab.groupId as number;
      if (typeof groupId === "number" && groupId !== -1) {
        counts.set(groupId, (counts.get(groupId) || 0) + 1);
      }
    }
    const windowGroups = (win.groups as Array<Record<string, unknown>>) || [];
    for (const group of windowGroups) {
      const groupId = group.groupId as number;
      if (!Number.isFinite(groupId)) {
        continue;
      }
      groups.push({
        windowId: winId,
        windowLabel: windowLabels.get(winId) ?? null,
        groupId,
        title: group.title ?? null,
        color: group.color ?? null,
        collapsed: group.collapsed ?? null,
        tabCount: counts.get(groupId) || 0,
      });
    }
  }
  return groups;
}

function filterGroupsByScope(groups: Array<Record<string, unknown>>, scope: ReturnType<typeof resolveScopeFlags>, snapshot: Record<string, unknown> | null) {
  let filtered = groups;
  const allScope = !scope.hasScope || scope.ungrouped === true;
  if (!allScope) {
    if (Number.isFinite(scope.windowId)) {
      filtered = filtered.filter((group) => group.windowId === scope.windowId);
    }
    if (Number.isFinite(scope.groupId)) {
      filtered = filtered.filter((group) => group.groupId === scope.groupId);
    }
    if (scope.groupTitle) {
      filtered = filtered.filter((group) => group.title === scope.groupTitle);
    }
    if (scope.tabIds.length > 0 && snapshot) {
      const tabIndex = buildTabIndex(snapshot);
      const groupIds = new Set<number>();
      for (const tabId of scope.tabIds) {
        const tab = tabIndex.get(tabId);
        if (!tab) {
          continue;
        }
        const groupId = tab.groupId as number;
        if (Number.isFinite(groupId) && groupId !== -1) {
          groupIds.add(groupId);
        }
      }
      filtered = filtered.filter((group) => groupIds.has(group.groupId as number));
    }
  }
  return filtered;
}

function formatCliArgValue(value: unknown) {
  const raw = String(value);
  if (!raw) {
    return raw;
  }
  if (/[\s"]/g.test(raw)) {
    const escaped = raw.replace(/"/g, "\\\"");
    return `"${escaped}"`;
  }
  return raw;
}

function buildScopeArgs(options: Options, includeAll: boolean) {
  const args: string[] = [];
  if (includeAll) {
    args.push("--all");
    return args;
  }
  if (options.ungrouped === true) {
    args.push("--ungrouped");
  }
  if (Array.isArray(options.tab)) {
    for (const entry of options.tab as Array<unknown>) {
      if (typeof entry === "string" && entry.trim()) {
        args.push("--tab", formatCliArgValue(entry.trim()));
      }
    }
  }
  if (typeof options.group === "string" && options.group.trim()) {
    args.push("--group", formatCliArgValue(options.group.trim()));
  }
  if (options["group-id"] != null && options.ungrouped !== true) {
    args.push("--group-id", formatCliArgValue(options["group-id"]));
  }
  if (options.window != null) {
    args.push("--window", formatCliArgValue(options.window));
  }
  return args;
}

function parseNumberOption(options: Options, key: string) {
  if (!Object.prototype.hasOwnProperty.call(options, key)) {
    return null;
  }
  const value = Number(options[key]);
  if (!Number.isFinite(value)) {
    errorOut(`Invalid --${key} value`);
  }
  return value;
}

function resolveScopeFlags(options: Options) {
  const tabIds = Array.isArray(options.tab)
    ? (options.tab as Array<unknown>).map(Number).filter(Number.isFinite)
    : [];
  const groupTitle = typeof options.group === "string" ? options.group.trim() : "";
  const ungrouped = options.ungrouped === true;
  const groupId = ungrouped ? -1 : (options["group-id"] != null ? Number(options["group-id"]) : null);
  const windowId = options.window != null ? Number(options.window) : null;
  if (options["group-id"] != null && !Number.isFinite(groupId)) {
    errorOut("Invalid --group-id value");
  }
  if (ungrouped && options["group-id"] != null) {
    errorOut("--ungrouped cannot be combined with --group-id");
  }
  if (options.window != null && !Number.isFinite(windowId)) {
    errorOut("Invalid --window value");
  }
  const hasScope = tabIds.length > 0
    || Boolean(groupTitle)
    || Number.isFinite(groupId)
    || Number.isFinite(windowId);
  return { tabIds, groupTitle, groupId, windowId, hasScope, ungrouped };
}

function buildNextCommand(command: string, scopeArgs: string[], offset: number, limit: number) {
  const parts = ["tabctl", command, ...scopeArgs, "--offset", String(offset), "--limit", String(limit)];
  return parts.join(" ");
}

function resolvePagination(options: Options, total: number, command: string, scopeArgs: string[]) {
  const noPage = options["no-page"] === true;
  if (noPage) {
    return { offset: 0, limit: total, page: null };
  }
  const limitRaw = parseNumberOption(options, "limit");
  const offsetRaw = parseNumberOption(options, "offset");
  const limit = limitRaw != null ? Math.floor(limitRaw) : DEFAULT_PAGE_LIMIT;
  const offset = offsetRaw != null ? Math.floor(offsetRaw) : 0;
  if (!Number.isFinite(limit) || limit <= 0) {
    errorOut("--limit must be a positive number");
  }
  if (!Number.isFinite(offset) || offset < 0) {
    errorOut("--offset must be a non-negative number");
  }
  const remaining = total - offset;
  const returned = remaining > 0 ? Math.min(limit, remaining) : 0;
  const hasMore = offset + limit < total;
  const nextOffset = hasMore ? offset + limit : null;
  const hint = hasMore ? `Partial results. Next: ${buildNextCommand(command, scopeArgs, nextOffset, limit)}` : null;
  return {
    offset,
    limit,
    page: {
      offset,
      limit,
      returned,
      total,
      hasMore,
      nextOffset,
      hint,
    },
  };
}

function compareTabIndex(a: Record<string, unknown>, b: Record<string, unknown>) {
  const aIndex = Number(a.index);
  const bIndex = Number(b.index);
  if (!Number.isFinite(aIndex) && !Number.isFinite(bIndex)) {
    return 0;
  }
  if (!Number.isFinite(aIndex)) {
    return 1;
  }
  if (!Number.isFinite(bIndex)) {
    return -1;
  }
  if (aIndex === bIndex) {
    const aId = Number(a.tabId);
    const bId = Number(b.tabId);
    if (Number.isFinite(aId) && Number.isFinite(bId)) {
      return aId - bId;
    }
    return 0;
  }
  return aIndex - bIndex;
}

function orderTabs(snapshot: Record<string, unknown>, tabFilter: Set<number> | null) {
  const windows = (snapshot.windows as Array<Record<string, unknown>>) || [];
  const ordered: Array<Record<string, unknown>> = [];
  for (const win of windows) {
    const tabs = ((win.tabs as Array<Record<string, unknown>>) || []).slice().sort(compareTabIndex);
    for (const tab of tabs) {
      const tabId = tab.tabId as number;
      if (!tabFilter || tabFilter.has(tabId)) {
        ordered.push(tab);
      }
    }
  }
  return ordered;
}

function buildPagedSnapshot(snapshot: Record<string, unknown>, tabs: Array<Record<string, unknown>>) {
  const tabsByWindow = new Map<number, Array<Record<string, unknown>>>();
  const groupsByWindow = new Map<number, Set<number>>();
  for (const tab of tabs) {
    const windowId = tab.windowId as number;
    if (!Number.isFinite(windowId)) {
      continue;
    }
    if (!tabsByWindow.has(windowId)) {
      tabsByWindow.set(windowId, []);
      groupsByWindow.set(windowId, new Set());
    }
    tabsByWindow.get(windowId)?.push(tab);
    const groupId = tab.groupId as number;
    if (Number.isFinite(groupId) && groupId !== -1) {
      groupsByWindow.get(windowId)?.add(groupId);
    }
  }

  const windows = (snapshot.windows as Array<Record<string, unknown>>) || [];
  const pagedWindows: Array<Record<string, unknown>> = [];
  for (const win of windows) {
    const windowId = win.windowId as number;
    const windowTabs = tabsByWindow.get(windowId) || [];
    if (windowTabs.length === 0) {
      continue;
    }
    const allowedGroupIds = groupsByWindow.get(windowId) || new Set<number>();
    const groups = ((win.groups as Array<Record<string, unknown>>) || []).filter((group) => allowedGroupIds.has(group.groupId as number));
    pagedWindows.push({
      ...win,
      tabs: windowTabs,
      groups,
    });
  }

  return {
    ...snapshot,
    windows: pagedWindows,
  };
}

function listGroupSummaries(snapshot: Record<string, unknown>, windowLabels: Map<number, string>) {
  const windows = (snapshot.windows as Array<Record<string, unknown>>) || [];
  const summaries: Array<Record<string, unknown>> = [];
  for (const win of windows) {
    const groups = (win.groups as Array<Record<string, unknown>>) || [];
    for (const group of groups) {
      summaries.push({
        windowId: win.windowId,
        windowLabel: windowLabels.get(win.windowId as number) ?? null,
        groupId: group.groupId,
        title: typeof group.title === "string" ? group.title : null,
      });
    }
  }
  return summaries;
}

function selectTabsFromSnapshot(snapshot: Record<string, unknown>, params: Record<string, unknown>) {
  const windows = (snapshot.windows as Array<Record<string, unknown>>) || [];
  const allTabs = windows.flatMap((win) => (win.tabs as Array<Record<string, unknown>>) || []);

  if (params.tabIds && (params.tabIds as Array<number>).length) {
    const idSet = new Set((params.tabIds as Array<number>).map(Number));
    return { tabs: allTabs.filter((tab) => idSet.has(tab.tabId as number)) };
  }

  if (params.groupId != null) {
    const groupId = Number(params.groupId);
    return { tabs: allTabs.filter((tab) => tab.groupId === groupId) };
  }

  if (params.groupTitle) {
    const windowLabels = buildWindowLabelIndex(snapshot);
    const matches: Array<{ windowId: number; groupId: number; windowLabel: string | null }> = [];
    for (const win of windows) {
      const groups = (win.groups as Array<Record<string, unknown>>) || [];
      for (const group of groups) {
        if (group.title === params.groupTitle) {
          matches.push({
            windowId: win.windowId as number,
            windowLabel: windowLabels.get(win.windowId as number) ?? null,
            groupId: group.groupId as number,
          });
        }
      }
    }

    const availableGroups = listGroupSummaries(snapshot, windowLabels);

    if (matches.length === 0) {
      return {
        tabs: [],
        error: {
          message: "No matching group title found",
          hint: "Use tabctl group-list to see existing groups.",
          availableGroups,
        },
      };
    }

    if (matches.length > 1 && !params.windowId) {
      return {
        tabs: [],
        error: {
          message: "Group title is ambiguous. Provide a windowId.",
          hint: "Use --window to disambiguate group titles.",
          matches,
          availableGroups,
        },
      };
    }

    const target = params.windowId
      ? matches.find((match) => match.windowId === Number(params.windowId))
      : matches[0];

    if (!target) {
      return {
        tabs: [],
        error: {
          message: "Group title not found in specified window",
          hint: "Use tabctl group-list to see existing groups.",
          matches,
          availableGroups,
        },
      };
    }

    return { tabs: allTabs.filter((tab) => tab.groupId === target.groupId && tab.windowId === target.windowId) };
  }

  if (params.windowId != null) {
    const windowId = Number(params.windowId);
    return { tabs: allTabs.filter((tab) => tab.windowId === windowId) };
  }

  if (params.all) {
    return { tabs: allTabs };
  }

  const focused = windows.find((win) => win.focused);
  return { tabs: focused ? ((focused.tabs as Array<Record<string, unknown>>) || []) : [] };
}

function filterSnapshotByPolicy(snapshot: Record<string, unknown>, policy: Record<string, unknown> | null) {
  if (!policy) {
    return snapshot;
  }

  const windows = (snapshot.windows as Array<Record<string, unknown>>) || [];
  const filteredWindows = windows.map((win) => {
    const tabs = (win.tabs as Array<Record<string, unknown>>) || [];
    const eligibleTabs = tabs
      .filter((tab) => evaluateTab(tab, policy).eligible)
      .map((tab) => annotateEntry(tab, policy));

    const eligibleGroupIds = new Set(
      eligibleTabs
        .map((tab) => tab.groupId)
        .filter((groupId) => typeof groupId === "number" && groupId !== -1) as number[],
    );
    const groups = (win.groups as Array<Record<string, unknown>>) || [];
    const filteredGroups = groups.filter((group) => eligibleGroupIds.has(group.groupId as number));

    return {
      ...win,
      tabs: eligibleTabs,
      groups: filteredGroups,
    };
  }).filter((win) => (win.tabs as Array<Record<string, unknown>>).length > 0);

  return {
    ...snapshot,
    windows: filteredWindows,
  };
}

function printJson(payload: Record<string, unknown>, pretty = true) {
  const output = pretty ? JSON.stringify(payload, null, 2) : JSON.stringify(payload);
  process.stdout.write(`${output}\n`);
}

function resolveBrowser(value: unknown): "edge" | "chrome" | null {
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim().toLowerCase();
  if (trimmed === "edge" || trimmed === "chrome") {
    return trimmed;
  }
  return null;
}

function resolveExtensionId(options: Options) {
  const raw = typeof options["extension-id"] === "string"
    ? String(options["extension-id"])
    : (process.env.TABCTL_EXTENSION_ID || "");
  const value = raw.trim().toLowerCase();
  if (!value) {
    errorOut("Missing --extension-id (or TABCTL_EXTENSION_ID)");
  }
  if (!EXTENSION_ID_PATTERN.test(value)) {
    errorOut(`Extension ID looks unusual: ${raw}`);
  }
  return value;
}

function resolveNodePath(options: Options) {
  const raw = typeof options.node === "string"
    ? String(options.node)
    : (process.env.TABCTL_NODE || process.execPath || "");
  const value = raw.trim();
  if (!value) {
    errorOut("Node binary not found. Set --node or TABCTL_NODE.");
  }
  if (!path.isAbsolute(value)) {
    errorOut(`Node path must be absolute: ${value}`);
  }
  try {
    fs.accessSync(value, fs.constants.X_OK);
  } catch (error) {
    errorOut(`Node binary not executable: ${value}`);
  }
  return value;
}

function resolveHostPath() {
  const root = path.resolve(__dirname, "..");
  const hostPath = path.join(root, "host", "host.js");
  if (!fs.existsSync(hostPath)) {
    errorOut(`Host script not found at ${hostPath}. Run: npm run build`);
  }
  return hostPath;
}

function resolveProjectRoot() {
  try {
    return fs.realpathSync(process.cwd());
  } catch {
    return path.resolve(process.cwd());
  }
}

function resolveConfigHome() {
  return process.env.XDG_CONFIG_HOME || path.join(os.homedir(), ".config");
}

function resolveSkillTargetDir(globalInstall: boolean) {
  if (globalInstall) {
    return path.join(resolveConfigHome(), "opencode", "skills", SKILL_NAME);
  }
  return path.join(resolveProjectRoot(), ".opencode", "skills", SKILL_NAME);
}

function runSkillsCli(args: string[]) {
  const result = spawnSync("npx", ["skills", ...args], { stdio: "pipe" });
  if (result.error) {
    errorOut(`Failed to run skills CLI: ${result.error.message}`);
  }
  if (typeof result.status === "number" && result.status !== 0) {
    const stderr = result.stderr ? result.stderr.toString().trim() : "";
    const stdout = result.stdout ? result.stdout.toString().trim() : "";
    const detail = stderr || stdout;
    const message = detail ? `skills CLI failed: ${detail}` : `skills CLI exited with status ${result.status}`;
    errorOut(message);
  }
}

function runSkillInstall(options: Options, prettyOutput: boolean) {
  const globalInstall = options.global === true;
  const installTarget = resolveSkillTargetDir(globalInstall);
  const agents = Array.isArray(options.agent)
    ? (options.agent as string[]).filter((value) => typeof value === "string" && value.trim())
    : [];
  const args = ["add", SKILL_REPO, "--skill", SKILL_NAME];
  if (agents.length > 0) {
    for (const agent of agents) {
      args.push("-a", agent);
    }
  }
  if (globalInstall) {
    args.push("-g");
  }
  const hintAgents = agents.length > 0 ? agents.map((agent) => `-a ${formatCliArgValue(agent)}`).join(" ") : "";
  const installHintParts = ["npx skills add", formatCliArgValue(SKILL_REPO), "--skill", SKILL_NAME];
  if (hintAgents) {
    installHintParts.push(hintAgents);
  }
  if (globalInstall) {
    installHintParts.push("-g");
  }
  const installHint = installHintParts.join(" ").trim();

  runSkillsCli(args);

  printJson({
    ok: true,
    data: {
      name: SKILL_NAME,
      targetDir: installTarget,
      scope: globalInstall ? "global" : "project",
      installHint,
      tool: "skills",
    },
  }, prettyOutput);
}

function resolveManifestDir(browser: "edge" | "chrome") {
  const home = os.homedir();
  if (!home) {
    errorOut("Home directory not found.");
  }
  if (browser === "edge") {
    return path.join(home, "Library", "Application Support", "Microsoft Edge", "NativeMessagingHosts");
  }
  return path.join(home, "Library", "Application Support", "Google", "Chrome", "NativeMessagingHosts");
}

function writeWrapper(nodePath: string, hostPath: string) {
  const stateHome = process.env.XDG_STATE_HOME || path.join(os.homedir(), ".local", "state");
  const wrapperDir = path.join(stateHome, "tabctl");
  fs.mkdirSync(wrapperDir, { recursive: true, mode: 0o700 });
  const wrapperPath = path.join(wrapperDir, "tabctl-host.sh");
  const escapedNode = nodePath.replace(/"/g, "\\\"");
  const escapedHost = hostPath.replace(/"/g, "\\\"");
  const wrapper = [
    "#!/usr/bin/env bash",
    "set -euo pipefail",
    `exec \"${escapedNode}\" \"${escapedHost}\"`,
    "",
  ].join("\n");
  fs.writeFileSync(wrapperPath, wrapper, "utf8");
  fs.chmodSync(wrapperPath, 0o700);
  return wrapperPath;
}

function runSetup(options: Options, prettyOutput: boolean) {
  if (process.platform !== "darwin") {
    errorOut("tabctl setup is only supported on macOS.");
  }

  const browser = resolveBrowser(options.browser);
  if (!browser) {
    errorOut("Missing or invalid --browser (edge|chrome)");
  }

  const extensionId = resolveExtensionId(options);
  const nodePath = resolveNodePath(options);
  const hostPath = resolveHostPath();
  const wrapperPath = writeWrapper(nodePath, hostPath);
  const manifestDir = resolveManifestDir(browser);
  fs.mkdirSync(manifestDir, { recursive: true });

  const manifestPath = path.join(manifestDir, `${HOST_NAME}.json`);
  const manifest = {
    name: HOST_NAME,
    description: HOST_DESCRIPTION,
    path: wrapperPath,
    type: "stdio",
    allowed_origins: [`chrome-extension://${extensionId}/`],
  };
  fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2), "utf8");

  printJson({
    ok: true,
    data: {
      browser,
      extensionId,
      manifestPath,
      wrapperPath,
      hostPath,
      nodePath,
    },
  }, prettyOutput);
}

function buildHelpData() {
  return {
    commands: [
      "help",
      "list",
      "analyze",
      "dedupe",
      "inspect",
      "focus",
      "refresh",
      "open",
      "group-list",
      "group",
      "group-update",
      "group-ungroup",
      "group-assign",
      "move-tab",
      "move-group",
      "merge-window",
      "setup",
      "policy",
      "archive",
      "close",
      "report",
      "undo",
      "history",
      "ping",
      "skill",
      "version",
    ],
    usage: "tabctl <command> [options]",
    options: {
      analyze: [
        "--stale-days <n>",
        "--github",
        "--github-concurrency <n>",
        "--github-timeout-ms <ms>",
        "--tab <id> (repeatable)",
        "--group <name>",
        "--group-id <id>",
        "--ungrouped",
        "--window <id>",
        "--all",
        "--window-title (include active window title)",
        "--progress",
      ],
      dedupe: [
        "--stale-days <n>",
        "--github",
        "--github-concurrency <n>",
        "--github-timeout-ms <ms>",
        "--tab <id> (repeatable)",
        "--group <name>",
        "--group-id <id>",
        "--ungrouped",
        "--window <id>",
        "--all",
        "--include-stale",
        "--window-title (include active window title)",
        "--progress",
        "--confirm",
      ],
      list: [
        "--tab <id> (repeatable)",
        "--group <name>",
        "--group-id <id>",
        "--ungrouped",
        "--window <id>",
        "--all",
        "--limit <n>",
        "--offset <n>",
        "--no-page",
        "--groups (alias for group-list)",
      ],
      inspect: [
        "--signal-config <path>",
        "--signal <id> (repeatable)",
        "--selector <name=css|json> (repeatable)",
        "--signal-concurrency <n>",
        "--signal-timeout-ms <ms>",
        "--tab <id> (repeatable)",
        "--group <name>",
        "--group-id <id>",
        "--ungrouped",
        "--window <id>",
        "--all",
        "--limit <n>",
        "--offset <n>",
        "--no-page",
        "--progress",
      ],
      focus: [
        "--tab <id>",
      ],
      refresh: [
        "--tab <id>",
      ],
      open: [
        "--url <url> (repeatable)",
        "--group <name>",
        "--color <name>",
        "--after-group <name>",
        "--window <id>",
        "--new-window",
        "--window-group <name>",
        "--window-tab <id>",
        "--window-url <substring>",
      ],
      "group-list": [
        "--tab <id> (repeatable)",
        "--group <name>",
        "--group-id <id>",
        "--ungrouped",
        "--window <id>",
        "--all",
        "--limit <n>",
        "--offset <n>",
        "--no-page",
      ],
      group: [
        "(alias for group-list)",
        "--window <id>",
      ],
      "group-update": [
        "--group <name>",
        "--group-id <id>",
        "--window <id>",
        "--title <name>",
        "--color <name>",
        "--collapsed",
        "--expanded",
      ],
      "group-ungroup": [
        "--group <name>",
        "--group-id <id>",
        "--window <id>",
      ],
      "group-assign": [
        "--tab <id> (repeatable)",
        "--group <name>",
        "--group-id <id>",
        "--window <id>",
        "--create",
        "--color <name>",
        "--collapsed",
        "--expanded",
      ],
      "move-tab": [
        "--tab <id>",
        "--before-tab <id>",
        "--after-tab <id>",
        "--before-group <name>",
        "--after-group <name>",
        "--window <id>",
        "--new-window",
      ],
      "move-group": [
        "--group <name>",
        "--group-id <id>",
        "--before-tab <id>",
        "--after-tab <id>",
        "--before-group <name>",
        "--after-group <name>",
        "--window <id>",
        "--new-window",
      ],
      "merge-window": [
        "--from <id>",
        "--to <id>",
        "--close-source",
        "--confirm",
      ],
      setup: [
        "--browser edge|chrome",
        "--extension-id <id>",
        "--node <path>",
      ],
      policy: [
        "--init",
      ],
      archive: [
        "--all",
        "--window <id>",
        "--group <name>",
        "--group-id <id>",
        "--ungrouped",
        "--tab <id> (repeatable)",
      ],
      close: [
        "--apply <analysisId>",
        "--tab <id> (repeatable)",
        "--group <name>",
        "--group-id <id>",
        "--ungrouped",
        "--window <id>",
        "--confirm",
        "--dry-run",
      ],
      report: [
        "--format json|md|csv",
        "--out <path>",
        "--tab <id> (repeatable)",
        "--group <name>",
        "--group-id <id>",
        "--ungrouped",
        "--window <id>",
        "--all",
        "--limit <n>",
        "--offset <n>",
        "--no-page",
      ],
      history: [
        "--limit <n>",
      ],
      undo: [
        "<txid>",
        "--txid <id>",
        "--latest",
      ],
      skill: [
        "--agent <name> (repeatable)",
        "--global",
      ],
      version: [],
      global: [
        "--help",
        "--json",
      ],
    },
    version: VERSION,
  };
}

function printHelp(jsonOutput: boolean) {
  const data = buildHelpData();
  if (jsonOutput) {
    printJson({ ok: true, data });
    return;
  }

  const lines: string[] = [];
  lines.push("tabctl - Edge tab management CLI");
  lines.push(`Version: ${data.version}`);
  lines.push("");
  lines.push(`Usage: ${data.usage}`);
  lines.push("");
  lines.push("Commands:");
  lines.push(`  ${data.commands.join(", ")}`);
  lines.push("");
  lines.push("Options:");
  for (const [section, options] of Object.entries(data.options)) {
    lines.push(`  ${section}:`);
    for (const option of options as string[]) {
      lines.push(`    ${option}`);
    }
  }
  lines.push("");
  lines.push("Notes:");
  lines.push("  --before-group/--after-group only position tabs; use group-assign to move tabs into a group.");
  lines.push("  undo accepts a txid as a positional arg (or --txid) and supports --latest.");
  lines.push("");
  lines.push("Policy: $XDG_CONFIG_HOME/tabctl/policy.json (or ~/.config/tabctl/policy.json)");
  lines.push("Policy is enforced when the file exists; missing file means no policy.");
  process.stdout.write(lines.join("\n") + "\n");
}

function setupStdoutErrorHandling() {
  process.stdout.on("error", (error: NodeJS.ErrnoException) => {
    if (error.code === "EPIPE") {
      process.exit(0);
    }
    throw error;
  });
}

function errorOut(message: string): never {
  const hints: Record<string, string> = {
    "Unknown option: --format": "Use --json for JSON output. --format is only for report.",
  };
  const hint = hints[message];
  if (hint) {
    printJson({ ok: false, error: { message, hint } });
  } else {
    printJson({ ok: false, error: { message } });
  }
  process.exit(1);
  throw new Error(message);
}

function emitVersionWarnings(response: Record<string, unknown>, fallbackAction: string) {
  const hostVersion = typeof response.version === "string" ? response.version : null;
  if (hostVersion && hostVersion !== VERSION) {
    process.stderr.write(`[tabctl] version mismatch: cli ${VERSION}, host ${hostVersion}\n`);
  }

  const data = response.data as Record<string, unknown> | undefined;
  const extensionVersion = data && typeof data.extensionVersion === "string" ? (data.extensionVersion as string) : null;
  const extensionComponent = data && typeof data.extensionComponent === "string" ? (data.extensionComponent as string) : null;
  if (extensionVersion && hostVersion && extensionVersion !== hostVersion) {
    process.stderr.write(`[tabctl] version mismatch: host ${hostVersion}, extension ${extensionVersion}\n`);
  }
  if (extensionComponent && extensionComponent !== "extension") {
    process.stderr.write(`[tabctl] unexpected extension component: ${extensionComponent}\n`);
  }

  const action = (response.action as string | undefined) || fallbackAction;
  const extensionExpected = !["history", "version"].includes(action);
  if (extensionExpected && !extensionVersion) {
    process.stderr.write("[tabctl] extension version unavailable; reload the extension to validate version match\n");
  }
}

async function main() {
  setupStdoutErrorHandling();
  let { command, options } = parseArgs(process.argv.slice(2));
  if (command === "dedupe" && (options as Record<string, unknown>).close) {
    errorOut("dedupe does not support --close; use --confirm or close --apply <analysisId>.");
  }
  const prettyOutput = options.pretty !== false;
  if (command === "groups" || command === "group") {
    command = "group-list";
  }
  if (command === "list" && options.groups === true) {
    command = "group-list";
  }
  if (options.format && command !== "report") {
    errorOut("Unknown option: --format");
  }
  if (Object.prototype.hasOwnProperty.call(options, "policy")) {
    errorOut("Custom policy path is not supported. Use XDG_CONFIG_HOME/tabctl/policy.json.");
  }

  if (command === "refresh") {
    const tabValues = Array.isArray(options.tab)
      ? (options.tab as string[]).map((value) => String(value).trim()).filter(Boolean)
      : [];
    if (tabValues.length === 0) {
      errorOut("refresh requires --tab");
    }
    if (tabValues.length > 1) {
      errorOut("refresh requires a single --tab");
    }
  }

  if (command === "open" && options.color && !options.group) {
    errorOut("--color requires --group");
  }

  if (command === "undo") {
    if (options.txid && options._.length > 0) {
      errorOut("undo requires a single txid (use positional arg or --txid)");
    }
    if (options.latest === true && options._.length > 0) {
      errorOut("undo --latest cannot be combined with a txid");
    }
    if (options.latest === true && options.txid) {
      errorOut("undo --latest cannot be combined with --txid");
    }
    if (options.txid && options._.length === 0) {
      options._ = [String(options.txid)];
    }
  }

  if (command === "inspect") {
    const selectorCount = Array.isArray(options.selector) ? options.selector.length : 0;
    if (selectorCount > 0) {
      const signalList = normalizeSignals(options.signal);
      if (!signalList.includes("selector")) {
        signalList.push("selector");
      }
      options.signal = signalList;
    }
    const signalList = normalizeSignals(options.signal);
    if (signalList.length > 0) {
      validateSignals(signalList);
      options.signal = signalList;
    }
  }

  const policyContext = loadPolicy();
  const policySummary = summarizePolicy(policyContext.policy, policyContext.path);
  const policyEnabled = policyContext.policy !== null;
  const enforcePolicy = policyEnabled;
  const includeWindowTitle = options["window-title"] === true;
  const includeStale = options["include-stale"] === true;
  let policySnapshot: Record<string, unknown> | null = null;
  const getPolicySnapshot = async () => {
    if (!policySnapshot) {
      policySnapshot = await fetchSnapshot();
    }
    return policySnapshot;
  };

  if (!command || command === "help" || options.help) {
    printHelp(options.json === true);
    return;
  }

  if (command === "skill") {
    runSkillInstall(options, prettyOutput);
    return;
  }

  if (command === "setup") {
    runSetup(options, prettyOutput);
    return;
  }

  if (command === "version") {
    printJson({
      ok: true,
      data: {
        version: VERSION,
        baseVersion: BASE_VERSION,
        gitSha: GIT_SHA,
        dirty: DIRTY,
        component: "cli",
      },
    }, prettyOutput);
    return;
  }

  if (command === "policy") {
    const policyPath = defaultPolicyPath();
    if (options.init) {
      if (fs.existsSync(policyPath)) {
        printJson({
          ok: true,
          data: {
            status: "exists",
            path: policyPath,
          },
        }, prettyOutput);
        return;
      }

      const dir = path.dirname(policyPath);
      fs.mkdirSync(dir, { recursive: true });
      fs.writeFileSync(policyPath, JSON.stringify(defaultPolicyTemplate(), null, 2), "utf8");
      printJson({
        ok: true,
        data: {
          status: "created",
          path: policyPath,
        },
      }, prettyOutput);
      return;
    }

    printJson({
      ok: true,
      data: {
        ...policySummary,
        path: policyPath,
      },
    }, prettyOutput);
    return;
  }


  let dedupeMode = false;
  if (command === "close" && options["dry-run"]) {
    command = "analyze";
  }
  if (command === "dedupe") {
    dedupeMode = true;
    command = "analyze";
  }

  let action = command;
  let params: Record<string, unknown> = {};
  let policyInfo: Record<string, unknown> | null = null;
  let earlyResponse: Record<string, unknown> | null = null;

  switch (command) {
    case "list":
      action = "list";
      break;
    case "ping":
      action = "ping";
      break;
    case "analyze":
      action = "analyze";
      params = {
        staleDays: options["stale-days"] ? Number(options["stale-days"]) : undefined,
        checkGitHub: Boolean(options.github),
        tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
        groupTitle: options.group,
        groupId: options.ungrouped ? -1 : (options["group-id"] ? Number(options["group-id"]) : undefined),
        windowId: options.window ? Number(options.window) : undefined,
        all: options.all === true,
        githubConcurrency: options["github-concurrency"] ? Number(options["github-concurrency"]) : undefined,
        githubTimeoutMs: options["github-timeout-ms"] ? Number(options["github-timeout-ms"]) : undefined,
        progress: Boolean(options.progress),
      };
      if (options.ungrouped && options["group-id"]) {
        errorOut("--ungrouped cannot be combined with --group-id");
      }
      break;
    case "inspect":
      action = "inspect";
      let selectorSpecs: Array<Record<string, unknown>> | undefined;
      if (options.selector) {
        selectorSpecs = (options.selector as string[]).map((value) => {
          const trimmed = value.trim();
          if (trimmed.startsWith("{")) {
            try {
              return JSON.parse(trimmed) as Record<string, unknown>;
            } catch (error) {
              errorOut(`Invalid selector JSON: ${trimmed}`);
            }
          }
          if (trimmed.includes("=")) {
            const [name, selector] = trimmed.split(/=(.+)/);
            return { name, selector };
          }
          return { selector: trimmed };
        }).filter(Boolean) as Array<Record<string, unknown>>;
      }

      let signalConfig: Record<string, unknown> | undefined;
      if (options["signal-config"]) {
        try {
          const configRaw = fs.readFileSync(String(options["signal-config"]), "utf8");
          signalConfig = JSON.parse(configRaw) as Record<string, unknown>;
        } catch (error) {
          errorOut("Failed to read --signal-config file");
        }
      }

      params = {
        all: Boolean(options.all),
        windowId: options.window ? Number(options.window) : undefined,
        groupTitle: options.group,
        groupId: options.ungrouped ? -1 : (options["group-id"] ? Number(options["group-id"]) : undefined),
        tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
        signals: options.signal ? (options.signal as string[]) : undefined,
        selectorSpecs,
        signalConfig,
        signalConcurrency: options["signal-concurrency"] ? Number(options["signal-concurrency"]) : undefined,
        signalTimeoutMs: options["signal-timeout-ms"] ? Number(options["signal-timeout-ms"]) : undefined,
        progress: Boolean(options.progress),
      };
      if (options.ungrouped && options["group-id"]) {
        errorOut("--ungrouped cannot be combined with --group-id");
      }
      break;
    case "focus":
      action = "focus";
      params = {
        tabId: options.tab ? Number((options.tab as string[])[0]) : undefined,
        tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
      };
      break;
    case "refresh":
      action = "refresh";
      params = {
        tabId: options.tab ? Number((options.tab as string[])[0]) : undefined,
        tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
      };
      break;
    case "open":
      action = "open";
      params = {
        urls: options.url ? (options.url as string[]).map(String) : undefined,
        groupTitle: options.group,
        color: normalizeGroupColor(options.color),
        afterGroupTitle: options["after-group"],
        windowId: options.window ? Number(options.window) : undefined,
        newWindow: options["new-window"] === true,
        windowGroupTitle: options["window-group"],
        windowTabId: options["window-tab"] ? Number(options["window-tab"]) : undefined,
        windowUrl: options["window-url"],
      };
      break;
    case "group-list":
      action = "group-list";
      params = {
        windowId: options.window ? Number(options.window) : undefined,
      };
      break;
    case "group-update":
      action = "group-update";
      params = {
        groupTitle: options.group,
        groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
        windowId: options.window ? Number(options.window) : undefined,
        title: options.title,
        color: options.color,
        collapsed: options.collapsed === true ? true : options.expanded === true ? false : undefined,
      };
      break;
    case "group-ungroup":
      action = "group-ungroup";
      params = {
        groupTitle: options.group,
        groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
        windowId: options.window ? Number(options.window) : undefined,
      };
      break;
    case "group-assign":
      action = "group-assign";
      params = {
        tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
        groupTitle: options.group,
        groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
        windowId: options.window ? Number(options.window) : undefined,
        create: Boolean(options.create),
        color: options.color,
        collapsed: options.collapsed === true ? true : options.expanded === true ? false : undefined,
      };
      break;
    case "move-tab":
      action = "move-tab";
      params = {
        tabId: options.tab ? Number((options.tab as string[])[0]) : undefined,
        tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
        beforeTabId: options["before-tab"] ? Number(options["before-tab"]) : undefined,
        afterTabId: options["after-tab"] ? Number(options["after-tab"]) : undefined,
        beforeGroupTitle: options["before-group"],
        afterGroupTitle: options["after-group"],
        windowId: options.window ? Number(options.window) : undefined,
        newWindow: options["new-window"] === true,
      };
      break;
    case "move-group":
      action = "move-group";
      params = {
        groupTitle: options.group,
        groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
        beforeTabId: options["before-tab"] ? Number(options["before-tab"]) : undefined,
        afterTabId: options["after-tab"] ? Number(options["after-tab"]) : undefined,
        beforeGroupTitle: options["before-group"],
        afterGroupTitle: options["after-group"],
        windowId: options.window ? Number(options.window) : undefined,
        newWindow: options["new-window"] === true,
      };
      break;
    case "merge-window":
      action = "merge-window";
      params = {
        fromWindowId: options.from ? Number(options.from) : undefined,
        toWindowId: options.to ? Number(options.to) : undefined,
        windowId: options.from ? Number(options.from) : undefined,
        closeSource: options["close-source"] === true,
        confirmed: options.confirm === true,
      };
      break;
    case "archive":
      action = "archive";
      params = {
        all: Boolean(options.all),
        windowId: options.window ? Number(options.window) : undefined,
        groupTitle: options.group,
        groupId: options.ungrouped ? -1 : (options["group-id"] ? Number(options["group-id"]) : undefined),
        tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
      };
      if (options.ungrouped && options["group-id"]) {
        errorOut("--ungrouped cannot be combined with --group-id");
      }
      break;
    case "close":
      action = "close";
      if (options.apply) {
        params = { mode: "apply", analysisId: options.apply };
      } else {
        if (!options.confirm) {
          errorOut("Direct close requires --confirm");
        }
        params = {
          mode: "direct",
          confirmed: true,
          windowId: options.window ? Number(options.window) : undefined,
          groupTitle: options.group,
          groupId: options.ungrouped ? -1 : (options["group-id"] ? Number(options["group-id"]) : undefined),
          tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
        };
        if (options.ungrouped && options["group-id"]) {
          errorOut("--ungrouped cannot be combined with --group-id");
        }
      }
      break;
    case "report":
      action = "report";
      params = {
        all: Boolean(options.all),
        windowId: options.window ? Number(options.window) : undefined,
        groupTitle: options.group,
        groupId: options.ungrouped ? -1 : (options["group-id"] ? Number(options["group-id"]) : undefined),
        tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
      };
      if (options.ungrouped && options["group-id"]) {
        errorOut("--ungrouped cannot be combined with --group-id");
      }
      break;
    case "undo":
      action = "undo";
      params = { txid: options._[0], latest: options.latest === true };
      break;
    case "history":
      action = "history";
      params = { limit: options.limit ? Number(options.limit) : undefined };
      break;
    case "version":
      action = "version";
      params = {};
      break;
    default:
      errorOut(`Unknown command: ${command}`);
  }

  if (command === "analyze") {
    const tabIds = (params as { tabIds?: number[] }).tabIds;
    const hasScope = (Array.isArray(tabIds) && tabIds.length > 0)
      || Boolean((params as { groupTitle?: string }).groupTitle)
      || Number.isFinite((params as { groupId?: number }).groupId)
      || Number.isFinite((params as { windowId?: number }).windowId)
      || (params as { all?: boolean }).all === true;
    if (!hasScope) {
      params = { ...params, all: true };
    }
  }

  if (command === "merge-window") {
    const fromWindowId = (params as { fromWindowId?: number }).fromWindowId;
    const toWindowId = (params as { toWindowId?: number }).toWindowId;
    if (!Number.isFinite(fromWindowId) || !Number.isFinite(toWindowId)) {
      errorOut("merge-window requires --from and --to window ids");
    }
    if (fromWindowId === toWindowId) {
      errorOut("merge-window --from and --to cannot be the same window");
    }
    if ((params as { closeSource?: boolean }).closeSource && !(params as { confirmed?: boolean }).confirmed) {
      errorOut("merge-window --close-source requires --confirm");
    }
  }

  if (enforcePolicy && ["analyze", "inspect", "report", "close", "archive", "focus", "refresh", "move-tab", "move-group", "group-assign", "group-update", "group-ungroup", "merge-window"].includes(command)) {
    if (command === "close" && options.apply) {
      errorOut("Policy blocks close --apply; use explicit tab targets.");
    }

    const snapshot = await getPolicySnapshot();
    if (!snapshot) {
      errorOut("Failed to load tabs for policy evaluation");
    }

    const selection = selectTabsFromSnapshot(snapshot, params);
    if ((selection as { error?: Record<string, unknown> }).error) {
      printJson({ ok: false, error: (selection as { error: Record<string, unknown> }).error }, prettyOutput);
      process.exit(1);
    }

    const selectedTabs = (selection as { tabs: Array<Record<string, unknown>> }).tabs;
    const eligibleTabs = selectedTabs.filter((tab) => evaluateTab(tab, policyContext.policy).eligible);
    const protectedTabs = selectedTabs.filter((tab) => !evaluateTab(tab, policyContext.policy).eligible);
    const eligibleIds = eligibleTabs.map((tab) => tab.tabId).filter((id) => typeof id === "number") as number[];

    if (command === "focus" || command === "refresh") {
      if (!eligibleIds.length) {
        errorOut(`Tab is protected by policy and cannot be ${command === "focus" ? "focused" : "refreshed"} via CLI`);
      }
      params = {
        tabId: eligibleIds[0],
      };
    } else if (command === "close" || command === "archive") {
      if (!eligibleIds.length) {
        earlyResponse = {
          ok: true,
          action: command,
          data: {
            summary: { eligible: 0, protected: protectedTabs.length },
            protected: protectedTabs.map((tab) => ({
              tabId: tab.tabId,
              windowId: tab.windowId,
              groupId: tab.groupId,
              groupTitle: tab.groupTitle,
              title: tab.title,
              url: tab.url,
              pinned: tab.pinned,
            })),
            policy: policySummary,
          },
        };
      } else if (command === "close") {
        params = {
          mode: "direct",
          confirmed: true,
          tabIds: eligibleIds,
        };
      } else if (command === "archive") {
        params = {
          tabIds: eligibleIds,
        };
      }

      policyInfo = {
        protected: protectedTabs.map((tab) => ({
          tabId: tab.tabId,
          windowId: tab.windowId,
          groupId: tab.groupId,
          groupTitle: tab.groupTitle,
          title: tab.title,
          url: tab.url,
          pinned: tab.pinned,
        })),
      };
    } else if (command === "move-tab" || command === "move-group" || command === "group-assign") {
      if (!eligibleIds.length || (command === "move-group" && protectedTabs.length > 0)) {
        earlyResponse = {
          ok: true,
          action: command,
          data: {
            summary: { eligible: eligibleIds.length, protected: protectedTabs.length },
            protected: protectedTabs.map((tab) => ({
              tabId: tab.tabId,
              windowId: tab.windowId,
              groupId: tab.groupId,
              groupTitle: tab.groupTitle,
              title: tab.title,
              url: tab.url,
              pinned: tab.pinned,
            })),
            policy: policySummary,
          },
        };
      } else if (command === "move-tab" || command === "group-assign") {
        params = {
          ...params,
          tabId: eligibleIds[0],
          tabIds: eligibleIds,
        };
      }

      policyInfo = {
        protected: protectedTabs.map((tab) => ({
          tabId: tab.tabId,
          windowId: tab.windowId,
          groupId: tab.groupId,
          groupTitle: tab.groupTitle,
          title: tab.title,
          url: tab.url,
          pinned: tab.pinned,
        })),
      };
    } else if (command === "merge-window") {
      if (!eligibleIds.length) {
        earlyResponse = {
          ok: true,
          action: command,
          data: {
            summary: { eligible: 0, protected: protectedTabs.length },
            protected: protectedTabs.map((tab) => ({
              tabId: tab.tabId,
              windowId: tab.windowId,
              groupId: tab.groupId,
              groupTitle: tab.groupTitle,
              title: tab.title,
              url: tab.url,
              pinned: tab.pinned,
            })),
            policy: policySummary,
          },
        };
      }

      params = {
        ...params,
        tabIds: eligibleIds,
      };

      policyInfo = {
        protected: protectedTabs.map((tab) => ({
          tabId: tab.tabId,
          windowId: tab.windowId,
          groupId: tab.groupId,
          groupTitle: tab.groupTitle,
          title: tab.title,
          url: tab.url,
          pinned: tab.pinned,
        })),
      };
    } else if (command === "group-update" || command === "group-ungroup") {
      if (!eligibleIds.length || protectedTabs.length > 0) {
        earlyResponse = {
          ok: true,
          action: command,
          data: {
            summary: { eligible: eligibleIds.length, protected: protectedTabs.length },
            protected: protectedTabs.map((tab) => ({
              tabId: tab.tabId,
              windowId: tab.windowId,
              groupId: tab.groupId,
              groupTitle: tab.groupTitle,
              title: tab.title,
              url: tab.url,
              pinned: tab.pinned,
            })),
            policy: policySummary,
          },
        };
      }

      policyInfo = {
        protected: protectedTabs.map((tab) => ({
          tabId: tab.tabId,
          windowId: tab.windowId,
          groupId: tab.groupId,
          groupTitle: tab.groupTitle,
          title: tab.title,
          url: tab.url,
          pinned: tab.pinned,
        })),
      };
    } else {
      if (!eligibleIds.length) {
        const generatedAt = Date.now();
        if (command === "analyze") {
          earlyResponse = {
            ok: true,
            action: command,
            data: {
              generatedAt,
              staleDays: params.staleDays || 0,
              totals: { tabs: 0, analyzed: 0, candidates: 0 },
              meta: { durationMs: 0, githubChecked: 0, githubTotal: 0, githubMatched: 0, githubTimeoutMs: params.githubTimeoutMs || 0 },
              candidates: [],
              analysisId: null,
              policy: policySummary,
            },
          };
        } else {
          earlyResponse = {
            ok: true,
            action: command,
            data: {
              generatedAt,
              entries: [],
              totals: { tabs: 0, signals: 0, tasks: 0 },
              meta: { durationMs: 0, signalTimeoutMs: params.signalTimeoutMs || 0, selectorCount: 0 },
              policy: policySummary,
            },
          };
        }
      } else {
        params = {
          ...params,
          tabIds: eligibleIds,
        };
      }
    }
  }

  const request = {
    id: createId(),
    action,
    params,
    client: {
      component: "cli",
      version: VERSION,
      baseVersion: BASE_VERSION,
      gitSha: GIT_SHA,
      dirty: DIRTY,
    },
  };

  let response: Record<string, unknown>;
  const showProgress = options.progress === true;
  const startedAt = Date.now();
  let progressTimer: NodeJS.Timeout | null = null;
  if (showProgress) {
    progressTimer = setInterval(() => {
      const elapsed = Math.round((Date.now() - startedAt) / 1000);
      process.stderr.write(`[tabctl] waiting ${elapsed}s...\n`);
    }, 2000);
  }

  const onProgress = showProgress
    ? (message: Record<string, unknown>) => {
      const data = message.data as Record<string, unknown> | undefined;
      if (data?.phase === "github") {
        const processed = data.processed as number;
        const total = data.total as number;
        const matched = data.matched as number;
        process.stderr.write(`[tabctl] github ${processed}/${total} (matched ${matched})\n`);
      }
      if (data?.phase === "inspect") {
        const processed = data.processed as number;
        const total = data.total as number;
        const signalId = data.signalId as string;
        process.stderr.write(`[tabctl] inspect ${processed}/${total} (${signalId})\n`);
      }
    }
    : undefined;

  if (earlyResponse) {
    response = earlyResponse;
  } else {
    try {
      response = await sendRequest(request, onProgress);
    } catch (error) {
      if (progressTimer) {
        clearInterval(progressTimer);
      }
      const message = error instanceof Error ? error.message : "Unknown error";
      errorOut(`Failed to connect to host: ${message}`);
      return;
    } finally {
      if (progressTimer) {
        clearInterval(progressTimer);
      }
    }
  }

  if (progressTimer) {
    clearInterval(progressTimer);
  }

  if (!response) {
    errorOut("No response received");
  }

  if (!response.ok) {
    printJson(response, prettyOutput);
    process.exit(1);
  }

  if (response.data && typeof response.data === "object") {
    const data = response.data as Record<string, unknown>;
    if (command === "group-list" && (!Array.isArray(data.groups) || data.groups === null)) {
      const scope = resolveScopeFlags(options);
      const snapshot = await getPolicySnapshot();
      if (snapshot) {
        const scopeWindow = Number.isFinite(scope.windowId) ? scope.windowId : null;
        const groups = buildGroupsFromSnapshot(snapshot, scopeWindow);
        data.groups = filterGroupsByScope(groups, scope, snapshot);
      } else {
        data.groups = [];
      }
    }
    if (command === "list" && Array.isArray(data.windows)) {
      const filtered = filterSnapshotByPolicy(data, policyContext.policy) as Record<string, unknown>;
      const scope = resolveScopeFlags(options);
      const allScope = options.all === true || !scope.hasScope;
      const listParams: Record<string, unknown> = allScope
        ? { all: true }
        : {
          tabIds: scope.tabIds.length ? scope.tabIds : undefined,
          groupTitle: scope.groupTitle || undefined,
          groupId: scope.groupId != null ? scope.groupId : undefined,
          windowId: scope.windowId != null ? scope.windowId : undefined,
        };
      const selection = selectTabsFromSnapshot(filtered, listParams) as { tabs: Array<Record<string, unknown>>; error?: Record<string, unknown> };
      if (selection.error) {
        printJson({ ok: false, error: selection.error }, prettyOutput);
        process.exit(1);
      }
      const selectedTabs = selection.tabs || [];
      const tabIdSet = new Set(selectedTabs.map((tab) => tab.tabId).filter((id) => typeof id === "number") as number[]);
      const ordered = listParams.all
        ? orderTabs(filtered, null)
        : (tabIdSet.size > 0 ? orderTabs(filtered, tabIdSet) : []);
      const scopeArgs = buildScopeArgs(options, allScope);
      const pagination = resolvePagination(options, ordered.length, "list", scopeArgs);
      const start = pagination.offset;
      const end = pagination.offset + pagination.limit;
      const pagedTabs = ordered.slice(start, end);
      const pagedSnapshot = buildPagedSnapshot(filtered, pagedTabs) as Record<string, unknown>;
      data.windows = pagedSnapshot.windows as Array<Record<string, unknown>>;
      if (pagination.page) {
        data.page = pagination.page;
      }
    }

    if (command === "group-list" && Array.isArray(data.groups)) {
      let groups = data.groups as Array<Record<string, unknown>>;
      const scope = resolveScopeFlags(options);
      const allScope = options.all === true || !scope.hasScope;
      if (!allScope) {
        if (Number.isFinite(scope.windowId)) {
          groups = groups.filter((group) => group.windowId === scope.windowId);
        }
        if (Number.isFinite(scope.groupId)) {
          groups = groups.filter((group) => group.groupId === scope.groupId);
        }
        if (scope.groupTitle) {
          groups = groups.filter((group) => group.title === scope.groupTitle);
        }
        if (scope.tabIds.length > 0) {
          const snapshot = await getPolicySnapshot();
          if (!snapshot) {
            errorOut("Failed to load tabs for group-list filtering");
          }
          const tabIndex = buildTabIndex(snapshot);
          const groupIds = new Set<number>();
          for (const tabId of scope.tabIds) {
            const tab = tabIndex.get(tabId);
            if (!tab) {
              continue;
            }
            const groupId = tab.groupId as number;
            if (Number.isFinite(groupId) && groupId !== -1) {
              groupIds.add(groupId);
            }
          }
          groups = groups.filter((group) => groupIds.has(group.groupId as number));
        }
      }

      const scopeArgs = buildScopeArgs(options, allScope);
      const pagination = resolvePagination(options, groups.length, "group-list", scopeArgs);
      const start = pagination.offset;
      const end = pagination.offset + pagination.limit;
      data.groups = groups.slice(start, end);
      if (pagination.page) {
        data.page = pagination.page;
      }
    }

    if ((command === "inspect" || command === "report") && Array.isArray(data.entries)) {
      let snapshot: Record<string, unknown> | null = null;
      if (policyEnabled) {
        snapshot = await getPolicySnapshot();
      }
      const tabIndex = snapshot ? buildTabIndex(snapshot) : null;
      const annotated = (data.entries as Array<Record<string, unknown>>).map((entry) => {
        const tab = tabIndex?.get(entry.tabId as number) || entry;
        const { eligible, protectedReasons } = evaluateTab(tab, policyContext.policy);
        return {
          ...entry,
          eligible,
          protectedReasons,
        };
      }).filter((entry) => entry.eligible !== false);
      const scope = resolveScopeFlags(options);
      const allScope = options.all === true || !scope.hasScope;
      const scopeArgs = buildScopeArgs(options, allScope);
      const pagination = resolvePagination(options, annotated.length, command, scopeArgs);
      const start = pagination.offset;
      const end = pagination.offset + pagination.limit;
      data.entries = annotated.slice(start, end);
      if (pagination.page) {
        data.page = pagination.page;
      }
    }

    if (command === "analyze" && Array.isArray(data.candidates)) {
      let snapshot: Record<string, unknown> | null = null;
      if (policyEnabled || includeWindowTitle) {
        snapshot = await getPolicySnapshot();
      }
      const tabIndex = snapshot ? buildTabIndex(snapshot) : null;
      const windowTitleIndex = snapshot && includeWindowTitle
        ? buildWindowTitleIndex(snapshot, policyContext.policy)
        : null;
      data.candidates = (data.candidates as Array<Record<string, unknown>>).map((candidate) => {
        const tab = tabIndex?.get(candidate.tabId as number) || candidate;
        const { eligible, protectedReasons } = evaluateTab(tab, policyContext.policy);
        const windowTitle = includeWindowTitle
          ? (windowTitleIndex?.get(candidate.windowId as number) ?? null)
          : undefined;
        return {
          ...candidate,
          eligible,
          protectedReasons,
          ...(includeWindowTitle ? { windowTitle } : {}),
        };
      }).filter((candidate) => candidate.eligible !== false);
    }

    data.policy = policySummary;
    if (policyInfo) {
      data.policyInfo = policyInfo;
    }
  } else if (response.ok) {
    response.policy = policySummary;
  }

  if (dedupeMode) {
    if (!response.ok) {
      printJson(response, prettyOutput);
      return;
    }

    const data = (response.data as Record<string, unknown>) || {};
    const candidates = Array.isArray(data.candidates) ? (data.candidates as Array<Record<string, unknown>>) : [];
    const planned = candidates.filter((candidate) => {
      const reasons = Array.isArray(candidate.reasons) ? (candidate.reasons as Array<Record<string, unknown>>) : [];
      const hasDuplicate = reasons.some((reason) => reason.type === "duplicate" || reason.type === "closed_issue");
      const hasStale = reasons.some((reason) => reason.type === "stale");
      return hasDuplicate || (includeStale && hasStale);
    });

    const planTabIds: number[] = [];
    const expectedUrls: Record<string, string> = {};
    for (const candidate of planned) {
      const tabId = candidate.tabId as number;
      if (!Number.isFinite(tabId)) {
        continue;
      }
      if (!planTabIds.includes(tabId)) {
        planTabIds.push(tabId);
      }
      if (typeof candidate.url === "string") {
        expectedUrls[String(tabId)] = candidate.url;
      }
    }

    let closeData: Record<string, unknown> | null = null;
    if (options.confirm === true && planTabIds.length > 0) {
      const closeResponse = await sendRequest({
        id: createId(),
        action: "close",
        params: {
          mode: "direct",
          confirmed: true,
          tabIds: planTabIds,
          expectedUrls,
        },
      });
      if (!closeResponse.ok) {
        printJson(closeResponse, prettyOutput);
        return;
      }
      closeData = (closeResponse.data as Record<string, unknown>) || {};
      closeData.policy = policySummary;
      if (policyInfo) {
        closeData.policyInfo = policyInfo;
      }
    }

    const closeSummary = closeData?.summary as Record<string, unknown> | undefined;
    const closedTabs = Number(closeSummary?.closedTabs ?? 0);
    const skippedTabs = Number(closeSummary?.skippedTabs ?? 0);
    const output = {
      ok: true,
      action: "dedupe",
      data: {
        analysisId: data.analysisId || null,
        summary: {
          candidates: candidates.length,
          planned: planTabIds.length,
          closed: Number.isFinite(closedTabs) ? closedTabs : 0,
          skipped: Number.isFinite(skippedTabs) ? skippedTabs : 0,
        },
        plan: {
          tabIds: planTabIds,
          candidates: planned,
        },
        close: closeData,
        nextCommand: options.confirm === true
          ? null
          : (planTabIds.length > 0 && data.analysisId ? `tabctl close --apply ${data.analysisId} --confirm` : null),
        policy: data.policy,
        policyInfo: data.policyInfo,
      },
    };
    printJson(output, prettyOutput);
    return;
  }

  if (command === "report") {
    const format = (options.format as string) || "json";
    const data = response.data as { entries?: Array<Record<string, unknown>>; generatedAt?: number } | undefined;
    const entries = data?.entries || [];
    const generatedAt = data?.generatedAt;
    const page = data && "page" in data ? (data.page as Record<string, unknown> | undefined) : undefined;
    let content = "";

    if (format === "json") {
      content = JSON.stringify({ generatedAt, entries }, null, 2);
    } else if (format === "csv") {
      content = renderCsv(entries);
    } else if (format === "md") {
      content = renderMarkdown(entries, generatedAt);
    } else {
      errorOut(`Unknown report format: ${format}`);
    }

    if (options.out) {
      fs.writeFileSync(String(options.out), content, "utf8");
      printJson({ ok: true, data: { writtenTo: options.out, format, count: entries.length, ...(page ? { page } : {}) } }, prettyOutput);
      return;
    }

    if (format === "json") {
      printJson({ ok: true, data: { format, entries, ...(page ? { page } : {}) } }, prettyOutput);
      return;
    }

    printJson({ ok: true, data: { format, entries, content, ...(page ? { page } : {}) } }, prettyOutput);
    return;
  }

  emitVersionWarnings(response, command);

  printJson(response, prettyOutput);
}

main().catch((error: Error) => {
  errorOut(error.message || "Unknown error");
});
