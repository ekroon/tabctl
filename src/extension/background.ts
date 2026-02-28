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
const { delay, executeWithTimeout, isScriptableUrl, extractPageMeta, extractSelectorSignal, waitForTabLoad, waitForDomReady, waitForSettle, waitForTabReady, SETTLE_STABILITY_MS, SETTLE_POLL_INTERVAL_MS } = content;
const groups = require("./lib/groups") as typeof import("./lib/groups");
const tabs = require("./lib/tabs") as typeof import("./lib/tabs");
const { getMostRecentFocusedWindowId, normalizeTabIndex } = tabs;
const move = require("./lib/move") as typeof import("./lib/move");
const inspect = require("./lib/inspect") as typeof import("./lib/inspect");
const { DESCRIPTION_MAX_LENGTH } = inspect;
const undoHandlers = require("./lib/undo-handlers") as typeof import("./lib/undo-handlers");
const archive = require("./lib/archive") as typeof import("./lib/archive");
type ExtensionDeps = import("./lib/deps").ExtensionDeps;
type GroupMatch = import("./lib/groups").GroupMatch;
type WindowSnapshot = import("./lib/groups").WindowSnapshot;

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
      ? payload
      : { payload };
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
        runtimeId: chrome.runtime.id,
        version: VERSION_INFO.version,
        baseVersion: VERSION_INFO.baseVersion,
        gitSha: VERSION_INFO.gitSha,
        dirty: VERSION_INFO.dirty,
        component: "extension",
      };
    case "version":
      return {
        runtimeId: chrome.runtime.id,
        version: VERSION_INFO.version,
        baseVersion: VERSION_INFO.baseVersion,
        gitSha: VERSION_INFO.gitSha,
        dirty: VERSION_INFO.dirty,
        component: "extension",
      };
    case "list":
      return shapeListSnapshot(await getTabSnapshot());
    case "analyze":
      return await inspect.analyzeTabs(params, requestId, deps);
    case "inspect":
      return await inspect.inspectTabs(params, requestId, deps);
    case "focus":
      return await tabs.focusTab(params);
    case "refresh":
      return await tabs.refreshTabs(params);
    case "open":
      return await tabs.openTabs(params, deps);
    case "group-list":
      return await listGroups(params);
    case "group-update":
      return await groupUpdate(params);
    case "group-ungroup":
      return await groupUngroup(params);
    case "group-assign":
      return await groupAssign(params);
    case "group-gather":
      return await groupGather(params);
    case "move-tab":
      return await move.moveTab(params, deps);
    case "move-group":
      return await move.moveGroup(params, deps);
    case "merge-window":
      return await archive.mergeWindow(params, deps);
    case "archive":
      return await archive.archiveTabs(params, deps);
    case "close":
      return await archive.closeTabs(params, deps);
    case "report":
      return await reportTabs(params);
    case "screenshot":
      return await screenshot.screenshotTabs(params, requestId, deps);
    case "undo":
      return await undoHandlers.undoTransaction(params, deps);
    case "reload":
      // Defer reload to allow the response to be sent first
      setTimeout(() => chrome.runtime.reload(), 100);
      return { reloading: true };

    // --- Primitives: thin Chrome API wrappers (p: prefix) ---

    case "p:snapshot":
      return await getTabSnapshot();

    case "p:tab-get":
      return await chrome.tabs.get(Number(params.tabId));

    case "p:tab-query":
      return await chrome.tabs.query(params.query as chrome.tabs.QueryInfo);

    case "p:tab-create":
      return await chrome.tabs.create(params as chrome.tabs.CreateProperties);

    case "p:tab-update": {
      const { tabId: rawTabId, ...updateProps } = params;
      return await chrome.tabs.update(Number(rawTabId), updateProps as chrome.tabs.UpdateProperties);
    }

    case "p:tab-move": {
      const { tabIds: moveTabIds, ...moveProps } = params;
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      return await (chrome.tabs.move as any)(moveTabIds, moveProps);
    }

    case "p:tab-remove":
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      await (chrome.tabs.remove as any)(params.tabIds);
      return { removed: true };

    case "p:tab-reload":
      await chrome.tabs.reload(Number(params.tabId));
      return { reloaded: true };

    case "p:tab-group":
      return { groupId: await chrome.tabs.group(params as chrome.tabs.GroupOptions) };

    case "p:tab-ungroup":
      await chrome.tabs.ungroup(params.tabIds as number[]);
      return { ungrouped: true };

    case "p:group-update": {
      const { groupId: rawGroupId, ...groupProps } = params;
      return await chrome.tabGroups.update(Number(rawGroupId), groupProps as chrome.tabGroups.UpdateProperties);
    }

    case "p:window-create":
      return await chrome.windows.create(params as chrome.windows.CreateData);

    case "p:window-remove":
      await chrome.windows.remove(Number(params.windowId));
      return { removed: true };

    case "p:window-update": {
      const { windowId: rawWinId, ...winProps } = params;
      return await chrome.windows.update(Number(rawWinId), winProps as chrome.windows.UpdateInfo);
    }

    case "p:execute-script": {
      const targetTabId = Number(params.tabId);
      const funcName = params.func as string;
      const funcArgs = (params.args || []) as Array<unknown>;
      const timeoutMs = Number(params.timeoutMs) || 8000;

      switch (funcName) {
        case "extractPageMeta":
          return await content.extractPageMeta(targetTabId, timeoutMs, (funcArgs[0] as number) || DESCRIPTION_MAX_LENGTH);
        case "extractSelectorSignal":
          return await content.extractSelectorSignal(targetTabId, funcArgs[0] as Array<Record<string, unknown>>, timeoutMs, (funcArgs[1] as number) || DESCRIPTION_MAX_LENGTH);
        default:
          throw new Error(`Unknown execute-script func: ${funcName}`);
      }
    }

    case "p:screenshot-tile":
      return await screenshot.captureTabTiles(
        params.tab as Record<string, unknown>,
        params.options as { mode: "viewport" | "full"; format: "png" | "jpeg"; quality: number; tileMaxDim: number; maxBytes: number },
        deps,
      );

    default:
      throw new Error(`Unknown action: ${action}`);
  }
}

