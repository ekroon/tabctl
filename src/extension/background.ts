const HOST_NAME = "com.erwinkroon.tabctl";
const manifest = chrome.runtime.getManifest();
const MANIFEST_VERSION = manifest.version || "0.0.0";
const MANIFEST_VERSION_NAME = manifest.version_name || MANIFEST_VERSION;

function parseVersionName(versionName: string) {
  const match = versionName.match(/-dev\.([0-9a-f]+)(\.dirty)?$/i);
  if (!match) {
    return { gitSha: null as string | null, dirty: false };
  }
  return { gitSha: match[1] || null, dirty: Boolean(match[2]) };
}

const parsed = parseVersionName(MANIFEST_VERSION_NAME);
const VERSION_INFO = {
  version: MANIFEST_VERSION_NAME,
  baseVersion: MANIFEST_VERSION,
  gitSha: parsed.gitSha,
  dirty: parsed.dirty,
};

const KEEPALIVE_ALARM = "tabctl-keepalive";
const KEEPALIVE_INTERVAL_MINUTES = 1;
const screenshot = require("./lib/screenshot") as typeof import("./lib/screenshot");
const content = require("./lib/content") as typeof import("./lib/content");
const { delay, executeWithTimeout, isScriptableUrl, isGitHubIssueOrPr, detectGitHubState, extractPageMeta, extractSelectorSignal, waitForTabLoad, waitForDomReady, waitForSettle, waitForTabReady, SETTLE_STABILITY_MS, SETTLE_POLL_INTERVAL_MS } = content;
const groups = require("./lib/groups") as typeof import("./lib/groups");
const tabs = require("./lib/tabs") as typeof import("./lib/tabs");
const { DESCRIPTION_MAX_LENGTH, getMostRecentFocusedWindowId, normalizeTabIndex } = tabs;
const undoHandlers = require("./lib/undo-handlers") as typeof import("./lib/undo-handlers");
type GroupDeps = import("./lib/groups").GroupDeps;
type GroupMatch = import("./lib/groups").GroupMatch;
type WindowSnapshot = import("./lib/groups").WindowSnapshot;
type TabDeps = import("./lib/tabs").TabDeps;

type AnyRecord = Record<string, any>;

const state = {
  port: null,
  archiveWindowId: null,
  archiveWindowIdLoaded: false,
  lastFocused: {},
  lastFocusedLoaded: false,
};

function log(...args: Array<unknown>) {
  console.log("[tabctl]", ...args);
}

function sendResponse(id: string, ok: boolean, payload: unknown) {
  if (!state.port) {
    return;
  }

  if (ok) {
    const data = typeof payload === "object" && payload !== null
      ? { ...(payload as Record<string, unknown>), component: "extension", version: VERSION_INFO.version, baseVersion: VERSION_INFO.baseVersion }
      : { payload, component: "extension", version: VERSION_INFO.version, baseVersion: VERSION_INFO.baseVersion };
    state.port.postMessage({ id, ok: true, data });
    return;
  }

  const error = payload instanceof Error
    ? { message: payload.message, stack: payload.stack }
    : payload;
  state.port.postMessage({ id, ok: false, error });
}

function connectNative() {
  if (state.port) {
    return;
  }

  try {
    const port = chrome.runtime.connectNative(HOST_NAME);
    state.port = port;
    port.onMessage.addListener(handleNativeMessage);
    port.onDisconnect.addListener(() => {
      const lastError = chrome.runtime.lastError;
      if (lastError) {
        log("Native host disconnected:", lastError.message);
      } else {
        log("Native host disconnected");
      }
      state.port = null;
    });
    log("Native host connected");
  } catch (error) {
    log("Native host connection failed", error);
  }
}

chrome.runtime.onInstalled.addListener(() => {
  connectNative();
  chrome.alarms.create(KEEPALIVE_ALARM, { periodInMinutes: KEEPALIVE_INTERVAL_MINUTES });
});

