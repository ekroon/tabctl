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
const RECONNECT_ALARM = "tabctl-reconnect";
const KEEPALIVE_INTERVAL_MINUTES = 1;
const BROWSER_STATE_SYNC_DEBOUNCE_MS = 750;
const ACTIVE_PAGE_CACHE_DEBOUNCE_MS = 1_000;
const ACTIVE_PAGE_CACHE_TIMEOUT_MS = 5_000;
const MAX_PAGE_HTML_CHARS = 10 * 1024 * 1024;
const ACTIVE_PAGE_CACHE_MAX_HTML_CHARS = MAX_PAGE_HTML_CHARS;
const ACTIVE_PAGE_CACHE_QUIESCENT_DELAY_MS = 6_000;
const ACTIVE_PAGE_CACHE_QUIESCENT_RETRY_MS = 1_000;
const ACTIVE_PAGE_CACHE_QUIESCENT_TIMEOUT_MS = 2_500;
const ACTIVE_PAGE_CACHE_QUIESCENT_SAMPLE_MS = 350;
const ACTIVE_PAGE_CACHE_QUIESCENT_COOLDOWN_MS = 30_000;
const ACTIVE_PAGE_CACHE_STATUS_TIMEOUT_MS = 30_000;
const CACHE_AVAILABLE_BADGE_TEXT = "C";
const CACHE_AVAILABLE_BADGE_COLOR = "#2da44e";
const CACHE_WAITING_BADGE_TEXT = "W";
const CACHE_WAITING_BADGE_COLOR = "#bf8700";
const CACHE_ERROR_BADGE_TEXT = "E";
const CACHE_ERROR_BADGE_COLOR = "#cf222e";
const RECONNECT_INITIAL_DELAY_MS = 250;
const RECONNECT_MAX_DELAY_MS = 30_000;
const RECONNECT_ALARM_MIN_DELAY_MS = 30_000;
const RECONNECT_STABLE_RESET_MS = 5_000;
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
  reconnectTimer: null as ReturnType<typeof setTimeout> | null,
  reconnectStableTimer: null as ReturnType<typeof setTimeout> | null,
  reconnectAttempt: 0,
};

const browserState = {
  nextId: 1,
  syncTimer: null as ReturnType<typeof setTimeout> | null,
  pendingEvents: [] as Array<Record<string, unknown>>,
  incognitoWindowIds: new Set<number>(),
  incognitoTabIds: new Set<number>(),
  incognitoGroupIds: new Set<number>(),
};

const activePageCache = {
  nextId: 1,
  timer: null as ReturnType<typeof setTimeout> | null,
  pending: null as { tab: chrome.tabs.Tab & { id: number }; reason: string; key: string } | null,
  quiescentTimer: null as ReturnType<typeof setTimeout> | null,
  quiescentPending: null as {
    tabId: number;
    key: string;
    reason: string;
    attempts: number;
  } | null,
  inFlightKeys: new Set<string>(),
  lastCapturedKey: null as string | null,
  lastQuiescentCapturedKey: null as string | null,
  lastQuiescentCapturedAt: 0,
  statusRequests: new Map<string, { tabId: number; url: string }>(),
  diagnostics: new Map<string, { kind: "waiting" | "error"; detail: string }>(),
};

function log(...args: Array<unknown>) {
  console.log("[tabctl]", ...args);
}

function reconnectDelayMs(attempt: number) {
  return Math.min(RECONNECT_INITIAL_DELAY_MS * (2 ** attempt), RECONNECT_MAX_DELAY_MS);
}

function clearReconnectTimer() {
  if (!state.reconnectTimer) {
    return;
  }
  clearTimeout(state.reconnectTimer);
  state.reconnectTimer = null;
}

function clearReconnectAlarm() {
  chrome.alarms.clear(RECONNECT_ALARM);
}

function clearReconnectStableTimer() {
  if (!state.reconnectStableTimer) {
    return;
  }
  clearTimeout(state.reconnectStableTimer);
  state.reconnectStableTimer = null;
}

