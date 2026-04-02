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
const BROWSER_STATE_SYNC_DEBOUNCE_MS = 750;
const screenshot = require("./lib/screenshot") as typeof import("./lib/screenshot");
const content = require("./lib/content") as typeof import("./lib/content");
const { delay, executeWithTimeout } = content;
const DESCRIPTION_MAX_LENGTH = 250;

function requireFiniteId(value: unknown, name: string): number {
  const n = Number(value);
  if (!Number.isFinite(n)) throw new Error(`${name} must be a finite number, got: ${String(value)}`);
  return n;
}

const state = {
  port: null as chrome.runtime.Port | null,
};

const browserState = {
  nextId: 1,
  syncTimer: null as ReturnType<typeof setTimeout> | null,
  pendingEvents: [] as Array<Record<string, unknown>>,
  incognitoWindowIds: new Set<number>(),
  incognitoTabIds: new Set<number>(),
  incognitoGroupIds: new Set<number>(),
};

function log(...args: Array<unknown>) {
  console.log("[tabctl]", ...args);
}

function sendResponse(port: chrome.runtime.Port | null, id: string, ok: boolean, payload: unknown) {
  if (!port) {
    log("dropping response because native port is unavailable", { id, ok });
    return;
  }

  try {
    if (ok) {
      const data = typeof payload === "object" && payload !== null
        ? payload
        : { payload };
      port.postMessage({ id, ok: true, data });
      return;
    }

    const error = payload instanceof Error
      ? { message: payload.message, stack: payload.stack }
      : payload;
    port.postMessage({ id, ok: false, error });
  } catch (error) {
    log("failed to send native response", { id, ok, error });
  }
}

function connectNative() {
  if (state.port) {
    return;
  }

  try {
    const port = chrome.runtime.connectNative(HOST_NAME);
    state.port = port;
    port.onMessage.addListener((message) => {
      void handleNativeMessage(port, message);
    });
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
    queueBrowserStateSync("startup");
  } catch (error) {
    log("Native host connection failed", error);
  }
}

function nextBrowserStateId() {
  const id = browserState.nextId;
  browserState.nextId += 1;
  return `browser-state-${Date.now()}-${id}`;
}

function normalizeEventPayload(
  kind: string,
  payload: Record<string, unknown>,
): Record<string, unknown> {
  const event: Record<string, unknown> = {
    kind,
    occurredAt: Date.now(),
  };
  for (const [key, value] of Object.entries(payload)) {
    if (value === undefined) {
      continue;
    }
    event[key] = value;
  }
  if (event.incognito !== true && inferIncognitoEvent(payload)) {
    event.incognito = true;
  }
  return event;
}

function inferIncognitoEvent(payload: Record<string, unknown>): boolean {
  const tabId = typeof payload.tabId === "number" ? payload.tabId : null;
  if (tabId !== null && browserState.incognitoTabIds.has(tabId)) {
    return true;
  }
  const groupId = typeof payload.groupId === "number" ? payload.groupId : null;
  if (groupId !== null && browserState.incognitoGroupIds.has(groupId)) {
    return true;
  }
  const windowId = typeof payload.windowId === "number" ? payload.windowId : null;
  return windowId !== null && browserState.incognitoWindowIds.has(windowId);
}

function updateIncognitoState(
  snapshot: { windows?: Array<{ windowId?: number; incognito?: boolean; tabs?: Array<{ tabId?: number }>; groups?: Array<{ groupId?: number }> }> },
) {
  browserState.incognitoWindowIds.clear();
  browserState.incognitoTabIds.clear();
  browserState.incognitoGroupIds.clear();
  for (const window of snapshot.windows || []) {
    if (window.incognito !== true || typeof window.windowId !== "number") {
      continue;
    }
    browserState.incognitoWindowIds.add(window.windowId);
    for (const tab of window.tabs || []) {
      if (typeof tab.tabId === "number") {
        browserState.incognitoTabIds.add(tab.tabId);
      }
    }
    for (const group of window.groups || []) {
      if (typeof group.groupId === "number") {
        browserState.incognitoGroupIds.add(group.groupId);
      }
    }
  }
}

async function postBrowserStateSync(reason: string) {
  if (!state.port) {
    return;
  }

  const eventCount = browserState.pendingEvents.length;
  const events = browserState.pendingEvents.slice(0, eventCount);
  try {
    const snapshot = await getTabSnapshot();
    updateIncognitoState(snapshot);
    state.port.postMessage({
      id: nextBrowserStateId(),
      action: "browser-state-sync",
      ok: true,
      data: {
        reason,
        recordedAt: Date.now(),
        events,
        snapshot,
      },
    });
    browserState.pendingEvents.splice(0, eventCount);
  } catch (error) {
    log("Browser state sync failed", error);
  }
}