chrome.runtime.onStartup.addListener(() => {
  connectNative();
  chrome.alarms.create(KEEPALIVE_ALARM, { periodInMinutes: KEEPALIVE_INTERVAL_MINUTES });
});

chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === KEEPALIVE_ALARM) {
    connectNative();
  }
});

async function ensureLastFocusedLoaded() {
  if (state.lastFocusedLoaded) {
    return;
  }
  const stored = await chrome.storage.local.get("lastFocused");
  state.lastFocused = stored.lastFocused || {};
  state.lastFocusedLoaded = true;
}

async function ensureArchiveWindowIdLoaded() {
  if (state.archiveWindowIdLoaded) {
    return;
  }
  const stored = await chrome.storage.local.get("archiveWindowId");
  state.archiveWindowId = stored.archiveWindowId || null;
  state.archiveWindowIdLoaded = true;
}

async function setLastFocused(tabId: number) {
  await ensureLastFocusedLoaded();
  state.lastFocused[String(tabId)] = Date.now();
  await chrome.storage.local.set({ lastFocused: state.lastFocused });
}

chrome.tabs.onActivated.addListener((info) => {
  setLastFocused(info.tabId).catch((error) => log("Failed to set last focused", error));
});

chrome.tabs.onRemoved.addListener((tabId) => {
  ensureLastFocusedLoaded().then(() => {
    const key = String(tabId);
    if (state.lastFocused[key]) {
      delete state.lastFocused[key];
      chrome.storage.local.set({ lastFocused: state.lastFocused });
    }
  }).catch((error) => log("Failed to prune last focused", error));
});

chrome.windows.onFocusChanged.addListener((windowId) => {
  if (windowId === chrome.windows.WINDOW_ID_NONE) {
    return;
  }
  chrome.tabs.query({ windowId, active: true }).then((tabs) => {
    if (tabs[0] && tabs[0].id != null) {
      setLastFocused(tabs[0].id).catch((error) => log("Failed to set last focused", error));
    }
  }).catch((error) => log("Failed to query active tab", error));
});

async function handleNativeMessage(message: { id?: string; action?: string; params?: Record<string, unknown> }) {
  if (!message || typeof message !== "object") {
    return;
  }

  const { id, action, params } = message;
  if (!id || !action) {
    return;
  }

  try {
    const data = await handleAction(action, params || {}, id);
    sendResponse(id, true, data);
  } catch (error) {
    sendResponse(id, false, error);
  }
}

function sendProgress(id: string, payload: Record<string, unknown>) {
  if (!state.port) {
    return;
  }
  state.port.postMessage({ id, progress: true, data: payload });
}

async function handleAction(action: string, params: Record<string, unknown>, requestId: string) {
  switch (action) {
    case "ping":
      return {
        now: Date.now(),
        version: VERSION_INFO.version,
        baseVersion: VERSION_INFO.baseVersion,
        gitSha: VERSION_INFO.gitSha,
        dirty: VERSION_INFO.dirty,
        component: "extension",
      };
    case "version":
      return {
        version: VERSION_INFO.version,
        baseVersion: VERSION_INFO.baseVersion,
        gitSha: VERSION_INFO.gitSha,
        dirty: VERSION_INFO.dirty,
        component: "extension",
      };
    case "list":
      return await getTabSnapshot();
    case "analyze":
      return await tabs.analyzeTabs(params, requestId, tabDeps);
    case "inspect":
      return await tabs.inspectTabs(params, requestId, tabDeps);
    case "focus":
      return await tabs.focusTab(params);
    case "refresh":
      return await tabs.refreshTabs(params);
    case "open":
      return await tabs.openTabs(params, tabDeps);
    case "group-list":
      return await listGroups(params);
    case "group-update":
      return await groupUpdate(params);
    case "group-ungroup":
      return await groupUngroup(params);
    case "group-assign":
      return await groupAssign(params);
    case "move-tab":
      return await tabs.moveTab(params, tabDeps);
    case "move-group":
      return await tabs.moveGroup(params, tabDeps);
    case "merge-window":
      return await mergeWindow(params);
    case "archive":
      return await archiveTabs(params);
    case "close":
      return await closeTabs(params);
    case "report":
      return await reportTabs(params);
    case "screenshot":
      return await screenshot.screenshotTabs(params, requestId, {
        delay,
        executeWithTimeout,
        isScriptableUrl,
        getTabSnapshot,
        selectTabsByScope,
        waitForTabReady,
        sendProgress,
      });
    case "undo":
      return await undoHandlers.undoTransaction(params, { log });
    default:
      throw new Error(`Unknown action: ${action}`);
  }
}