function scheduleReconnect(reason: string) {
  if (state.port || state.reconnectTimer) {
    return;
  }

  const attempt = state.reconnectAttempt;
  const delayMs = reconnectDelayMs(attempt);
  state.reconnectAttempt += 1;
  log("Scheduling native host reconnect", { reason, delayMs, attempt });
  chrome.alarms.create(RECONNECT_ALARM, {
    delayInMinutes: Math.max(delayMs, RECONNECT_ALARM_MIN_DELAY_MS) / 60_000,
  });
  state.reconnectTimer = setTimeout(() => {
    state.reconnectTimer = null;
    clearReconnectAlarm();
    connectNative();
  }, delayMs);
}

function resetReconnectBackoffAfterStablePort(port: chrome.runtime.Port) {
  clearReconnectStableTimer();
  state.reconnectStableTimer = setTimeout(() => {
    state.reconnectStableTimer = null;
    if (state.port === port) {
      state.reconnectAttempt = 0;
    }
  }, RECONNECT_STABLE_RESET_MS);
}

function ensureKeepaliveAlarm() {
  chrome.alarms.create(KEEPALIVE_ALARM, { periodInMinutes: KEEPALIVE_INTERVAL_MINUTES });
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
    clearReconnectTimer();
    clearReconnectAlarm();
    state.port = port;
    resetReconnectBackoffAfterStablePort(port);
    port.onMessage.addListener((message) => {
      if (handlePageCacheStatusMessage(message)) {
        return;
      }
      void handleNativeMessage(port, message);
    });
    port.onDisconnect.addListener(() => {
      const lastError = chrome.runtime.lastError;
      if (lastError) {
        log("Native host disconnected:", lastError.message);
      } else {
        log("Native host disconnected");
      }
      if (state.port === port) {
        state.port = null;
      }
      activePageCache.statusRequests.clear();
      clearReconnectStableTimer();
      scheduleReconnect("disconnect");
    });
    log("Native host connected");
    queueBrowserStateSync("startup");
    void refreshActivePageCacheIndicator("connectNative");
  } catch (error) {
    log("Native host connection failed", error);
    scheduleReconnect("connect-failed");
  }
}

function nextBrowserStateId() {
  const id = browserState.nextId;
  browserState.nextId += 1;
  return `browser-state-${Date.now()}-${id}`;
}

function nextActivePageCacheId() {
  const id = activePageCache.nextId;
  activePageCache.nextId += 1;
  return `page-cache-capture-${Date.now()}-${id}`;
}

function nextActivePageCacheStatusId() {
  const id = activePageCache.nextId;
  activePageCache.nextId += 1;
  return `page-cache-status-${Date.now()}-${id}`;
}

function trackPageCacheStatusRequest(id: string, tabId: number, url: string) {
  activePageCache.statusRequests.set(id, { tabId, url });
  setTimeout(() => {
    activePageCache.statusRequests.delete(id);
  }, ACTIVE_PAGE_CACHE_STATUS_TIMEOUT_MS);
}

function isScriptableUrl(url: string) {
  const lower = url.toLowerCase();
  return lower.startsWith("http://") || lower.startsWith("https://");
}

function activePageCacheKey(tab: chrome.tabs.Tab, url: string) {
  return `${tab.id}:${url}`;
}

function activePageCacheKeyForTabId(tabId: number, url: string) {
  return `${tabId}:${url}`;
}

function activePageCacheUrl(tab: chrome.tabs.Tab) {
  return tab.url || tab.pendingUrl || "";
}

async function tabUrlMatches(tabId: number, expectedUrl: string) {
  try {
    return activePageCacheUrl(await chrome.tabs.get(tabId)) === expectedUrl;
  } catch {
    return false;
  }
}

async function clearCacheAvailableIndicator(tabId: number) {
  try {
    await chrome.action?.setBadgeText?.({ tabId, text: "" });
    await chrome.action?.setTitle?.({ tabId, title: "Tab Control" });
  } catch (error) {
    log("clear cache indicator failed", { tabId, error });
  }
}