function queueBrowserStateSync(reason: string) {
  if (!state.port) {
    connectNative();
    // Let connectNative() schedule the immediate startup sync after reconnect.
    return;
  }
  if (browserState.syncTimer) {
    clearTimeout(browserState.syncTimer);
  }
  const delayMs = reason === "startup" ? 0 : BROWSER_STATE_SYNC_DEBOUNCE_MS;
  browserState.syncTimer = setTimeout(() => {
    browserState.syncTimer = null;
    void postBrowserStateSync(reason);
  }, delayMs);
}

function enqueueBrowserStateEvent(kind: string, payload: Record<string, unknown>, reason = "event") {
  browserState.pendingEvents.push(normalizeEventPayload(kind, payload));
  queueBrowserStateSync(reason);
}

function registerBrowserStateListeners() {
  chrome.tabs?.onCreated?.addListener((tab) => {
    enqueueBrowserStateEvent("tabs.onCreated", {
      tabId: tab.id,
      windowId: tab.windowId,
      groupId: tab.groupId,
      incognito: tab.incognito,
      url: tab.url,
      title: tab.title,
      index: tab.index,
    });
  });

  chrome.tabs?.onUpdated?.addListener((tabId, changeInfo, tab) => {
    const interesting = ["url", "title", "status", "pinned", "audible", "discarded", "favIconUrl"];
    if (!interesting.some((key) => key in changeInfo)) {
      return;
    }
    enqueueBrowserStateEvent("tabs.onUpdated", {
      tabId,
      windowId: tab.windowId,
      groupId: tab.groupId,
      incognito: tab.incognito,
      changeInfo,
    });
  });

  chrome.tabs?.onMoved?.addListener((tabId, moveInfo) => {
    enqueueBrowserStateEvent("tabs.onMoved", {
      tabId,
      windowId: moveInfo.windowId,
      fromIndex: moveInfo.fromIndex,
      toIndex: moveInfo.toIndex,
    });
  });

  chrome.tabs?.onAttached?.addListener((tabId, attachInfo) => {
    enqueueBrowserStateEvent("tabs.onAttached", {
      tabId,
      windowId: attachInfo.newWindowId,
      newPosition: attachInfo.newPosition,
    });
  });

  chrome.tabs?.onDetached?.addListener((tabId, detachInfo) => {
    enqueueBrowserStateEvent("tabs.onDetached", {
      tabId,
      windowId: detachInfo.oldWindowId,
      oldPosition: detachInfo.oldPosition,
    });
  });

  chrome.tabs?.onRemoved?.addListener((tabId, removeInfo) => {
    enqueueBrowserStateEvent("tabs.onRemoved", {
      tabId,
      windowId: removeInfo.windowId,
      isWindowClosing: removeInfo.isWindowClosing,
    });
  });

  chrome.tabs?.onActivated?.addListener((activeInfo) => {
    enqueueBrowserStateEvent("tabs.onActivated", {
      tabId: activeInfo.tabId,
      windowId: activeInfo.windowId,
    });
  });

  chrome.tabGroups?.onCreated?.addListener((group) => {
    enqueueBrowserStateEvent("tabGroups.onCreated", {
      groupId: group.id,
      windowId: group.windowId,
      title: group.title,
      color: group.color,
      collapsed: group.collapsed,
    });
  });

  chrome.tabGroups?.onUpdated?.addListener((group) => {
    enqueueBrowserStateEvent("tabGroups.onUpdated", {
      groupId: group.id,
      windowId: group.windowId,
      title: group.title,
      color: group.color,
      collapsed: group.collapsed,
    });
  });

  chrome.tabGroups?.onMoved?.addListener((group) => {
    enqueueBrowserStateEvent("tabGroups.onMoved", {
      groupId: group.id,
      windowId: group.windowId,
      title: group.title,
      color: group.color,
      collapsed: group.collapsed,
    });
  });

  chrome.tabGroups?.onRemoved?.addListener((group) => {
    enqueueBrowserStateEvent("tabGroups.onRemoved", {
      groupId: group.id,
      windowId: group.windowId,
      title: group.title,
      color: group.color,
      collapsed: group.collapsed,
    });
  });

  chrome.windows?.onCreated?.addListener((window) => {
    enqueueBrowserStateEvent("windows.onCreated", {
      windowId: window.id,
      incognito: window.incognito,
      focused: window.focused,
      state: window.state,
    });
  });

  chrome.windows?.onRemoved?.addListener((windowId) => {
    enqueueBrowserStateEvent("windows.onRemoved", { windowId });
  });

  chrome.windows?.onFocusChanged?.addListener((windowId) => {
    enqueueBrowserStateEvent("windows.onFocusChanged", { windowId });
  });
}