function resolveWindowIdFromParams(snapshot: { windows: Array<Record<string, unknown>> }, value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) {
    return value;
  }
  if (typeof value === "string") {
    const normalized = value.trim().toLowerCase();
    if (normalized === "active") {
      const focused = (snapshot.windows as WindowSnapshot[]).find((win) => win.focused);
      return focused ? focused.windowId : null;
    }
    if (normalized === "last-focused") {
      return getMostRecentFocusedWindowId(snapshot.windows as WindowSnapshot[]);
    }
    const parsed = Number(normalized);
    return Number.isFinite(parsed) ? parsed : null;
  }
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

async function getTabSnapshot() {
  await ensureLastFocusedLoaded();
  const windows = await chrome.windows.getAll({ populate: true, windowTypes: ["normal"] });
  const groups = await chrome.tabGroups.query({});
  const groupById = new Map(groups.map((group) => [group.id, group]));

  const snapshot = windows.map((win) => {
    const tabs = (win.tabs || []).map((tab) => {
      const group = tab.groupId !== -1 ? groupById.get(tab.groupId) : null;
      return {
        tabId: tab.id,
        windowId: win.id,
        index: tab.index,
        url: tab.url,
        title: tab.title,
        active: tab.active,
        pinned: tab.pinned,
        groupId: tab.groupId,
        groupTitle: group ? group.title : null,
        groupColor: group ? group.color : null,
        groupCollapsed: group ? group.collapsed : null,
        lastFocusedAt: state.lastFocused[String(tab.id)] || null,
      };
    });

    const windowGroups = groups
      .filter((group) => group.windowId === win.id)
      .map((group) => ({
        groupId: group.id,
        title: group.title,
        color: group.color,
        collapsed: group.collapsed,
      }));

    return {
      windowId: win.id,
      focused: win.focused,
      state: win.state,
      tabs,
      groups: windowGroups,
    };
  });

  return {
    generatedAt: Date.now(),
    windows: snapshot,
  };
}

function flattenTabs(snapshot: { windows: Array<{ tabs: Array<Record<string, unknown>> }> }) {
  const result: Array<Record<string, unknown>> = [];
  for (const win of snapshot.windows) {
    for (const tab of win.tabs) {
      result.push(tab);
    }
  }
  return result;
}

function resolveGroupByTitle(snapshot: { windows: Array<Record<string, unknown>> }, groupTitle: string, windowId?: number) {
  return groups.resolveGroupByTitle(snapshot, buildWindowLabels, groupTitle, windowId);
}

function resolveGroupById(snapshot: { windows: Array<Record<string, unknown>> }, groupId: number) {
  return groups.resolveGroupById(snapshot, buildWindowLabels, groupId);
}