async function setCacheBadgeIndicator(tabId: number, expectedUrl: string, text: string, color: string, title: string) {
  try {
    const tab = await chrome.tabs.get(tabId);
    if (!isEligibleActivePageCacheTab(tab) || activePageCacheUrl(tab) !== expectedUrl) {
      await clearCacheAvailableIndicator(tabId);
      return;
    }
    await chrome.action?.setBadgeBackgroundColor?.({ tabId, color });
    await chrome.action?.setBadgeText?.({ tabId, text });
    await chrome.action?.setTitle?.({ tabId, title });
  } catch (error) {
    log("set cache indicator failed", { tabId, text, error });
  }
}

async function setCacheAvailableIndicator(tabId: number, expectedUrl: string) {
  await setCacheBadgeIndicator(
    tabId,
    expectedUrl,
    CACHE_AVAILABLE_BADGE_TEXT,
    CACHE_AVAILABLE_BADGE_COLOR,
    "Tab Control - page cache available",
  );
}

async function setCacheWaitingIndicator(tabId: number, expectedUrl: string, detail: string) {
  await setCacheBadgeIndicator(
    tabId,
    expectedUrl,
    CACHE_WAITING_BADGE_TEXT,
    CACHE_WAITING_BADGE_COLOR,
    `Tab Control - waiting for page cache (${detail})`,
  );
}

async function setCacheErrorIndicator(tabId: number, expectedUrl: string, detail: string) {
  await setCacheBadgeIndicator(
    tabId,
    expectedUrl,
    CACHE_ERROR_BADGE_TEXT,
    CACHE_ERROR_BADGE_COLOR,
    `Tab Control - page cache error (${detail})`,
  );
}

function hasPendingActivePageCacheWork(tabId: number, url: string) {
  const key = activePageCacheKeyForTabId(tabId, url);
  return activePageCache.inFlightKeys.has(key)
    || activePageCache.pending?.key === key
    || activePageCache.quiescentPending?.key === key;
}

function setActivePageCacheDiagnostic(tabId: number, url: string, kind: "waiting" | "error", detail: string) {
  activePageCache.diagnostics.set(activePageCacheKeyForTabId(tabId, url), { kind, detail });
}

function clearActivePageCacheDiagnostic(tabId: number, url: string) {
  activePageCache.diagnostics.delete(activePageCacheKeyForTabId(tabId, url));
}

function clearActivePageCacheDiagnosticsForTab(tabId: number) {
  const prefix = `${tabId}:`;
  for (const key of activePageCache.diagnostics.keys()) {
    if (key.startsWith(prefix)) {
      activePageCache.diagnostics.delete(key);
    }
  }
}

async function applyPageCacheStatus(tabId: number, expectedUrl: string, available: boolean) {
  try {
    const tab = await chrome.tabs.get(tabId);
    if (activePageCacheUrl(tab) !== expectedUrl) {
      return;
    }
    if (!isEligibleActivePageCacheTab(tab)) {
      await clearCacheAvailableIndicator(tabId);
      return;
    }
    if (!available) {
      const diagnostic = activePageCache.diagnostics.get(activePageCacheKeyForTabId(tabId, expectedUrl));
      if (diagnostic?.kind === "waiting") {
        await setCacheWaitingIndicator(tabId, expectedUrl, diagnostic.detail);
        return;
      }
      if (diagnostic?.kind === "error") {
        await setCacheErrorIndicator(tabId, expectedUrl, diagnostic.detail);
        return;
      }
      if (hasPendingActivePageCacheWork(tabId, expectedUrl)) {
        await setCacheWaitingIndicator(tabId, expectedUrl, "capture pending");
        return;
      }
      await clearCacheAvailableIndicator(tabId);
      return;
    }
    clearActivePageCacheDiagnostic(tabId, expectedUrl);
    await setCacheAvailableIndicator(tabId, expectedUrl);
  } catch {
    await clearCacheAvailableIndicator(tabId);
  }
}