function shapeListSnapshot(snapshot: { generatedAt: number; windows: Array<Record<string, unknown>> }) {
  return {
    windows: snapshot.windows.map((win) => ({
      windowId: win.windowId,
      focused: win.focused,
      tabs: ((win.tabs as Array<Record<string, unknown>>) || []).map((tab) => ({
        tabId: tab.tabId,
        windowId: tab.windowId,
        url: tab.url,
        title: tab.title,
        active: tab.active,
        groupId: tab.groupId,
        groupTitle: tab.groupTitle,
      })),
      groups: ((win.groups as Array<Record<string, unknown>>) || []).map((group) => ({
        groupId: group.groupId,
        title: group.title,
        color: group.color,
        collapsed: group.collapsed,
      })),
    })),
  };
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

const deps: ExtensionDeps = {
  getTabSnapshot,
  selectTabsByScope,
  sendProgress,
  log,
  resolveWindowIdFromParams,
  resolveGroupByTitle,
  resolveGroupById,
  buildWindowLabels,
  getArchiveWindowId,
  setArchiveWindowId,
  delay,
  executeWithTimeout,
  isScriptableUrl,
  waitForTabReady,
};

async function listGroups(params: Record<string, unknown>) {
  return groups.listGroups(params, deps);
}

async function groupUpdate(params: Record<string, unknown>) {
  return groups.groupUpdate(params, deps);
}

async function groupUngroup(params: Record<string, unknown>) {
  return groups.groupUngroup(params, deps);
}

async function groupAssign(params: Record<string, unknown>) {
  return groups.groupAssign(params, deps);
}

async function groupGather(params: Record<string, unknown>) {
  return groups.groupGather(params, deps);
}

async function getArchiveWindowId(): Promise<number | null> {
  await ensureArchiveWindowIdLoaded();
  return state.archiveWindowId;
}

async function setArchiveWindowId(id: number | null): Promise<void> {
  state.archiveWindowId = id;
  state.archiveWindowIdLoaded = true;
  await chrome.storage.local.set({ archiveWindowId: id });
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

// Expose for CDP Runtime.evaluate (esbuild IIFE scoping hides local bindings)
(self as any).__tabctl = { state, connectNative };