async function mergeWindow(params: Record<string, unknown>) {
  const fromWindowId = Number.isFinite(params.fromWindowId as number)
    ? Number(params.fromWindowId)
    : Number(params.windowId);
  const toWindowId = Number.isFinite(params.toWindowId as number) ? Number(params.toWindowId) : null;
  if (!Number.isFinite(fromWindowId) || !Number.isFinite(toWindowId)) {
    throw new Error("Missing source or target window id");
  }
  if (fromWindowId === toWindowId) {
    throw new Error("Source and target windows must differ");
  }

  const snapshot = await getTabSnapshot();
  const windows = snapshot.windows as WindowSnapshot[];
  const sourceWindow = windows.find((win) => win.windowId === fromWindowId);
  if (!sourceWindow) {
    throw new Error("Source window not found");
  }
  const targetWindow = windows.find((win) => win.windowId === toWindowId);
  if (!targetWindow) {
    throw new Error("Target window not found");
  }

  const rawTabIds = Array.isArray(params.tabIds) ? params.tabIds.map(Number) : [];
  const tabIdSet = new Set(rawTabIds.filter((id) => Number.isFinite(id)) as number[]);
  const skipped: Array<Record<string, unknown>> = [];
  let selectedTabs = sourceWindow.tabs;

  if (tabIdSet.size > 0) {
    const sourceTabIds = new Set(
      sourceWindow.tabs.map((tab) => tab.tabId).filter((id) => typeof id === "number") as number[],
    );
    for (const tabId of tabIdSet) {
      if (!sourceTabIds.has(tabId)) {
        skipped.push({ tabId, reason: "not_in_source" });
      }
    }
    selectedTabs = sourceWindow.tabs.filter((tab) => tabIdSet.has(tab.tabId as number));
  }

  if (selectedTabs.length === 0) {
    return {
      fromWindowId,
      toWindowId,
      summary: { movedTabs: 0, movedGroups: 0, skippedTabs: skipped.length, closedSource: false },
      skipped,
      groups: [],
      undo: {
        action: "merge-window",
        fromWindowId,
        toWindowId,
        closedSource: false,
        tabs: [],
      },
    };
  }

  const orderedTabs = [...selectedTabs].sort((a, b) => {
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
    return aIndex - bIndex;
  });

  const groupById = new Map<number, Record<string, unknown>>();
  for (const group of sourceWindow.groups) {
    groupById.set(group.groupId as number, group);
  }

  const plans: Array<{ groupId: number | null; tabs: Array<Record<string, unknown>> }> = [];
  let currentPlan: { groupId: number | null; tabs: Array<Record<string, unknown>> } | null = null;
  for (const tab of orderedTabs) {
    const rawGroupId = tab.groupId as number;
    const groupId = typeof rawGroupId === "number" && rawGroupId !== -1 ? rawGroupId : null;
    if (!currentPlan || currentPlan.groupId !== groupId) {
      currentPlan = { groupId, tabs: [] };
      plans.push(currentPlan);
    }
    currentPlan.tabs.push(tab);
  }

  let movedTabs = 0;
  let movedGroups = 0;
  const groups: Array<Record<string, unknown>> = [];
  const undoTabs: Array<Record<string, unknown>> = [];

  for (const plan of plans) {
    const tabIds = plan.tabs.map((tab) => tab.tabId).filter((id) => typeof id === "number") as number[];
    if (!tabIds.length) {
      continue;
    }

    let moved: chrome.tabs.Tab[] | chrome.tabs.Tab;
    try {
      moved = await chrome.tabs.move(tabIds, { windowId: toWindowId, index: -1 });
    } catch (error) {
      for (const tabId of tabIds) {
        skipped.push({ tabId, reason: "move_failed" });
      }
      log("Failed to move tabs", error);
      continue;
    }

    const movedList = Array.isArray(moved) ? moved : [moved];
    const movedIds = movedList.map((tab) => tab.id as number).filter((id) => typeof id === "number");
    movedTabs += movedIds.length;

    for (const entry of plan.tabs) {
      if (typeof entry.tabId !== "number") {
        continue;
      }
      const meta = groupById.get(entry.groupId as number);
      undoTabs.push({
        tabId: entry.tabId,
        windowId: entry.windowId,
        index: entry.index,
        groupId: entry.groupId,
        groupTitle: entry.groupTitle,
        groupColor: entry.groupColor,
        groupCollapsed: meta ? meta.collapsed : null,
      });
    }

    if (plan.groupId != null && movedIds.length > 0) {
      movedGroups += 1;
      let newGroupId: number | null = null;
      try {
        newGroupId = await chrome.tabs.group({ tabIds: movedIds, createProperties: { windowId: toWindowId } });
        const meta = groupById.get(plan.groupId);
        if (meta) {
          await chrome.tabGroups.update(newGroupId, {
            title: (meta.title as string) || "",
            color: (meta.color as chrome.tabGroups.ColorEnum) || "grey",
            collapsed: (meta.collapsed as boolean | undefined) || false,
          });
        }
      } catch (error) {
        log("Failed to regroup tabs", error);
      }
      groups.push({ sourceGroupId: plan.groupId, newGroupId });
    }
  }

  let closedSource = false;
  if (params.closeSource === true) {
    try {
      const remainingTabs = await chrome.tabs.query({ windowId: fromWindowId });
      if (remainingTabs.length === 0) {
        await chrome.windows.remove(fromWindowId);
        closedSource = true;
      }
    } catch (error) {
      log("Failed to close source window", error);
    }
  }

  return {
    fromWindowId,
    toWindowId,
    summary: { movedTabs, movedGroups, skippedTabs: skipped.length, closedSource },
    skipped,
    groups,
    undo: {
      action: "merge-window",
      fromWindowId,
      toWindowId,
      closedSource,
      tabs: undoTabs,
    },
    txid: params.txid || null,
  };
}