function handlePageCacheStatusMessage(message: {
  id?: string;
  action?: string;
  ok?: boolean;
  data?: Record<string, unknown>;
}) {
  if (!message || typeof message !== "object" || message.action) {
    return false;
  }
  const requestId = typeof message.id === "string" ? message.id : "";
  const pending = activePageCache.statusRequests.get(requestId);
  if (!pending) {
    return false;
  }
  activePageCache.statusRequests.delete(requestId);
  const data = message.data && typeof message.data === "object" ? message.data : {};
  const tabId = typeof data.tabId === "number" ? data.tabId : pending.tabId;
  const url = typeof data.url === "string" ? data.url : pending.url;
  const available = message.ok === true && data.available === true && tabId === pending.tabId && url === pending.url;
  void applyPageCacheStatus(pending.tabId, pending.url, available);
  return true;
}

function requestPageCacheStatus(tab: chrome.tabs.Tab, reason: string) {
  const url = activePageCacheUrl(tab);
  if (typeof tab.id !== "number" || !isEligibleActivePageCacheTab(tab)) {
    if (typeof tab.id === "number") {
      void clearCacheAvailableIndicator(tab.id);
    }
    return;
  }
  const port = state.port;
  if (!port) {
    connectNative();
    void setCacheWaitingIndicator(tab.id, url, "native host reconnecting");
    return;
  }

  const id = nextActivePageCacheStatusId();
  trackPageCacheStatusRequest(id, tab.id, url);
  port.postMessage({
    id,
    action: "page-cache-status",
    ok: true,
    data: {
      reason,
      requestedAt: Date.now(),
      tab: {
        tabId: tab.id,
        url,
        incognito: tab.incognito || false,
        discarded: tab.discarded || false,
        status: tab.status || null,
      },
    },
  });
}

async function requestPageCacheStatusForTab(tabId: number, reason: string) {
  try {
    requestPageCacheStatus(await chrome.tabs.get(tabId), reason);
  } catch {
    await clearCacheAvailableIndicator(tabId);
  }
}

async function refreshActivePageCacheIndicator(reason: string) {
  try {
    const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
    if (tab) {
      requestPageCacheStatus(tab, reason);
    }
  } catch (error) {
    log("active page cache indicator refresh failed", { reason, error });
  }
}

function urlMismatchPageHtmlResponse(expectedUrl: string) {
  return {
    status: "URL_MISMATCH",
    html: "",
    sourceHtmlChars: 0,
    sourceTextChars: 0,
    documentReadyState: null,
    truncatedHtml: false,
    error: `Tab URL changed before page HTML extraction completed: expected ${expectedUrl}`,
  };
}

function isEligibleActivePageCacheTab(tab: chrome.tabs.Tab): tab is chrome.tabs.Tab & { id: number } {
  const url = activePageCacheUrl(tab);
  return typeof tab.id === "number"
    && Boolean(url)
    && tab.incognito !== true
    && !browserState.incognitoTabIds.has(tab.id)
    && (typeof tab.windowId !== "number" || !browserState.incognitoWindowIds.has(tab.windowId))
    && tab.discarded !== true
    && tab.status !== "loading"
    && isScriptableUrl(url);
}

function clearPendingActivePageCacheCapture() {
  if (activePageCache.timer) {
    clearTimeout(activePageCache.timer);
    activePageCache.timer = null;
  }
  activePageCache.pending = null;
}

function clearPendingQuiescentActivePageCacheCapture() {
  if (activePageCache.quiescentTimer) {
    clearTimeout(activePageCache.quiescentTimer);
    activePageCache.quiescentTimer = null;
  }
  activePageCache.quiescentPending = null;
}

function isQuiescentCaptureCoolingDown(key: string) {
  return activePageCache.lastQuiescentCapturedKey === key
    && Date.now() - activePageCache.lastQuiescentCapturedAt < ACTIVE_PAGE_CACHE_QUIESCENT_COOLDOWN_MS;
}

