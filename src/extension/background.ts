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
const { delay, executeWithTimeout } = content;
const DESCRIPTION_MAX_LENGTH = 250;

const state = {
  port: null,
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
        case "extractSelectorSignal": {
          const specs = funcArgs[0] as Array<Record<string, unknown>> | undefined;
          if (!Array.isArray(specs)) throw new Error("extractSelectorSignal requires specs array as first arg");
          const selectorMaxLen = (funcArgs[1] as number) || 500;
          return await content.extractSelectorSignal(targetTabId, specs, timeoutMs, selectorMaxLen);
        }
        default:
          throw new Error(`Unknown execute-script func: ${funcName}`);
      }
    }

    case "p:screenshot-tile":
      return await screenshot.captureTabTiles(
        params.tab as Record<string, unknown>,
        params.options as { mode: "viewport" | "full"; format: "png" | "jpeg"; quality: number; tileMaxDim: number; maxBytes: number },
        { delay, executeWithTimeout },
      );

    default:
      throw new Error(`Unknown action: ${action}`);
  }
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

// Expose for CDP Runtime.evaluate (esbuild IIFE scoping hides local bindings)
(self as any).__tabctl = { state, connectNative };