const groupDeps: GroupDeps = {
  getTabSnapshot,
  buildWindowLabels,
  resolveWindowIdFromParams,
  log,
};

const tabDeps: TabDeps = {
  getTabSnapshot,
  selectTabsByScope,
  sendProgress,
  log,
  resolveWindowIdFromParams,
  resolveGroupByTitle,
  resolveGroupById,
};

async function listGroups(params: Record<string, unknown>) {
  return groups.listGroups(params, groupDeps);
}

async function groupUpdate(params: Record<string, unknown>) {
  return groups.groupUpdate(params, groupDeps);
}

async function groupUngroup(params: Record<string, unknown>) {
  return groups.groupUngroup(params, groupDeps);
}

async function groupAssign(params: Record<string, unknown>) {
  return groups.groupAssign(params, groupDeps);
}


async function ensureArchiveWindow() {
  await ensureArchiveWindowIdLoaded();
  if (state.archiveWindowId) {
    try {
      await chrome.windows.get(state.archiveWindowId);
      return state.archiveWindowId;
    } catch {
      state.archiveWindowId = null;
    }
  }

  const created = await chrome.windows.create({ focused: false });
  state.archiveWindowId = created.id;
  await chrome.storage.local.set({ archiveWindowId: created.id });
  return created.id;
}

function buildWindowLabels(snapshot: { windows: Array<{ windowId: number }> }) {
  const labels = new Map<number, string>();
  snapshot.windows.forEach((win, index) => {
    labels.set(win.windowId, `W${index + 1}`);
  });
  return labels;
}

function selectTabsByScope(snapshot: { windows: Array<Record<string, unknown>> }, params: Record<string, unknown>) {
  const allTabs = flattenTabs(snapshot as { windows: Array<{ tabs: Array<Record<string, unknown>> }> });

  if (params.tabIds && (params.tabIds as Array<number>).length) {
    const idSet = new Set((params.tabIds as Array<number>).map(Number));
    return { tabs: allTabs.filter((tab) => idSet.has(tab.tabId as number)) };
  }

  if (params.groupId) {
    const groupId = Number(params.groupId);
    return { tabs: allTabs.filter((tab) => tab.groupId === groupId) };
  }

  if (params.groupTitle) {
    const windowId = params.windowId != null ? resolveWindowIdFromParams(snapshot, params.windowId) ?? undefined : undefined;
    const resolved = resolveGroupByTitle(snapshot, params.groupTitle as string, windowId);
    if ((resolved as { error?: Record<string, unknown> }).error) {
      return { tabs: [], error: (resolved as { error: Record<string, unknown> }).error };
    }
    const match = (resolved as { match: GroupMatch }).match;
    return {
      tabs: allTabs.filter((tab) => tab.groupId === match.group.groupId && tab.windowId === match.windowId),
    };
  }

  if (params.windowId) {
    const windowId = resolveWindowIdFromParams(snapshot, params.windowId);
    if (!Number.isFinite(windowId)) {
      return { tabs: [] };
    }
    return { tabs: allTabs.filter((tab) => tab.windowId === windowId) };
  }

  if (params.all) {
    return { tabs: allTabs };
  }

  const focusedWindow = (snapshot.windows as Array<{ focused: boolean; tabs: Array<Record<string, unknown>> }>).find((win) => win.focused);
  if (!focusedWindow) {
    return { tabs: [] };
  }
  return { tabs: focusedWindow.tabs };
}