function scheduleQuiescentActivePageCacheCapture(tab: chrome.tabs.Tab & { id: number }, reason: string, key: string) {
  if (isQuiescentCaptureCoolingDown(key)) {
    return;
  }

  clearPendingQuiescentActivePageCacheCapture();
  activePageCache.quiescentPending = {
    tabId: tab.id,
    key,
    reason: `${reason}:quiescent`,
    attempts: 0,
  };
  activePageCache.quiescentTimer = setTimeout(() => {
    activePageCache.quiescentTimer = null;
    const pending = activePageCache.quiescentPending;
    activePageCache.quiescentPending = null;
    if (pending) {
      void captureQuiescentActivePageCache(pending);
    }
  }, ACTIVE_PAGE_CACHE_QUIESCENT_DELAY_MS);
}

function scheduleActivePageCacheCapture(tab: chrome.tabs.Tab, reason: string) {
  if (!isEligibleActivePageCacheTab(tab)) {
    if (typeof tab.id === "number") {
      void clearCacheAvailableIndicator(tab.id);
    }
    clearPendingActivePageCacheCapture();
    clearPendingQuiescentActivePageCacheCapture();
    return;
  }

  const url = activePageCacheUrl(tab);
  const key = activePageCacheKey(tab, url);
  clearActivePageCacheDiagnostic(tab.id, url);
  void setCacheWaitingIndicator(tab.id, url, "checking status");
  requestPageCacheStatus(tab, reason);
  scheduleQuiescentActivePageCacheCapture(tab, reason, key);
  if (activePageCache.inFlightKeys.has(key) || activePageCache.lastCapturedKey === key) {
    return;
  }

  clearPendingActivePageCacheCapture();
  activePageCache.pending = { tab, reason, key };
  activePageCache.timer = setTimeout(() => {
    activePageCache.timer = null;
    const pending = activePageCache.pending;
    activePageCache.pending = null;
    if (pending) {
      void captureActivePageCache(pending.tab, pending.reason, pending.key);
    }
  }, ACTIVE_PAGE_CACHE_DEBOUNCE_MS);
}

async function scheduleActivePageCacheCaptureForTab(tabId: number, reason: string) {
  try {
    scheduleActivePageCacheCapture(await chrome.tabs.get(tabId), reason);
  } catch (error) {
    log("active page cache tab lookup failed", { tabId, reason, error });
    await clearCacheAvailableIndicator(tabId);
  }
}

async function scheduleActivePageCacheCaptureForWindow(windowId: number, reason: string) {
  if (windowId === chrome.windows.WINDOW_ID_NONE) {
    return;
  }
  try {
    const [tab] = await chrome.tabs.query({ active: true, windowId });
    if (tab) {
      scheduleActivePageCacheCapture(tab, reason);
    }
  } catch (error) {
    log("active page cache window lookup failed", { windowId, reason, error });
  }
}

async function currentActivePageCacheTab(tabId: number, key: string) {
  try {
    const tab = await chrome.tabs.get(tabId);
    const url = activePageCacheUrl(tab);
    if (!tab.active || !isEligibleActivePageCacheTab(tab) || activePageCacheKey(tab, url) !== key) {
      return null;
    }
    return tab;
  } catch {
    return null;
  }
}

async function rescheduleQuiescentActivePageCacheCapture(
  pending: { tabId: number; key: string; reason: string; attempts: number },
) {
  if (pending.attempts >= 2 || isQuiescentCaptureCoolingDown(pending.key)) {
    return;
  }
  activePageCache.quiescentPending = { ...pending, attempts: pending.attempts + 1 };
  activePageCache.quiescentTimer = setTimeout(() => {
    activePageCache.quiescentTimer = null;
    const retry = activePageCache.quiescentPending;
    activePageCache.quiescentPending = null;
    if (retry) {
      void captureQuiescentActivePageCache(retry);
    }
  }, ACTIVE_PAGE_CACHE_QUIESCENT_RETRY_MS);
}