connectNative();
registerBrowserStateListeners();

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

async function handleNativeMessage(
  requestPort: chrome.runtime.Port,
  message: { id?: string; action?: string; params?: Record<string, unknown> },
) {
  if (!message || typeof message !== "object") {
    return;
  }

  const { id, action, params } = message;
  if (!id || !action) {
    return;
  }

  try {
    const data = await handleAction(action, params || {}, id);
    sendResponse(requestPort, id, true, data);
  } catch (error) {
    sendResponse(requestPort, id, false, error);
  }
}

function sendProgress(id: string, payload: Record<string, unknown>) {
  if (!state.port) {
    return;
  }
  state.port.postMessage({ id, progress: true, data: payload });
}

async function handleAction(action: string, params: Record<string, unknown>, requestId: string) {
  // Strip host-injected metadata from primitives so only Chrome-safe fields reach APIs
  if (action.startsWith("p:")) {
    delete params.client;
    delete params.txid;
  }

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
      return await chrome.tabs.get(requireFiniteId(params.tabId, "tabId"));

    case "p:tab-query":
      return await chrome.tabs.query(params.query as chrome.tabs.QueryInfo);

    case "p:tab-create":
      return await chrome.tabs.create(params as chrome.tabs.CreateProperties);

    case "p:tab-update": {
      const { tabId: rawTabId, ...updateProps } = params;
      return await chrome.tabs.update(requireFiniteId(rawTabId, "tabId"), updateProps as chrome.tabs.UpdateProperties);
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
      await chrome.tabs.reload(requireFiniteId(params.tabId, "tabId"));
      return { reloaded: true };

    case "p:tab-group":
      return { groupId: await chrome.tabs.group(params as chrome.tabs.GroupOptions) };

    case "p:tab-ungroup":
      await chrome.tabs.ungroup(params.tabIds as number[]);
      return { ungrouped: true };

    case "p:group-update": {
      const { groupId: rawGroupId, ...groupProps } = params;
      return await chrome.tabGroups.update(requireFiniteId(rawGroupId, "groupId"), groupProps as chrome.tabGroups.UpdateProperties);
    }

    case "p:window-create":
      return await chrome.windows.create(params as chrome.windows.CreateData);

    case "p:window-remove":
      await chrome.windows.remove(requireFiniteId(params.windowId, "windowId"));
      return { removed: true };

    case "p:window-update": {
      const { windowId: rawWinId, ...winProps } = params;
      return await chrome.windows.update(requireFiniteId(rawWinId, "windowId"), winProps as chrome.windows.UpdateInfo);
    }

    case "p:execute-script": {
      const targetTabId = requireFiniteId(params.tabId, "tabId");
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

    case "p:screenshot-tile": {
      const opts = params.options as Record<string, unknown> ?? {};
      const mode = (opts.mode === "viewport" || opts.mode === "full") ? opts.mode : "viewport";
      const fmt = (opts.format === "png" || opts.format === "jpeg") ? opts.format : "png";
      const quality = typeof opts.quality === "number" ? Math.max(1, Math.min(100, opts.quality)) : 80;
      const tileMaxDim = typeof opts.tileMaxDim === "number" ? Math.max(50, opts.tileMaxDim) : 50;
      const maxBytes = typeof opts.maxBytes === "number" ? Math.max(50_000, opts.maxBytes) : 50_000;
      return await screenshot.captureTabTiles(
        params.tab as Record<string, unknown>,
        { mode, format: fmt, quality, tileMaxDim, maxBytes },
        { delay, executeWithTimeout },
      );
    }

    default:
      throw new Error(`Unknown action: ${action}`);
  }
}

async function getTabSnapshot() {
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
        incognito: win.incognito || false,
        url: tab.url,
        title: tab.title,
        active: tab.active,
        pinned: tab.pinned,
        groupId: tab.groupId,
        groupTitle: group ? group.title : null,
        groupColor: group ? group.color : null,
        groupCollapsed: group ? group.collapsed : null,
        lastAccessedAt: tab.lastAccessed || null,
        favIconUrl: tab.favIconUrl || null,
        status: tab.status || null,
        discarded: tab.discarded || false,
        audible: tab.audible || false,
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
      incognito: win.incognito || false,
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