async function archiveTabs(params: Record<string, unknown>) {
  const snapshot = await getTabSnapshot();
  const windowLabels = buildWindowLabels(snapshot as { windows: Array<{ windowId: number }> });

  let windowsToProcess = snapshot.windows as Array<{ windowId: number; focused: boolean; state: string; tabs: Array<Record<string, unknown>>; groups: Array<Record<string, unknown>> }>;
  if (params.windowId) {
    const resolvedWindowId = resolveWindowIdFromParams(snapshot, params.windowId);
    windowsToProcess = resolvedWindowId != null
      ? windowsToProcess.filter((win) => win.windowId === resolvedWindowId)
      : [];
  } else if (!params.all) {
    const focused = windowsToProcess.find((win) => win.focused);
    windowsToProcess = focused ? [focused] : [];
  }

  if (params.groupTitle || params.groupId || params.tabIds) {
    const selected = selectTabsByScope(snapshot, params);
    if ((selected as { error?: Record<string, unknown> }).error) {
      throw (selected as { error: Record<string, unknown> }).error;
    }
    windowsToProcess = (snapshot.windows as Array<{ windowId: number; focused: boolean; state: string; tabs: Array<Record<string, unknown>>; groups: Array<Record<string, unknown>> }>)
      .map((win) => ({
        windowId: win.windowId,
        focused: win.focused,
        state: win.state,
        tabs: win.tabs.filter((tab) => (selected as { tabs: Array<Record<string, unknown>> }).tabs.some((sel) => sel.tabId === tab.tabId)),
        groups: win.groups,
      }))
      .filter((win) => win.tabs.length > 0);
  }

  if (windowsToProcess.length === 0) {
    return {
      txid: params.txid || null,
      summary: { movedTabs: 0, movedGroups: 0, skippedTabs: 0 },
      archiveWindowId: null,
      skipped: [],
      undo: { action: "archive", tabs: [] },
    };
  }

  const archiveWindowId = await ensureArchiveWindow();
  const undoTabs: Array<Record<string, unknown>> = [];
  const skipped: Array<Record<string, unknown>> = [];
  let movedGroups = 0;
  let movedTabs = 0;

  for (const window of windowsToProcess) {
    const groupsById = new Map<number, Record<string, unknown>>();
    for (const group of window.groups) {
      groupsById.set(group.groupId as number, group);
    }

    const groupedTabs = new Map<number, Array<Record<string, unknown>>>();
    const ungroupedTabs: Array<Record<string, unknown>> = [];

    for (const tab of window.tabs) {
      if (tab.tabId == null) {
        continue;
      }
      if (tab.groupId === -1) {
        ungroupedTabs.push(tab);
      } else {
        if (!groupedTabs.has(tab.groupId as number)) {
          groupedTabs.set(tab.groupId as number, []);
        }
        groupedTabs.get(tab.groupId as number)?.push(tab);
      }
    }

    const windowLabel = windowLabels.get(window.windowId) || `W${window.windowId}`;

    const plans: Array<Record<string, unknown>> = [];
    for (const [groupId, tabs] of groupedTabs.entries()) {
      const group = groupsById.get(groupId) || null;
      plans.push({
        windowId: window.windowId,
        windowLabel,
        group,
        tabs,
        isUngrouped: false,
      });
    }
    if (ungroupedTabs.length > 0) {
      plans.push({
        windowId: window.windowId,
        windowLabel,
        group: null,
        tabs: ungroupedTabs,
        isUngrouped: true,
      });
    }

    for (const plan of plans) {
      const tabIds = (plan.tabs as Array<Record<string, unknown>>).map((tab) => tab.tabId as number);
      if (tabIds.length === 0) {
        continue;
      }

      const tabById = new Map<number, Record<string, unknown>>();
      for (const tab of plan.tabs as Array<Record<string, unknown>>) {
        tabById.set(tab.tabId as number, tab);
      }

      let moved: chrome.tabs.Tab[] | chrome.tabs.Tab;
      try {
        moved = await chrome.tabs.move(tabIds, { windowId: archiveWindowId, index: -1 });
      } catch (error) {
        for (const tabId of tabIds) {
          skipped.push({ tabId, reason: "move_failed" });
        }
        log("Failed to move tabs", error);
        continue;
      }

      const movedList = Array.isArray(moved) ? moved : [moved];
      const movedIds = movedList.map((tab) => tab.id as number);
      movedTabs += movedIds.length;

      for (const movedId of movedIds) {
        const tab = tabById.get(movedId);
        if (!tab) {
          continue;
        }
        undoTabs.push({
          tabId: tab.tabId,
          url: tab.url,
          title: tab.title,
          pinned: tab.pinned,
          active: tab.active,
          from: {
            windowId: tab.windowId,
            index: tab.index,
            groupId: tab.groupId,
            groupTitle: plan.group ? (plan.group as Record<string, unknown>).title : null,
            groupColor: plan.group ? (plan.group as Record<string, unknown>).color : null,
            groupCollapsed: plan.group ? (plan.group as Record<string, unknown>).collapsed : null,
          },
        });
      }

      const titleBase = plan.group && (plan.group as Record<string, unknown>).title
        ? (plan.group as Record<string, unknown>).title
        : plan.isUngrouped
          ? "Ungrouped"
          : "Group";
      const archiveTitle = `${plan.windowLabel} - ${titleBase}`;
      const groupColor = plan.group && (plan.group as Record<string, unknown>).color
        ? (plan.group as Record<string, unknown>).color
        : "grey";

      try {
        const newGroupId = await chrome.tabs.group({ tabIds: movedIds, createProperties: { windowId: archiveWindowId } });
        await chrome.tabGroups.update(newGroupId, { title: archiveTitle, color: groupColor as chrome.tabGroups.ColorEnum });
        movedGroups += 1;
      } catch (error) {
        log("Failed to group archived tabs", error);
      }
    }
  }

  return {
    txid: params.txid || null,
    summary: {
      movedTabs,
      movedGroups,
      skippedTabs: skipped.length,
    },
    archiveWindowId,
    skipped,
    undo: {
      action: "archive",
      tabs: undoTabs,
    },
  };
}