async function captureQuiescentActivePageCache(pending: { tabId: number; key: string; reason: string; attempts: number }) {
  if (isQuiescentCaptureCoolingDown(pending.key)) {
    return;
  }
  if (activePageCache.inFlightKeys.has(pending.key)) {
    await rescheduleQuiescentActivePageCacheCapture(pending);
    return;
  }

  const tab = await currentActivePageCacheTab(pending.tabId, pending.key);
  if (!tab) {
    return;
  }

  const probe = await content.probePageQuiescence(
    tab.id,
    ACTIVE_PAGE_CACHE_QUIESCENT_TIMEOUT_MS,
    ACTIVE_PAGE_CACHE_QUIESCENT_SAMPLE_MS,
  );
  if (!probe.quiet) {
    await rescheduleQuiescentActivePageCacheCapture(pending);
    return;
  }

  const captured = await captureActivePageCache(tab, pending.reason, pending.key);
  if (captured) {
    activePageCache.lastQuiescentCapturedKey = pending.key;
    activePageCache.lastQuiescentCapturedAt = Date.now();
  }
}

async function captureActivePageCache(
  tab: chrome.tabs.Tab & { id: number },
  reason: string,
  key: string,
) {
  if (!state.port) {
    connectNative();
    return false;
  }
  if (!isEligibleActivePageCacheTab(tab) || activePageCache.inFlightKeys.has(key)) {
    return false;
  }

  activePageCache.inFlightKeys.add(key);
  try {
    const captureTab = await currentActivePageCacheTab(tab.id, key);
    if (!captureTab) {
      return false;
    }

    const extraction = await content.extractPageHtml(captureTab.id, ACTIVE_PAGE_CACHE_TIMEOUT_MS, ACTIVE_PAGE_CACHE_MAX_HTML_CHARS);
    if (extraction.status !== "READ" || typeof extraction.html !== "string" || extraction.html.length === 0) {
      log("active page cache extraction not readable", {
        tabId: captureTab.id,
        reason,
        key,
        status: extraction.status,
        error: extraction.error,
        sourceHtmlChars: extraction.sourceHtmlChars,
        sourceTextChars: extraction.sourceTextChars,
        documentReadyState: extraction.documentReadyState,
        truncatedHtml: extraction.truncatedHtml,
      });
      if (extraction.status === "NOT_LOADED") {
        const url = activePageCacheUrl(captureTab);
        setActivePageCacheDiagnostic(captureTab.id, url, "waiting", "page still loading");
        void setCacheWaitingIndicator(captureTab.id, url, "page still loading");
      } else {
        const detail = typeof extraction.status === "string" ? extraction.status.toLowerCase().replace(/_/g, " ") : "capture failed";
        const url = activePageCacheUrl(captureTab);
        setActivePageCacheDiagnostic(captureTab.id, url, "error", detail);
        void setCacheErrorIndicator(captureTab.id, url, detail);
      }
      void requestPageCacheStatusForTab(captureTab.id, `${reason}:capture-failed`);
      return false;
    }

    const verifiedTab = await currentActivePageCacheTab(captureTab.id, key);
    if (!verifiedTab) {
      return false;
    }

    const port = state.port;
    if (!port) {
      return false;
    }

    const id = nextActivePageCacheId();
    trackPageCacheStatusRequest(id, verifiedTab.id, activePageCacheUrl(verifiedTab));
    port.postMessage({
      id,
      action: "page-cache-capture",
      ok: true,
      data: {
        reason,
        capturedAt: Date.now(),
        tab: {
          tabId: verifiedTab.id,
          windowId: verifiedTab.windowId,
          index: verifiedTab.index,
          url: activePageCacheUrl(verifiedTab),
          title: verifiedTab.title,
          groupId: verifiedTab.groupId,
          favIconUrl: verifiedTab.favIconUrl || null,
          status: verifiedTab.status || null,
          pinned: verifiedTab.pinned || false,
          active: verifiedTab.active || false,
          incognito: verifiedTab.incognito || false,
          discarded: verifiedTab.discarded || false,
          lastAccessedAt: verifiedTab.lastAccessed || null,
        },
        extraction,
      },
    });
    activePageCache.lastCapturedKey = key;
    return true;
  } catch (error) {
    log("active page cache capture failed", { tabId: tab.id, reason, error });
    const url = activePageCacheUrl(tab);
    setActivePageCacheDiagnostic(tab.id, url, "error", "capture exception");
    void setCacheErrorIndicator(tab.id, url, "capture exception");
    void requestPageCacheStatusForTab(tab.id, `${reason}:capture-error`);
    return false;
  } finally {
    activePageCache.inFlightKeys.delete(key);
  }
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
      url: tab.url || tab.pendingUrl,
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
    if ("url" in changeInfo || "status" in changeInfo || "discarded" in changeInfo) {
      clearActivePageCacheDiagnosticsForTab(tabId);
      void clearCacheAvailableIndicator(tabId);
    }
    if (tab.active && ("url" in changeInfo || "status" in changeInfo || "discarded" in changeInfo)) {
      scheduleActivePageCacheCapture(tab, "tabs.onUpdated");
    }
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
    activePageCache.statusRequests.forEach((pending, requestId) => {
      if (pending.tabId === tabId) {
        activePageCache.statusRequests.delete(requestId);
      }
    });
    clearActivePageCacheDiagnosticsForTab(tabId);
    void clearCacheAvailableIndicator(tabId);
  });

  chrome.tabs?.onActivated?.addListener((activeInfo) => {
    enqueueBrowserStateEvent("tabs.onActivated", {
      tabId: activeInfo.tabId,
      windowId: activeInfo.windowId,
    });
    void scheduleActivePageCacheCaptureForTab(activeInfo.tabId, "tabs.onActivated");
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
    void scheduleActivePageCacheCaptureForWindow(windowId, "windows.onFocusChanged");
  });
}