async function getTabsByIds(tabIds: Array<number>) {
  const results: Array<chrome.tabs.Tab | null> = [];
  for (const tabId of tabIds) {
    try {
      const tab = await chrome.tabs.get(tabId);
      results.push(tab);
    } catch {
      results.push(null);
    }
  }
  return results;
}

async function closeTabs(params: Record<string, unknown>) {
  const mode = params.mode || "direct";
  if (mode === "direct" && !params.confirmed) {
    throw new Error("Direct close requires confirmation");
  }

  let tabIds = (params.tabIds as Array<number>) || [];
  if (!tabIds.length && (params.groupTitle || params.groupId || params.windowId)) {
    const snapshot = await getTabSnapshot();
    const selection = selectTabsByScope(snapshot, params) as { tabs: Array<Record<string, unknown>>; error?: Record<string, unknown> };
    if (selection.error) {
      throw selection.error;
    }
    tabIds = selection.tabs.map((tab) => tab.tabId as number);
  }

  if (!tabIds.length) {
    return {
      txid: params.txid || null,
      summary: { closedTabs: 0, skippedTabs: 0 },
      skipped: [],
      undo: { action: "close", tabs: [] },
    };
  }

  const expectedUrls = (params.expectedUrls as Record<string, string>) || {};
  const tabInfos = await getTabsByIds(tabIds);
  const validTabs: Array<Record<string, unknown>> = [];
  const skipped: Array<Record<string, unknown>> = [];

  const groups = await chrome.tabGroups.query({});
  const groupById = new Map(groups.map((group) => [group.id, group]));

  for (let i = 0; i < tabIds.length; i += 1) {
    const tabId = tabIds[i];
    const tab = tabInfos[i];
    if (!tab) {
      skipped.push({ tabId, reason: "not_found" });
      continue;
    }

    const expected = expectedUrls[String(tabId)];
    if (expected && tab.url !== expected) {
      skipped.push({ tabId, reason: "url_mismatch" });
      continue;
    }

    const group = tab.groupId !== -1 ? groupById.get(tab.groupId) : null;
    validTabs.push({
      tabId,
      url: tab.url,
      title: tab.title,
      pinned: tab.pinned,
      active: tab.active,
      from: {
        windowId: tab.windowId,
        index: tab.index,
        groupId: tab.groupId,
        groupTitle: group ? group.title : null,
        groupColor: group ? group.color : null,
        groupCollapsed: group ? group.collapsed : null,
      },
    });
  }

  if (validTabs.length > 0) {
    await chrome.tabs.remove(validTabs.map((tab) => tab.tabId as number));
  }

  return {
    txid: params.txid || null,
    summary: {
      closedTabs: validTabs.length,
      skippedTabs: skipped.length,
    },
    skipped,
    undo: {
      action: "close",
      tabs: validTabs.map((tab) => ({
        url: tab.url,
        title: tab.title,
        pinned: tab.pinned,
        active: tab.active,
        from: tab.from,
      })),
    },
  };
}