connectNative();
registerBrowserStateListeners();
ensureKeepaliveAlarm();

chrome.runtime.onInstalled.addListener(() => {
  connectNative();
  ensureKeepaliveAlarm();
});

chrome.runtime.onStartup.addListener(() => {
  connectNative();
  ensureKeepaliveAlarm();
});

chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === KEEPALIVE_ALARM) {
    connectNative();
  } else if (alarm.name === RECONNECT_ALARM) {
    clearReconnectTimer();
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
      chrome.alarms.create(RECONNECT_ALARM, {
        delayInMinutes: RECONNECT_ALARM_MIN_DELAY_MS / 60_000,
      });
      // Defer reload to allow the response to be sent first. The reconnect alarm
      // gives the reloaded worker a product-owned wakeup if no other event fires.
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

    case "p:page-html": {
      const targetTabId = requireFiniteId(params.tabId, "tabId");
      const expectedUrl = typeof params.expectedUrl === "string" ? params.expectedUrl : "";
      const maxHtmlChars = typeof params.maxHtmlChars === "number"
        ? Math.max(1, Math.min(params.maxHtmlChars, MAX_PAGE_HTML_CHARS))
        : MAX_PAGE_HTML_CHARS;
      const timeoutMs = typeof params.timeoutMs === "number"
        ? Math.max(1, params.timeoutMs)
        : 15_000;

      if (expectedUrl && !(await tabUrlMatches(targetTabId, expectedUrl))) {
        return urlMismatchPageHtmlResponse(expectedUrl);
      }

      const result = await content.extractPageHtml(targetTabId, timeoutMs, maxHtmlChars);
      if (expectedUrl && !(await tabUrlMatches(targetTabId, expectedUrl))) {
        return urlMismatchPageHtmlResponse(expectedUrl);
      }
      return result;
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
        url: tab.url || tab.pendingUrl,
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