async function extractDescription(tabId: number) {
  try {
    const [{ result }] = await chrome.scripting.executeScript({
      target: { tabId },
      func: () => {
        const pickContent = (selector: string) => {
          const el = document.querySelector(selector);
          if (!el) {
            return "";
          }
          const content = el.getAttribute("content") || el.textContent || "";
          return content.trim();
        };

        let description =
          pickContent("meta[name='description']") ||
          pickContent("meta[property='og:description']") ||
          pickContent("meta[name='twitter:description']");

        if (!description) {
          const h1 = document.querySelector("h1");
          const h1Text = h1 ? h1.textContent?.trim() : "";
          const paragraphs = Array.from(document.querySelectorAll("p"))
            .map((p) => p.textContent?.replace(/\s+/g, " ").trim() || "")
            .filter((text) => text.length > 40);
          const pText = paragraphs.length ? paragraphs[0] : "";
          if (h1Text && pText) {
            description = `${h1Text} - ${pText}`;
          } else {
            description = h1Text || pText;
          }
        }

        return description.replace(/\s+/g, " ").trim();
      },
    });

    if (!result) {
      return "";
    }

    return String(result).slice(0, DESCRIPTION_MAX_LENGTH);
  } catch {
    return "";
  }
}

async function reportTabs(params: Record<string, unknown>) {
  const snapshot = await getTabSnapshot();
  const selection = selectTabsByScope(snapshot, params) as { tabs: Array<Record<string, unknown>>; error?: Record<string, unknown> };
  if (selection.error) {
    throw selection.error;
  }

  const windowLabels = buildWindowLabels(snapshot as { windows: Array<{ windowId: number }> });
  const tabs = selection.tabs;

  const entries: Array<Record<string, unknown>> = [];
  for (const tab of tabs) {
    const description = isScriptableUrl(tab.url) ? await extractDescription(tab.tabId as number) : "";
    entries.push({
      tabId: tab.tabId,
      windowId: tab.windowId,
      windowLabel: windowLabels.get(tab.windowId as number) || `W${tab.windowId}`,
      groupId: tab.groupId,
      groupTitle: tab.groupTitle,
      groupColor: tab.groupColor,
      url: tab.url,
      title: tab.title,
      description,
      lastFocusedAt: tab.lastFocusedAt,
    });
  }

  return {
    generatedAt: Date.now(),
    entries,
    totals: {
      tabs: entries.length,
    },
  };
}
