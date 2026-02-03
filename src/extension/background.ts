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
const DEFAULT_STALE_DAYS = 30;
const DESCRIPTION_MAX_LENGTH = 250;
const SELECTOR_VALUE_MAX_LENGTH = 500;
const SCREENSHOT_TILE_MAX_DIM = 2000;
const SCREENSHOT_MAX_BYTES = 2_000_000;
const SCREENSHOT_QUALITY = 80;
const SCREENSHOT_SCROLL_DELAY_MS = 150;
const SCREENSHOT_CAPTURE_DELAY_MS = 350;
const SCREENSHOT_PROCESS_TIMEOUT_MS = 8000;
const SETTLE_STABILITY_MS = 500;
const SETTLE_POLL_INTERVAL_MS = 50;

type AnyRecord = Record<string, any>;

const state = {
  port: null,
  archiveWindowId: null,
  archiveWindowIdLoaded: false,
  lastFocused: {},
  lastFocusedLoaded: false,
};

function log(...args: Array<unknown>) {
  console.log("[TabArchive]", ...args);
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
      return await analyzeTabs(params, requestId);
    case "inspect":
      return await inspectTabs(params, requestId);
    case "focus":
      return await focusTab(params);
    case "refresh":
      return await refreshTabs(params);
    case "open":
      return await openTabs(params);
    case "group-list":
      return await listGroups(params);
    case "group-update":
      return await groupUpdate(params);
    case "group-ungroup":
      return await groupUngroup(params);
    case "group-assign":
      return await groupAssign(params);
    case "move-tab":
      return await moveTab(params);
    case "move-group":
      return await moveGroup(params);
    case "merge-window":
      return await mergeWindow(params);
    case "archive":
      return await archiveTabs(params);
    case "close":
      return await closeTabs(params);
    case "report":
      return await reportTabs(params);
    case "screenshot":
      return await screenshotTabs(params, requestId);
    case "undo":
      return await undoTransaction(params);
    default:
      throw new Error(`Unknown action: ${action}`);
  }
}

function isScriptableUrl(url: unknown) {
  return typeof url === "string" && (url.startsWith("http://") || url.startsWith("https://"));
}

function getMostRecentFocusedWindowId(windows: WindowSnapshot[]) {
  let bestWindowId: number | null = null;
  let bestFocusedAt = -Infinity;
  for (const win of windows) {
    for (const tab of win.tabs) {
      const focusedAt = Number(tab.lastFocusedAt);
      if (!Number.isFinite(focusedAt)) {
        continue;
      }
      if (focusedAt > bestFocusedAt) {
        bestFocusedAt = focusedAt;
        bestWindowId = win.windowId;
      }
    }
  }
  return bestWindowId;
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

function normalizeUrl(rawUrl: unknown) {
  if (!rawUrl || typeof rawUrl !== "string") {
    return null;
  }
  let url: URL;
  try {
    url = new URL(rawUrl);
  } catch {
    return null;
  }

  if (url.protocol !== "http:" && url.protocol !== "https:") {
    return null;
  }

  url.hash = "";
  const dropKeys = new Set([
    "fbclid",
    "gclid",
    "igshid",
    "mc_cid",
    "mc_eid",
    "ref",
    "ref_src",
    "ref_url",
    "utm_campaign",
    "utm_content",
    "utm_medium",
    "utm_source",
    "utm_term",
    "utm_name",
    "si",
  ]);
  for (const key of Array.from(url.searchParams.keys())) {
    if (key.startsWith("utm_") || dropKeys.has(key)) {
      url.searchParams.delete(key);
    }
  }

  const search = url.searchParams.toString();
  url.search = search ? `?${search}` : "";
  return url.toString();
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

function isGitHubIssueOrPr(url: string | null) {
  if (!url) {
    return false;
  }
  return /^https:\/\/github\.com\/[^/]+\/[^/]+\/(issues|pull)\/\d+/.test(url);
}

async function executeWithTimeout<T>(
  tabId: number,
  timeoutMs: number,
  func: (...args: Array<any>) => T,
  args: Array<unknown> = [],
): Promise<T | null> {
  const execPromise = chrome.scripting.executeScript({
    target: { tabId },
    func,
    args,
  });

  const timeoutPromise = new Promise<null>((resolve) => {
    const handle = setTimeout(() => {
      clearTimeout(handle);
      resolve(null);
    }, timeoutMs);
  });

  try {
    const result = await Promise.race([execPromise, timeoutPromise]);
    if (!result || !Array.isArray(result)) {
      return null;
    }
    const [{ result: value }] = result as Array<{ result?: T | null }>;
    return value ?? null;
  } catch {
    return null;
  }
}

async function detectGitHubState(tabId: number, timeoutMs: number) {
  const result = await executeWithTimeout(tabId, timeoutMs, () => {
    const stateEl =
      document.querySelector(".gh-header-meta .State") ||
      document.querySelector(".State") ||
      document.querySelector(".js-issue-state");

    if (!stateEl) {
      return null;
    }

    const text = (stateEl.textContent || "").trim().toLowerCase();
    if (text.includes("merged")) {
      return "merged";
    }
    if (text.includes("closed")) {
      return "closed";
    }
    if (text.includes("open")) {
      return "open";
    }
    return null;
  });

  return typeof result === "string" ? result : null;
}

async function extractPageMeta(tabId: number, timeoutMs: number) {
  const result = await executeWithTimeout(tabId, timeoutMs, () => {
    const pickContent = (selector: string) => {
      const el = document.querySelector(selector);
      if (!el) {
        return "";
      }
      const content = el.getAttribute("content") || el.textContent || "";
      return content.trim();
    };

    const description =
      pickContent("meta[name='description']") ||
      pickContent("meta[property='og:description']") ||
      pickContent("meta[name='twitter:description']");

    const h1 = document.querySelector("h1");
    const h1Text = h1 ? h1.textContent?.trim() : "";

    return {
      description: description.replace(/\s+/g, " ").trim(),
      h1: (h1Text || "").replace(/\s+/g, " ").trim(),
    };
  });

  if (!result || typeof result !== "object") {
    return null;
  }

  const meta = result as { description?: string; h1?: string };
  return {
    description: (meta.description || "").slice(0, DESCRIPTION_MAX_LENGTH),
    h1: (meta.h1 || "").slice(0, DESCRIPTION_MAX_LENGTH),
  };
}

async function extractSelectorSignal(tabId: number, specs: Array<Record<string, unknown>>, timeoutMs: number) {
  if (!specs.length) {
    return null;
  }

  const result = await executeWithTimeout(tabId, timeoutMs, (rawSpecs: Array<Record<string, unknown>>, maxLen: number) => {
    const values: Record<string, unknown> = {};
    const missing: string[] = [];
    const errors: Record<string, string> = {};
    const hints: Record<string, string> = {};

    for (const raw of rawSpecs) {
      const selector = typeof raw.selector === "string" ? raw.selector : "";
      if (!selector) {
        continue;
      }
      const name = typeof raw.name === "string" && raw.name ? raw.name : selector;
      const attr = typeof raw.attr === "string" ? raw.attr : "text";
      const all = Boolean(raw.all);
      const text = typeof raw.text === "string" ? raw.text.trim() : "";
      const textMode = typeof raw.textMode === "string" ? raw.textMode.trim().toLowerCase() : "";
      const normalizedTextMode = textMode === "includes" ? "contains" : textMode;
      const textModes = new Set(["", "contains", "exact", "starts-with"]);
      if (!textModes.has(normalizedTextMode)) {
        errors[name] = `Unsupported textMode: ${textMode || "unknown"}`;
        hints[name] = "Use textMode: contains | exact | starts-with";
        continue;
      }

      try {
        const elements = Array.from(document.querySelectorAll(selector));
        if (!elements.length) {
          missing.push(name);
          if (selector.includes(":contains(")) {
            hints[name] = "CSS :contains() is not supported; use selector text filters or a different selector.";
          } else {
            hints[name] = "No matches found; capture a screenshot for context or adjust the selector.";
          }
          continue;
        }

        const matchesText = (el: Element) => {
          if (!text) {
            return true;
          }
          const content = (el.textContent || "").replace(/\s+/g, " ").trim();
          if (normalizedTextMode === "exact") {
            return content === text;
          }
          if (normalizedTextMode === "starts-with") {
            return content.startsWith(text);
          }
          return content.includes(text);
        };

        const filtered = text ? elements.filter(matchesText) : elements;
        if (!filtered.length) {
          missing.push(name);
          hints[name] = "Selector matched elements, but none matched the text filter; capture a screenshot for context or adjust text/textMode.";
          continue;
        }

        const getValue = (el: Element) => {
          let value = "";
          if (attr === "text") {
            value = el.textContent || "";
          } else if (attr === "href-url" || attr === "src-url") {
            const rawValue = el.getAttribute(attr === "href-url" ? "href" : "src") || "";
            if (!rawValue) {
              value = "";
            } else {
              try {
                const resolved = new URL(rawValue, document.baseURI);
                if (resolved.protocol === "http:" || resolved.protocol === "https:") {
                  value = resolved.toString();
                } else {
                  value = "";
                }
              } catch {
                value = "";
              }
            }
          } else {
            value = el.getAttribute(attr) || "";
          }
          return value.replace(/\s+/g, " ").trim().slice(0, maxLen);
        };

        if (all) {
          values[name] = filtered.map(getValue).filter((val) => val.length > 0);
        } else {
          values[name] = getValue(filtered[0]);
        }
      } catch (err) {
        const message = err instanceof Error ? err.message : "selector_error";
        errors[name] = message;
        if (selector.includes(":contains(")) {
          hints[name] = "CSS :contains() is not supported; use selector text filters or a different selector.";
        } else {
          hints[name] = "Selector failed to evaluate; capture a screenshot for context or adjust the selector.";
        }
      }
    }

    return { values, missing, errors, hints };
  }, [specs, SELECTOR_VALUE_MAX_LENGTH]);

  if (!result || typeof result !== "object") {
    return null;
  }

  return result as Record<string, unknown>;
}

function delay(ms: number) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitForTabReady(tabId: number, params: Record<string, unknown>, fallbackTimeoutMs: number) {
  const waitFor = typeof params.waitFor === "string" ? params.waitFor.trim().toLowerCase() : "";
  if (!waitFor || waitFor === "none") {
    return;
  }
  const timeoutRaw = Number(params.waitTimeoutMs);
  const timeoutMs = Number.isFinite(timeoutRaw) && timeoutRaw > 0 ? Math.floor(timeoutRaw) : fallbackTimeoutMs;

  // settle mode handles its own URL checking, so skip the early return
  if (waitFor === "settle") {
    await waitForSettle(tabId, timeoutMs);
    return;
  }

  try {
    const tab = await chrome.tabs.get(tabId);
    if (!isScriptableUrl(tab.url)) {
      return;
    }
  } catch {
    return;
  }

  if (waitFor === "load") {
    await waitForTabLoad(tabId, timeoutMs);
    return;
  }

  if (waitFor === "dom") {
    await waitForDomReady(tabId, timeoutMs);
  }
}

function waitForTabLoad(tabId: number, timeoutMs: number) {
  return new Promise<void>((resolve) => {
    let settled = false;
    const done = () => {
      if (settled) {
        return;
      }
      settled = true;
      chrome.tabs.onUpdated.removeListener(onUpdated);
      resolve();
    };

    const onUpdated = (updatedTabId: number, info: chrome.tabs.TabChangeInfo) => {
      if (updatedTabId === tabId && info.status === "complete") {
        done();
      }
    };

    chrome.tabs.onUpdated.addListener(onUpdated);
    chrome.tabs.get(tabId).then((tab) => {
      if (tab.status === "complete") {
        done();
      }
    }).catch(() => {
      done();
    });

    setTimeout(done, timeoutMs);
  });
}

async function waitForDomReady(tabId: number, timeoutMs: number) {
  const result = await executeWithTimeout(tabId, timeoutMs, () => {
    if (document.readyState === "interactive" || document.readyState === "complete") {
      return true;
    }
    return new Promise<boolean>((resolve) => {
      const onReady = () => {
        document.removeEventListener("DOMContentLoaded", onReady);
        resolve(true);
      };
      document.addEventListener("DOMContentLoaded", onReady, { once: true });
      setTimeout(() => {
        document.removeEventListener("DOMContentLoaded", onReady);
        resolve(false);
      }, Math.max(0, timeoutMs - 50));
    });
  });

  if (result === null) {
    await delay(Math.min(200, Math.max(50, Math.floor(timeoutMs / 10))));
  }
}

async function waitForSettle(tabId: number, timeoutMs: number): Promise<void> {
  const startTime = Date.now();
  let lastUrl = "";
  let lastTitle = "";
  let stableStart = Date.now();

  while (Date.now() - startTime < timeoutMs) {
    const tab = await chrome.tabs.get(tabId).catch(() => null);
    if (!tab) return;

    const currentUrl = tab.url || "";
    const currentTitle = tab.title || "";

    // Reset stability timer if URL or title changed
    if (currentUrl !== lastUrl || currentTitle !== lastTitle) {
      lastUrl = currentUrl;
      lastTitle = currentTitle;
      stableStart = Date.now();
    } else if (
      isScriptableUrl(currentUrl) &&
      tab.status === "complete" &&
      Date.now() - stableStart >= SETTLE_STABILITY_MS
    ) {
      // Page is loaded, URL is valid, and stable for long enough
      return;
    }

    await delay(SETTLE_POLL_INTERVAL_MS);
  }
  // Timeout reached, continue anyway
}

function estimateDataUrlBytes(dataUrl: string) {
  const commaIndex = dataUrl.indexOf(",");
  if (commaIndex < 0) {
    return dataUrl.length;
  }
  const base64 = dataUrl.slice(commaIndex + 1);
  const padding = base64.endsWith("==") ? 2 : base64.endsWith("=") ? 1 : 0;
  return Math.max(0, Math.floor((base64.length * 3) / 4) - padding);
}

function arrayBufferToBase64(buffer: ArrayBuffer) {
  const bytes = new Uint8Array(buffer);
  let binary = "";
  for (let i = 0; i < bytes.length; i += 1) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary);
}

async function resizeDataUrl(
  dataUrl: string,
  format: "png" | "jpeg",
  quality: number,
  scale: number,
): Promise<{ dataUrl: string; bytes: number } | null> {
  if (!globalThis.OffscreenCanvas || !globalThis.createImageBitmap) {
    return null;
  }
  const response = await fetch(dataUrl);
  const blob = await response.blob();
  const bitmap = await createImageBitmap(blob);
  const width = Math.max(1, Math.floor(bitmap.width * scale));
  const height = Math.max(1, Math.floor(bitmap.height * scale));
  const canvas = new OffscreenCanvas(width, height);
  const ctx = canvas.getContext("2d");
  if (!ctx) {
    return null;
  }
  ctx.drawImage(bitmap, 0, 0, width, height);
  const type = format === "jpeg" ? "image/jpeg" : "image/png";
  const blobOut = await canvas.convertToBlob({ type, quality: format === "jpeg" ? quality / 100 : undefined });
  const buffer = await blobOut.arrayBuffer();
  const base64 = arrayBufferToBase64(buffer);
  return {
    dataUrl: `data:${type};base64,${base64}`,
    bytes: buffer.byteLength,
  };
}

async function resizeDataUrlToMaxDim(
  dataUrl: string,
  format: "png" | "jpeg",
  quality: number,
  maxDim: number,
): Promise<{ dataUrl: string; bytes: number } | null> {
  if (!globalThis.OffscreenCanvas || !globalThis.createImageBitmap) {
    return null;
  }
  const response = await fetch(dataUrl);
  const blob = await response.blob();
  const bitmap = await createImageBitmap(blob);
  const maxSize = Math.max(bitmap.width, bitmap.height);
  if (!Number.isFinite(maxSize) || maxSize <= maxDim) {
    return null;
  }
  const scale = maxDim / maxSize;
  return resizeDataUrl(dataUrl, format, quality, scale);
}

async function cropDataUrl(
  dataUrl: string,
  format: "png" | "jpeg",
  quality: number,
  width: number,
  height: number,
  devicePixelRatio: number,
): Promise<string> {
  if (!globalThis.OffscreenCanvas || !globalThis.createImageBitmap) {
    return dataUrl;
  }
  const response = await fetch(dataUrl);
  const blob = await response.blob();
  const bitmap = await createImageBitmap(blob);
  const targetWidth = Math.min(bitmap.width, Math.max(1, Math.round(width * devicePixelRatio)));
  const targetHeight = Math.min(bitmap.height, Math.max(1, Math.round(height * devicePixelRatio)));
  if (targetWidth === bitmap.width && targetHeight === bitmap.height) {
    return dataUrl;
  }
  const canvas = new OffscreenCanvas(targetWidth, targetHeight);
  const ctx = canvas.getContext("2d");
  if (!ctx) {
    return dataUrl;
  }
  ctx.drawImage(bitmap, 0, 0, targetWidth, targetHeight, 0, 0, targetWidth, targetHeight);
  const type = format === "jpeg" ? "image/jpeg" : "image/png";
  const blobOut = await canvas.convertToBlob({ type, quality: format === "jpeg" ? quality / 100 : undefined });
  const buffer = await blobOut.arrayBuffer();
  const base64 = arrayBufferToBase64(buffer);
  return `data:${type};base64,${base64}`;
}

async function ensureMaxBytes(
  dataUrl: string,
  format: "png" | "jpeg",
  quality: number,
  maxBytes: number,
): Promise<{ dataUrl: string; bytes: number; scaled: boolean; oversized: boolean }> {
  let currentUrl = dataUrl;
  let currentBytes = estimateDataUrlBytes(currentUrl);
  if (currentBytes <= maxBytes) {
    return { dataUrl: currentUrl, bytes: currentBytes, scaled: false, oversized: false };
  }

  let scaled = false;
  let attempts = 0;
  while (currentBytes > maxBytes && attempts < 3) {
    const scale = Math.max(0.2, Math.sqrt(maxBytes / currentBytes) * 0.95);
    if (scale >= 0.99) {
      break;
    }
    const resized = await resizeDataUrl(currentUrl, format, quality, scale);
    if (!resized) {
      break;
    }
    currentUrl = resized.dataUrl;
    currentBytes = resized.bytes;
    scaled = true;
    attempts += 1;
  }

  return {
    dataUrl: currentUrl,
    bytes: currentBytes,
    scaled,
    oversized: currentBytes > maxBytes,
  };
}

async function constrainDataUrl(
  dataUrl: string,
  format: "png" | "jpeg",
  quality: number,
  maxDim: number,
  maxBytes: number,
): Promise<{ dataUrl: string; bytes: number; scaled: boolean; oversized: boolean }> {
  let currentUrl = dataUrl;
  let currentBytes = estimateDataUrlBytes(currentUrl);
  let scaled = false;

  if (Number.isFinite(maxDim) && maxDim > 0) {
    const resized = await resizeDataUrlToMaxDim(currentUrl, format, quality, maxDim);
    if (resized) {
      currentUrl = resized.dataUrl;
      currentBytes = resized.bytes;
      scaled = true;
    }
  }

  if (currentBytes > maxBytes) {
    const resized = await ensureMaxBytes(currentUrl, format, quality, maxBytes);
    return {
      dataUrl: resized.dataUrl,
      bytes: resized.bytes,
      scaled: scaled || resized.scaled,
      oversized: resized.oversized,
    };
  }

  return { dataUrl: currentUrl, bytes: currentBytes, scaled, oversized: false };
}

async function captureVisible(windowId: number, format: "png" | "jpeg", quality: number) {
  const options: chrome.tabs.CaptureVisibleTabOptions = { format };
  if (format === "jpeg") {
    options.quality = quality;
  }
  return chrome.tabs.captureVisibleTab(windowId, options);
}

async function getPageMetrics(tabId: number, timeoutMs: number) {
  const result = await executeWithTimeout(tabId, timeoutMs, () => {
    const doc = document.documentElement;
    const body = document.body;
    const pageWidth = Math.max(
      doc.scrollWidth,
      doc.clientWidth,
      body ? body.scrollWidth : 0,
      body ? body.clientWidth : 0,
    );
    const pageHeight = Math.max(
      doc.scrollHeight,
      doc.clientHeight,
      body ? body.scrollHeight : 0,
      body ? body.clientHeight : 0,
    );
    return {
      pageWidth,
      pageHeight,
      viewportWidth: window.innerWidth,
      viewportHeight: window.innerHeight,
      devicePixelRatio: window.devicePixelRatio || 1,
      scrollX: window.scrollX || window.pageXOffset || 0,
      scrollY: window.scrollY || window.pageYOffset || 0,
    };
  });

  if (!result || typeof result !== "object") {
    return null;
  }
  return result as Record<string, unknown>;
}

async function scrollToPosition(tabId: number, timeoutMs: number, x: number, y: number) {
  const result = await executeWithTimeout(tabId, timeoutMs, (scrollX: number, scrollY: number) => {
    window.scrollTo(scrollX, scrollY);
    return {
      scrollX: window.scrollX || window.pageXOffset || 0,
      scrollY: window.scrollY || window.pageYOffset || 0,
    };
  }, [x, y]);
  if (!result || typeof result !== "object") {
    return null;
  }
  return result as Record<string, unknown>;
}

async function captureTabTiles(
  tab: Record<string, unknown>,
  options: {
    mode: "viewport" | "full";
    format: "png" | "jpeg";
    quality: number;
    tileMaxDim: number;
    maxBytes: number;
  },
): Promise<Array<Record<string, unknown>>> {
  const tabId = tab.tabId as number;
  const windowId = tab.windowId as number;
  if (!Number.isFinite(tabId) || !Number.isFinite(windowId)) {
    throw new Error("Missing tab/window id");
  }

  const metrics = await getPageMetrics(tabId, SCREENSHOT_PROCESS_TIMEOUT_MS);
  if (!metrics) {
    throw new Error("Failed to read page metrics");
  }

  const pageWidth = Number(metrics.pageWidth);
  const pageHeight = Number(metrics.pageHeight);
  const viewportWidth = Number(metrics.viewportWidth);
  const viewportHeight = Number(metrics.viewportHeight);
  const devicePixelRatio = Number(metrics.devicePixelRatio) || 1;
  const startScrollX = Number(metrics.scrollX) || 0;
  const startScrollY = Number(metrics.scrollY) || 0;

  if (!Number.isFinite(viewportWidth) || !Number.isFinite(viewportHeight)) {
    throw new Error("Viewport size unavailable");
  }

  const tiles: Array<Record<string, unknown>> = [];
  let tileIndex = 0;

  const captureTile = async (x: number, y: number, width: number, height: number, total: number) => {
    if (tileIndex > 0) {
      await delay(SCREENSHOT_CAPTURE_DELAY_MS);
    }
    const rawDataUrl = await captureVisible(windowId, options.format, options.quality);
    if (!rawDataUrl) {
      throw new Error("Capture failed");
    }
    const croppedUrl = await cropDataUrl(
      rawDataUrl,
      options.format,
      options.quality,
      width,
      height,
      devicePixelRatio,
    );
    const sizeResult = await constrainDataUrl(
      croppedUrl,
      options.format,
      options.quality,
      options.tileMaxDim,
      options.maxBytes,
    );
    tiles.push({
      index: tileIndex,
      total,
      x,
      y,
      width,
      height,
      scale: devicePixelRatio,
      format: options.format,
      bytes: sizeResult.bytes,
      scaled: sizeResult.scaled,
      oversized: sizeResult.oversized,
      dataUrl: sizeResult.dataUrl,
    });
    tileIndex += 1;
  };

  if (options.mode === "viewport") {
    await captureTile(startScrollX, startScrollY, viewportWidth, viewportHeight, 1);
    return tiles;
  }

  const stepX = viewportWidth;
  const stepY = Math.min(viewportHeight, options.tileMaxDim);
  const maxX = viewportWidth;
  const maxY = Math.max(viewportHeight, pageHeight);
  const tileCount = Math.ceil(maxX / stepX) * Math.ceil(maxY / stepY);

  for (let y = 0; y < maxY; y += stepY) {
    for (let x = 0; x < maxX; x += stepX) {
      await scrollToPosition(tabId, SCREENSHOT_PROCESS_TIMEOUT_MS, x, y);
      await delay(SCREENSHOT_SCROLL_DELAY_MS);
      const width = Math.min(stepX, maxX - x);
      const height = Math.min(stepY, maxY - y);
      try {
        await captureTile(x, y, width, height, tileCount);
      } catch (err) {
        const message = err instanceof Error ? err.message : "capture_failed";
        if (message.includes("MAX_CAPTURE_VISIBLE_TAB_CALLS_PER_SECOND")) {
          await delay(1000);
          await captureTile(x, y, width, height, tileCount);
        } else {
          throw err;
        }
      }
    }
  }

  await scrollToPosition(tabId, SCREENSHOT_PROCESS_TIMEOUT_MS, startScrollX, startScrollY);
  return tiles;
}

async function screenshotTabs(params: Record<string, unknown>, requestId: string) {
  const snapshot = await getTabSnapshot();
  const selection = selectTabsByScope(snapshot, params) as { tabs: Array<Record<string, unknown>>; error?: Record<string, unknown> };
  if (selection.error) {
    throw selection.error;
  }

  const mode = params.mode === "full" ? "full" : "viewport";
  const format = params.format === "jpeg" ? "jpeg" : "png";
  const qualityRaw = Number(params.quality);
  const quality = Number.isFinite(qualityRaw) ? Math.min(100, Math.max(0, Math.floor(qualityRaw))) : SCREENSHOT_QUALITY;
  const tileMaxDimRaw = Number(params.tileMaxDim);
  const tileMaxDim = Number.isFinite(tileMaxDimRaw) && tileMaxDimRaw > 0 ? Math.floor(tileMaxDimRaw) : SCREENSHOT_TILE_MAX_DIM;
  const adjustedTileMaxDim = tileMaxDim < 50 ? 50 : tileMaxDim;
  const maxBytesRaw = Number(params.maxBytes);
  const maxBytes = Number.isFinite(maxBytesRaw) && maxBytesRaw > 0 ? Math.floor(maxBytesRaw) : SCREENSHOT_MAX_BYTES;
  const adjustedMaxBytes = maxBytes < 50_000 ? 50_000 : maxBytes;
  const progressEnabled = params.progress === true;

  const tabs = selection.tabs;
  const entries: Array<Record<string, unknown>> = [];
  let totalTiles = 0;
  const startedAt = Date.now();

  for (let index = 0; index < tabs.length; index += 1) {
    const tab = tabs[index];
    const tabId = tab.tabId as number;
    const url = tab.url as string | undefined;
    if (!isScriptableUrl(url)) {
      entries.push({
        tabId,
        windowId: tab.windowId,
        groupId: tab.groupId,
        url: tab.url,
        title: tab.title,
        error: { message: "unsupported_url" },
        tiles: [],
      });
      if (progressEnabled) {
        sendProgress(requestId, { phase: "screenshot", processed: index + 1, total: tabs.length, tabId });
      }
      continue;
    }

    let tiles: Array<Record<string, unknown>> = [];
    let error: Record<string, unknown> | null = null;
    try {
      const windowId = tab.windowId as number;
      const activeTabs = await chrome.tabs.query({ windowId, active: true });
      const activeTabId = activeTabs[0]?.id ?? null;
      if (activeTabId && activeTabId !== tabId) {
        await chrome.tabs.update(tabId, { active: true });
        await delay(SCREENSHOT_SCROLL_DELAY_MS);
      }

      try {
        await waitForTabReady(tabId, params, SCREENSHOT_PROCESS_TIMEOUT_MS);
        tiles = await captureTabTiles(tab, { mode, format, quality, tileMaxDim: adjustedTileMaxDim, maxBytes: adjustedMaxBytes });
      } finally {
        if (activeTabId && activeTabId !== tabId) {
          await chrome.tabs.update(activeTabId, { active: true });
        }
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : "capture_failed";
      error = { message };
    }

    totalTiles += tiles.length;
    entries.push({
      tabId: tab.tabId,
      windowId: tab.windowId,
      groupId: tab.groupId,
      url: tab.url,
      title: tab.title,
      tiles,
      ...(error ? { error } : {}),
    });

    if (progressEnabled) {
      sendProgress(requestId, { phase: "screenshot", processed: index + 1, total: tabs.length, tabId });
    }
  }

  return {
    generatedAt: Date.now(),
    totals: { tabs: tabs.length, tiles: totalTiles },
    meta: {
      durationMs: Date.now() - startedAt,
      mode,
      format,
      quality: format === "jpeg" ? quality : null,
      tileMaxDim: adjustedTileMaxDim,
      maxBytes: adjustedMaxBytes,
    },
    entries,
  };
}

async function analyzeTabs(params: Record<string, unknown>, requestId: string) {
  const staleDays = Number.isFinite(params.staleDays) ? params.staleDays : DEFAULT_STALE_DAYS;
  const checkGitHub = params.checkGitHub === true;
  const githubConcurrencyRaw = Number(params.githubConcurrency);
  const githubConcurrency = Number.isFinite(githubConcurrencyRaw) && githubConcurrencyRaw > 0
    ? Math.min(10, Math.floor(githubConcurrencyRaw))
    : 4;
  const githubTimeoutRaw = Number(params.githubTimeoutMs);
  const githubTimeoutMs = Number.isFinite(githubTimeoutRaw) && githubTimeoutRaw > 0
    ? Math.floor(githubTimeoutRaw)
    : 4000;
  const progressEnabled = params.progress === true;
  const snapshot = await getTabSnapshot();
  const selection = selectTabsByScope(snapshot, params) as { tabs: Array<Record<string, unknown>>; error?: Record<string, unknown> };
  if (selection.error) {
    throw selection.error;
  }
  const selectedTabs = selection.tabs;
  const scopeTabs = selectedTabs;
  const now = Date.now();
  const startedAt = Date.now();
  let githubChecked = 0;
  let githubTotal = 0;
  let githubMatched = 0;

  const normalizedMap = new Map<string, Record<string, unknown>>();
  const duplicates = new Map<number, number>();

  for (const tab of scopeTabs) {
    const normalized = normalizeUrl(tab.url);
    if (!normalized) {
      continue;
    }
    if (normalizedMap.has(normalized)) {
      const existing = normalizedMap.get(normalized) as { tabId: number };
      duplicates.set(tab.tabId as number, existing.tabId);
    } else {
      normalizedMap.set(normalized, tab);
    }
  }

  const candidateMap = new Map<number, { tab: Record<string, unknown>; reasons: Array<Record<string, unknown>> }>();
  const addReason = (tab: Record<string, unknown>, reason: Record<string, unknown>) => {
    const tabId = tab.tabId as number;
    const entry = candidateMap.get(tabId) || { tab, reasons: [] };
    entry.reasons.push(reason);
    candidateMap.set(tabId, entry);
  };

  for (const tab of selectedTabs) {
    if (duplicates.has(tab.tabId as number)) {
      addReason(tab, {
        type: "duplicate",
        detail: `Matches tab ${duplicates.get(tab.tabId as number)}`,
      });
    }

    if (tab.lastFocusedAt) {
      const ageDays = (now - (tab.lastFocusedAt as number)) / (24 * 60 * 60 * 1000);
      if (ageDays >= (staleDays as number)) {
        addReason(tab, {
          type: "stale",
          detail: `Last focused ${Math.floor(ageDays)} days ago`,
        });
      }
    }
  }

  const githubTabs = checkGitHub
    ? selectedTabs.filter((tab) => isGitHubIssueOrPr(tab.url as string) && isScriptableUrl(tab.url))
    : [];
  githubTotal = githubTabs.length;

  if (checkGitHub && githubTabs.length > 0) {
    let index = 0;
    const total = githubTabs.length;
    const workers = Array.from({ length: Math.min(githubConcurrency, total) }, async () => {
      while (true) {
        const currentIndex = index;
        if (currentIndex >= total) {
          return;
        }
        index += 1;
        const tab = githubTabs[currentIndex];
        const state = await detectGitHubState(tab.tabId as number, githubTimeoutMs);
        githubChecked += 1;
        if (state === "closed" || state === "merged") {
          githubMatched += 1;
          addReason(tab, {
            type: "closed_issue",
            detail: `GitHub state: ${state}`,
          });
        }

        if (progressEnabled) {
          sendProgress(requestId, {
            phase: "github",
            processed: githubChecked,
            total,
            matched: githubMatched,
            tabId: tab.tabId,
            timeoutMs: githubTimeoutMs,
          });
        }
      }
    });

    await Promise.all(workers);
  }

  const candidates = Array.from(candidateMap.values()).map((entry) => {
    const reasons = entry.reasons;
    const severity = reasons.some((reason) => reason.type === "duplicate" || reason.type === "closed_issue")
      ? "high"
      : "medium";
    return {
      tabId: entry.tab.tabId,
      windowId: entry.tab.windowId,
      groupId: entry.tab.groupId,
      url: entry.tab.url,
      title: entry.tab.title,
      lastFocusedAt: entry.tab.lastFocusedAt,
      reasons,
      severity,
    };
  });

  return {
    generatedAt: Date.now(),
    staleDays,
    totals: {
      tabs: scopeTabs.length,
      analyzed: selectedTabs.length,
      candidates: candidates.length,
    },
    meta: {
      durationMs: Date.now() - startedAt,
      githubChecked,
      githubTotal,
      githubMatched,
      githubTimeoutMs,
    },
    candidates,
  };
}

async function inspectTabs(params: Record<string, unknown>, requestId: string) {
  const signalList = Array.isArray(params.signals) && params.signals.length > 0
    ? params.signals.map(String)
    : ["page-meta"];
  const signalConcurrencyRaw = Number(params.signalConcurrency);
  const signalConcurrency = Number.isFinite(signalConcurrencyRaw) && signalConcurrencyRaw > 0
    ? Math.min(10, Math.floor(signalConcurrencyRaw))
    : 4;
  const signalTimeoutRaw = Number(params.signalTimeoutMs);
  const signalTimeoutMs = Number.isFinite(signalTimeoutRaw) && signalTimeoutRaw > 0
    ? Math.floor(signalTimeoutRaw)
    : 4000;
  const progressEnabled = params.progress === true;

  // For settle mode with specific tab IDs, wait BEFORE taking snapshot
  // This ensures the tab URL is available for isScriptableUrl() checks
  const waitFor = typeof params.waitFor === "string" ? params.waitFor.trim().toLowerCase() : "";
  if (waitFor === "settle" && Array.isArray(params.tabIds) && params.tabIds.length > 0) {
    const waitTimeoutRaw = Number(params.waitTimeoutMs);
    const waitTimeoutMs = Number.isFinite(waitTimeoutRaw) && waitTimeoutRaw > 0 ? Math.floor(waitTimeoutRaw) : signalTimeoutMs;
    const tabIds = (params.tabIds as number[]).map(Number).filter(Number.isFinite);
    await Promise.all(tabIds.map((id) => waitForSettle(id, waitTimeoutMs)));
  }

  const snapshot = await getTabSnapshot();
  const selection = selectTabsByScope(snapshot, params) as { tabs: Array<Record<string, unknown>>; error?: Record<string, unknown> };
  if (selection.error) {
    throw selection.error;
  }

  const tabs = selection.tabs;
  const startedAt = Date.now();

  const selectorSpecs: Array<Record<string, unknown>> = [];
  if (Array.isArray(params.selectorSpecs)) {
    selectorSpecs.push(...(params.selectorSpecs as Array<Record<string, unknown>>));
  }
  if (params.signalConfig && typeof params.signalConfig === "object") {
    const config = params.signalConfig as Record<string, unknown>;
    if (Array.isArray(config.selectors)) {
      selectorSpecs.push(...(config.selectors as Array<Record<string, unknown>>));
    }
    if (config.signals && typeof config.signals === "object") {
      const signals = config.signals as Record<string, unknown>;
      const selectorConfig = signals.selector as Record<string, unknown> | undefined;
      if (selectorConfig && Array.isArray(selectorConfig.selectors)) {
        selectorSpecs.push(...(selectorConfig.selectors as Array<Record<string, unknown>>));
      }
    }
  }

  const normalizedSelectors = selectorSpecs
    .filter((spec) => spec && typeof spec.selector === "string" && spec.selector.length > 0)
    .map((spec) => ({
      name: typeof spec.name === "string" ? spec.name : undefined,
      selector: spec.selector,
      attr: typeof spec.attr === "string" ? spec.attr : "text",
      all: Boolean(spec.all),
      text: typeof spec.text === "string" && spec.text.trim() ? spec.text.trim() : undefined,
      textMode: typeof spec.textMode === "string" ? spec.textMode.trim().toLowerCase() : undefined,
    }));

  const selectorWarnings = normalizedSelectors
    .filter((spec) => typeof spec.selector === "string" && spec.selector.includes(":contains("))
    .map((spec) => ({
      name: spec.name || spec.selector,
      hint: "CSS :contains() is not supported; use selector text filters or a different selector.",
    }));

  const signalDefs: Array<{ id: string; match: (tab: Record<string, unknown>) => boolean; run: (tabId: number) => Promise<unknown> }> = [];
  for (const signalId of signalList) {
    if (signalId === "github-state") {
      signalDefs.push({
        id: signalId,
        match: (tab) => isGitHubIssueOrPr(tab.url as string) && isScriptableUrl(tab.url),
        run: async (tabId) => {
          const state = await detectGitHubState(tabId, signalTimeoutMs);
          return state ? { state } : null;
        },
      });
    } else if (signalId === "page-meta") {
      signalDefs.push({
        id: signalId,
        match: (tab) => isScriptableUrl(tab.url),
        run: async (tabId) => extractPageMeta(tabId, signalTimeoutMs),
      });
    } else if (signalId === "selector") {
      signalDefs.push({
        id: signalId,
        match: (tab) => isScriptableUrl(tab.url),
        run: async (tabId) => extractSelectorSignal(tabId, normalizedSelectors, signalTimeoutMs),
      });
    }
  }

  const tasks: Array<{ tab: Record<string, unknown>; signal: { id: string; run: (tabId: number) => Promise<unknown> } }> = [];
  for (const tab of tabs) {
    for (const signal of signalDefs) {
      if (signal.match(tab)) {
        tasks.push({ tab, signal });
      }
    }
  }

  const totalTasks = tasks.length;
  let completedTasks = 0;

  const entryMap = new Map<number, { tab: Record<string, unknown>; signals: Record<string, unknown> }>();

  const workerCount = Math.min(signalConcurrency, totalTasks || 1);
  let index = 0;
  const workers = Array.from({ length: workerCount }, async () => {
    while (true) {
      const currentIndex = index;
      if (currentIndex >= totalTasks) {
        return;
      }
      index += 1;
      const task = tasks[currentIndex];
      const tabId = task.tab.tabId as number;

      let result: unknown = null;
      let error: string | null = null;
      const started = Date.now();
      try {
        await waitForTabReady(tabId, params, signalTimeoutMs);
        result = await task.signal.run(tabId);
      } catch (err) {
        const message = err instanceof Error ? err.message : "signal_error";
        error = message;
      }
      const durationMs = Date.now() - started;

      const entry = entryMap.get(tabId) || { tab: task.tab, signals: {} };
      entry.signals[task.signal.id] = {
        ok: error === null,
        durationMs,
        data: result,
        error,
      };
      entryMap.set(tabId, entry);

      completedTasks += 1;
      if (progressEnabled) {
        sendProgress(requestId, {
          phase: "inspect",
          processed: completedTasks,
          total: totalTasks,
          signalId: task.signal.id,
          tabId,
        });
      }
    }
  });

  await Promise.all(workers);

  const entries = Array.from(entryMap.values()).map((entry) => ({
    tabId: entry.tab.tabId,
    windowId: entry.tab.windowId,
    groupId: entry.tab.groupId,
    url: entry.tab.url,
    title: entry.tab.title,
    signals: entry.signals,
  }));

  return {
    generatedAt: Date.now(),
    totals: {
      tabs: tabs.length,
      signals: signalDefs.length,
      tasks: totalTasks,
    },
    meta: {
      durationMs: Date.now() - startedAt,
      signalTimeoutMs,
      selectorCount: normalizedSelectors.length,
      selectorWarnings: selectorWarnings.length > 0 ? selectorWarnings : undefined,
    },
    entries,
  };
}

async function focusTab(params: Record<string, unknown>) {
  const tabIds = Array.isArray(params.tabIds) ? params.tabIds.map(Number) : [];
  const tabId = Number.isFinite(params.tabId as number)
    ? Number(params.tabId)
    : tabIds.length
      ? Number(tabIds[0])
      : null;

  if (!tabId) {
    throw new Error("Missing tabId");
  }

  const tab = await chrome.tabs.get(tabId);
  await chrome.windows.update(tab.windowId, { focused: true });
  await chrome.tabs.update(tabId, { active: true });

  return {
    tabId,
    windowId: tab.windowId,
  };
}

async function refreshTabs(params: Record<string, unknown>) {
  const tabId = Number.isFinite(params.tabId as number)
    ? Number(params.tabId)
    : null;

  if (!tabId) {
    throw new Error("Missing tabId");
  }

  await chrome.tabs.reload(tabId);

  return {
    tabId,
    summary: { refreshedTabs: 1 },
  };
}

type WindowSnapshot = {
  windowId: number;
  focused: boolean;
  tabs: Array<Record<string, unknown>>;
  groups: Array<Record<string, unknown>>;
};

function matchIncludes(value: unknown, needle: string) {
  if (!needle) {
    return false;
  }
  return typeof value === "string" && value.toLowerCase().includes(needle);
}

function resolveOpenWindow(snapshot: { windows: Array<Record<string, unknown>> }, params: Record<string, unknown>) {
  const windows = snapshot.windows as WindowSnapshot[];
  if (!windows.length) {
    return { error: { message: "No windows available" } };
  }

  if (params.windowId != null) {
    if (typeof params.windowId === "string") {
      const normalized = params.windowId.trim().toLowerCase();
      if (normalized === "active") {
        const focused = windows.find((win) => win.focused);
        if (focused) {
          return { windowId: focused.windowId };
        }
        return { error: { message: "Active window not found" } };
      }
      if (normalized === "last-focused") {
        const lastFocused = getMostRecentFocusedWindowId(windows);
        if (lastFocused != null) {
          return { windowId: lastFocused };
        }
        return { error: { message: "Last focused window not found" } };
      }
      if (normalized === "new") {
        return { error: { message: "--window new is only supported by open" } };
      }
    }
    const windowId = Number(params.windowId);
    const found = windows.find((win) => win.windowId === windowId);
    if (!found) {
      return { error: { message: "Window not found" } };
    }
    return { windowId };
  }

  if (params.windowTabId != null) {
    const tabId = Number(params.windowTabId);
    const found = windows.find((win) => win.tabs.some((tab) => tab.tabId === tabId));
    if (!found) {
      return { error: { message: "Window not found for tab" } };
    }
    return { windowId: found.windowId };
  }

  let candidates = [...windows];
  let filtered = false;

  if (typeof params.afterGroupTitle === "string" && params.afterGroupTitle.trim()) {
    const groupTitle = params.afterGroupTitle.trim();
    candidates = candidates.filter((win) => win.groups.some((group) => group.title === groupTitle));
    filtered = true;
  }

  if (typeof params.windowGroupTitle === "string" && params.windowGroupTitle.trim()) {
    const groupTitle = params.windowGroupTitle.trim();
    candidates = candidates.filter((win) => win.groups.some((group) => group.title === groupTitle));
    filtered = true;
  }

  if (typeof params.windowUrl === "string" && params.windowUrl.trim()) {
    const needle = params.windowUrl.trim().toLowerCase();
    candidates = candidates.filter((win) => win.tabs.some((tab) => matchIncludes(tab.url, needle)));
    filtered = true;
  }

  if (filtered) {
    if (candidates.length === 1) {
      return { windowId: candidates[0].windowId };
    }
    if (candidates.length === 0) {
      return { error: { message: "No matching window found" } };
    }
    return { error: { message: "Multiple windows match selection. Provide --window to disambiguate." } };
  }

  const focused = windows.find((win) => win.focused);
  if (focused) {
    return { windowId: focused.windowId };
  }

  if (windows.length === 1) {
    return { windowId: windows[0].windowId };
  }

  const lastFocused = getMostRecentFocusedWindowId(windows);
  if (lastFocused != null) {
    return { windowId: lastFocused };
  }

  return { error: { message: "Multiple windows available. Provide --window to target one." } };
}

async function openTabs(params: Record<string, unknown>) {
  const urls = Array.isArray(params.urls)
    ? params.urls.map((url) => (typeof url === "string" ? url.trim() : "")).filter(Boolean)
    : [];
  const groupTitle = typeof params.groupTitle === "string" ? params.groupTitle.trim() : "";
  const groupColor = typeof params.color === "string" ? params.color.trim() : "";
  const afterGroupTitle = typeof params.afterGroupTitle === "string" ? params.afterGroupTitle.trim() : "";
  const beforeTabId = Number.isFinite(params.beforeTabId as number) ? Number(params.beforeTabId) : null;
  const afterTabId = Number.isFinite(params.afterTabId as number) ? Number(params.afterTabId) : null;
  if (beforeTabId != null && afterTabId != null) {
    throw new Error("Only one target position is allowed");
  }
  const newWindow = params.newWindow === true;
  if (!urls.length && !newWindow) {
    throw new Error("No URLs provided");
  }

  if (newWindow) {
    if (afterGroupTitle || beforeTabId || afterTabId) {
      throw new Error("Cannot use --before/--after with --new-window");
    }
    if (params.windowId != null || params.windowGroupTitle || params.windowTabId != null || params.windowUrl) {
      throw new Error("Cannot combine --new-window with window selectors");
    }

    const created: Array<Record<string, unknown>> = [];
    const skipped: Array<Record<string, unknown>> = [];

    const createdWindow = await chrome.windows.create({ focused: false });
    const windowId = createdWindow.id as number;
    let seedTabs = createdWindow.tabs;
    if (!seedTabs) {
      seedTabs = await chrome.tabs.query({ windowId });
    }
    const seedTabId = seedTabs.find((tab) => typeof tab.id === "number")?.id ?? null;

    for (const url of urls) {
      try {
        const tab = await chrome.tabs.create({ windowId, url, active: false });
        created.push({
          tabId: tab.id,
          windowId: tab.windowId,
          index: tab.index,
          url: tab.url,
          title: tab.title,
        });
      } catch (error) {
        skipped.push({ url, reason: "create_failed" });
      }
    }

    if (!urls.length && seedTabs.length) {
      const tab = seedTabs[0];
      created.push({
        tabId: tab.id,
        windowId: tab.windowId,
        index: tab.index,
        url: tab.url,
        title: tab.title,
      });
    }

    if (seedTabId && created.length > 0 && urls.length > 0) {
      try {
        await chrome.tabs.remove(seedTabId);
      } catch (error) {
        log("Failed to remove seed tab", error);
      }
    }

    let groupId: number | null = null;
    if (groupTitle && created.length > 0) {
      try {
        const tabIds = created.map((tab) => tab.tabId as number).filter((id) => typeof id === "number");
        if (tabIds.length > 0) {
          groupId = await chrome.tabs.group({ tabIds, createProperties: { windowId } });
          const update: chrome.tabGroups.UpdateProperties = { title: groupTitle };
          if (groupColor) {
            update.color = groupColor as chrome.tabGroups.ColorEnum;
          }
          await chrome.tabGroups.update(groupId, update);
        }
      } catch (error) {
        log("Failed to create group", error);
        groupId = null;
      }
    }

    return {
      windowId,
      groupId,
      groupTitle: groupTitle || null,
      afterGroupTitle: null,
      insertIndex: null,
      created,
      skipped,
      summary: {
        createdTabs: created.length,
        skippedUrls: skipped.length,
        grouped: Boolean(groupId),
      },
    };
  }

  const snapshot = await getTabSnapshot();
  let openParams = params;
  if (params.windowId == null && (beforeTabId != null || afterTabId != null)) {
    const anchorId = beforeTabId != null ? beforeTabId : (afterTabId as number);
    const anchorWindow = (snapshot.windows as WindowSnapshot[])
      .find((win) => win.tabs.some((tab) => tab.tabId === anchorId));
    if (anchorWindow) {
      openParams = { ...params, windowId: anchorWindow.windowId };
    }
  }
  const selection = resolveOpenWindow(snapshot, openParams);
  if ((selection as { error?: Record<string, unknown> }).error) {
    throw (selection as { error: Record<string, unknown> }).error;
  }

  const windowId = (selection as { windowId: number }).windowId;
  const windowSnapshot = (snapshot.windows as WindowSnapshot[]).find((win) => win.windowId === windowId);
  if (!windowSnapshot) {
    throw new Error("Window snapshot unavailable");
  }
  const created: Array<Record<string, unknown>> = [];
  const skipped: Array<Record<string, unknown>> = [];
  let insertIndex: number | null = null;

  if (afterGroupTitle) {
    const targetGroup = windowSnapshot.groups.find((group) => group.title === afterGroupTitle);
    if (!targetGroup) {
      throw new Error("Group not found in target window");
    }
    const groupTabs = windowSnapshot.tabs.filter((tab) => tab.groupId === targetGroup.groupId);
    if (!groupTabs.length) {
      throw new Error("Group has no tabs to anchor insertion");
    }
    const indices = groupTabs
      .map((tab) => normalizeTabIndex(tab.index))
      .filter((value): value is number => value != null);
    if (!indices.length) {
      throw new Error("Group tabs missing indices");
    }
    insertIndex = Math.max(...indices) + 1;
  }

  if (beforeTabId != null || afterTabId != null) {
    if (afterGroupTitle) {
      throw new Error("Only one target position is allowed");
    }
    const anchorId = beforeTabId != null ? beforeTabId : afterTabId as number;
    const anchorTab = windowSnapshot.tabs.find((tab) => tab.tabId === anchorId);
    if (!anchorTab) {
      throw new Error("Anchor tab not found in target window");
    }
    const anchorIndex = normalizeTabIndex(anchorTab.index);
    if (!Number.isFinite(anchorIndex)) {
      throw new Error("Anchor tab index unavailable");
    }
    insertIndex = beforeTabId != null ? anchorIndex : anchorIndex + 1;
  }

  let nextIndex = insertIndex;
  for (const url of urls) {
    try {
      const createOptions: chrome.tabs.CreateProperties = { windowId, url, active: false };
      if (nextIndex != null) {
        createOptions.index = nextIndex;
        nextIndex += 1;
      }
      const tab = await chrome.tabs.create(createOptions);
      created.push({
        tabId: tab.id,
        windowId: tab.windowId,
        index: tab.index,
        url: tab.url,
        title: tab.title,
      });
    } catch (error) {
      skipped.push({ url, reason: "create_failed" });
    }
  }

  let groupId: number | null = null;
  if (groupTitle && created.length > 0) {
    try {
      const tabIds = created.map((tab) => tab.tabId as number).filter((id) => typeof id === "number");
      if (tabIds.length > 0) {
        groupId = await chrome.tabs.group({ tabIds, createProperties: { windowId } });
        const update: chrome.tabGroups.UpdateProperties = { title: groupTitle };
        if (groupColor) {
          update.color = groupColor as chrome.tabGroups.ColorEnum;
        }
        await chrome.tabGroups.update(groupId, update);
      }
    } catch (error) {
      log("Failed to create group", error);
      groupId = null;
    }
  }

  return {
    windowId,
    groupId,
    groupTitle: groupTitle || null,
    afterGroupTitle: afterGroupTitle || null,
    insertIndex,
    created,
    skipped,
    summary: {
      createdTabs: created.length,
      skippedUrls: skipped.length,
      grouped: Boolean(groupId),
    },
  };
}

type GroupMatch = {
  windowId: number;
  group: Record<string, unknown>;
  tabs: Array<Record<string, unknown>>;
};

type GroupSummary = {
  windowId: number;
  windowLabel: string | null;
  groupId: number;
  title: string | null;
};

function getGroupTabs(windowSnapshot: WindowSnapshot, groupId: number) {
  return windowSnapshot.tabs
    .filter((tab) => tab.groupId === groupId)
    .sort((a, b) => (Number(a.index) || 0) - (Number(b.index) || 0));
}

function listGroupSummaries(snapshot: { windows: Array<Record<string, unknown>> }, windowId?: number) {
  const windowLabels = buildWindowLabels(snapshot as { windows: Array<{ windowId: number }> });
  const summaries: GroupSummary[] = [];
  const windows = snapshot.windows as WindowSnapshot[];
  for (const win of windows) {
    if (windowId && win.windowId !== windowId) {
      continue;
    }
    for (const group of win.groups) {
      summaries.push({
        windowId: win.windowId,
        windowLabel: windowLabels.get(win.windowId) ?? null,
        groupId: group.groupId as number,
        title: typeof group.title === "string" ? group.title : null,
      });
    }
  }
  return summaries;
}

function summarizeGroupMatch(match: GroupMatch, windowLabels: Map<number, string>) {
  return {
    windowId: match.windowId,
    windowLabel: windowLabels.get(match.windowId) ?? null,
    groupId: match.group.groupId,
    title: typeof match.group.title === "string" ? match.group.title : null,
  };
}

function findGroupMatches(snapshot: { windows: Array<Record<string, unknown>> }, groupTitle: string, windowId?: number) {
  const matches: GroupMatch[] = [];
  const windows = snapshot.windows as WindowSnapshot[];
  for (const win of windows) {
    if (windowId && win.windowId !== windowId) {
      continue;
    }
    for (const group of win.groups) {
      if (group.title === groupTitle) {
        matches.push({
          windowId: win.windowId,
          group,
          tabs: getGroupTabs(win, group.groupId as number),
        });
      }
    }
  }
  return matches;
}

function resolveGroupByTitle(snapshot: { windows: Array<Record<string, unknown>> }, groupTitle: string, windowId?: number) {
  const windowLabels = buildWindowLabels(snapshot as { windows: Array<{ windowId: number }> });
  const allMatches = findGroupMatches(snapshot, groupTitle);
  const matches = windowId ? allMatches.filter((match) => match.windowId === windowId) : allMatches;
  const availableGroups = listGroupSummaries(snapshot);
  if (matches.length === 0) {
    const message = windowId && allMatches.length > 0
      ? "Group title not found in specified window"
      : "No matching group title found";
    return {
      error: {
        message,
        hint: "Use tabctl group-list to see existing groups.",
        matches: allMatches.map((match) => summarizeGroupMatch(match, windowLabels)),
        availableGroups,
      },
    };
  }
  if (matches.length > 1) {
    return {
      error: {
        message: "Group title is ambiguous. Provide a windowId.",
        hint: "Use --window to disambiguate group titles.",
        matches: matches.map((match) => summarizeGroupMatch(match, windowLabels)),
        availableGroups,
      },
    };
  }
  return { match: matches[0] };
}

function resolveGroupById(snapshot: { windows: Array<Record<string, unknown>> }, groupId: number) {
  const windows = snapshot.windows as WindowSnapshot[];
  const matches: GroupMatch[] = [];
  for (const win of windows) {
    const group = win.groups.find((entry) => entry.groupId === groupId);
    if (group) {
      matches.push({
        windowId: win.windowId,
        group,
        tabs: getGroupTabs(win, groupId),
      });
    }
  }
  if (matches.length === 0) {
    return {
      error: {
        message: "Group not found",
        hint: "Use tabctl group-list to see existing groups.",
        availableGroups: listGroupSummaries(snapshot),
      },
    };
  }
  if (matches.length > 1) {
    const windowLabels = buildWindowLabels(snapshot as { windows: Array<{ windowId: number }> });
    return {
      error: {
        message: "Group id is ambiguous. Provide a windowId.",
        hint: "Use --window to disambiguate group ids.",
        matches: matches.map((match) => summarizeGroupMatch(match, windowLabels)),
        availableGroups: listGroupSummaries(snapshot),
      },
    };
  }
  return { match: matches[0] };
}

function resolveMoveTarget(snapshot: { windows: Array<Record<string, unknown>> }, params: Record<string, unknown>) {
  const beforeTabId = Number(params.beforeTabId);
  const afterTabId = Number(params.afterTabId);
  const beforeGroupTitle = typeof params.beforeGroupTitle === "string" ? params.beforeGroupTitle.trim() : "";
  const afterGroupTitle = typeof params.afterGroupTitle === "string" ? params.afterGroupTitle.trim() : "";

  const targets = [
    Number.isFinite(beforeTabId) ? "before-tab" : null,
    Number.isFinite(afterTabId) ? "after-tab" : null,
    beforeGroupTitle ? "before-group" : null,
    afterGroupTitle ? "after-group" : null,
  ].filter(Boolean) as string[];

  if (targets.length === 0) {
    return { error: { message: "Missing target position (--before/--after)" } };
  }
  if (targets.length > 1) {
    return { error: { message: "Only one target position is allowed" } };
  }

  const windows = snapshot.windows as WindowSnapshot[];
  const findTab = (tabId: number) => {
    for (const win of windows) {
      const tab = win.tabs.find((entry) => entry.tabId === tabId);
      if (tab) {
        return { tab, windowId: win.windowId };
      }
    }
    return null;
  };

  if (targets[0] === "before-tab" || targets[0] === "after-tab") {
    const tabId = targets[0] === "before-tab" ? beforeTabId : afterTabId;
    if (!Number.isFinite(tabId)) {
      return { error: { message: "Invalid tab target" } };
    }
    const match = findTab(tabId);
    if (!match) {
      return { error: { message: "Target tab not found" } };
    }
    const index = normalizeTabIndex(match.tab.index);
    if (!Number.isFinite(index)) {
      return { error: { message: "Target tab index unavailable" } };
    }
    return {
      windowId: match.windowId,
      index: targets[0] === "before-tab" ? index : index + 1,
      anchor: { type: "tab", tabId },
    };
  }

  const groupTitle = targets[0] === "before-group" ? beforeGroupTitle : afterGroupTitle;
  const windowId = Number.isFinite(params.windowId as number) ? Number(params.windowId) : undefined;
  const resolved = resolveGroupByTitle(snapshot, groupTitle, windowId);
  if (resolved.error) {
    return resolved;
  }
  const match = resolved.match as GroupMatch;
  if (!match.tabs.length) {
    return { error: { message: "Target group has no tabs" } };
  }
  const indices = match.tabs
    .map((tab) => normalizeTabIndex(tab.index))
    .filter((value): value is number => value != null);
  if (!indices.length) {
    return { error: { message: "Target group indices unavailable" } };
  }
  const minIndex = Math.min(...indices);
  const maxIndex = Math.max(...indices);
  return {
    windowId: match.windowId,
    index: targets[0] === "before-group" ? minIndex : maxIndex + 1,
    anchor: { type: "group", groupId: match.group.groupId, groupTitle },
  };
}

async function moveTab(params: Record<string, unknown>) {
  const tabIds = Array.isArray(params.tabIds) ? params.tabIds.map(Number) : [];
  const tabId = Number.isFinite(params.tabId as number)
    ? Number(params.tabId)
    : tabIds.length
      ? Number(tabIds[0])
      : null;
  if (!tabId) {
    throw new Error("Missing tabId");
  }

  const snapshot = await getTabSnapshot();

  const windows = snapshot.windows as WindowSnapshot[];
  const sourceWindow = windows.find((win) => win.tabs.some((tab) => tab.tabId === tabId));
  if (!sourceWindow) {
    throw new Error("Source tab not found");
  }
  const sourceTab = sourceWindow.tabs.find((tab) => tab.tabId === tabId);
  if (!sourceTab) {
    throw new Error("Source tab not found");
  }

  const newWindow = params.newWindow === true;
  const hasTarget = Number.isFinite(params.beforeTabId as number)
    || Number.isFinite(params.afterTabId as number)
    || (typeof params.beforeGroupTitle === "string" && params.beforeGroupTitle.trim())
    || (typeof params.afterGroupTitle === "string" && params.afterGroupTitle.trim());
  if (newWindow) {
    if (hasTarget) {
      throw new Error("Cannot combine --new-window with --before/--after");
    }
    const createdWindow = await chrome.windows.create({ tabId, focused: false });
    const targetWindowId = createdWindow.id as number;
    let targetIndex = 0;
    const createdTab = createdWindow.tabs?.find((tab) => tab.id === tabId) || null;
    if (createdTab && Number.isFinite(createdTab.index)) {
      targetIndex = createdTab.index;
    } else {
      try {
        const updated = await chrome.tabs.get(tabId);
        targetIndex = updated.index;
      } catch {
        targetIndex = 0;
      }
    }
    return {
      tabId,
      from: { windowId: sourceWindow.windowId, index: sourceTab.index },
      to: { windowId: targetWindowId, index: targetIndex },
      summary: { movedTabs: 1 },
      undo: {
        action: "move-tab",
        tabId,
        from: {
          windowId: sourceWindow.windowId,
          index: sourceTab.index,
          groupId: sourceTab.groupId,
          groupTitle: sourceTab.groupTitle,
          groupColor: sourceTab.groupColor,
          groupCollapsed: sourceTab.groupCollapsed ?? null,
        },
        to: {
          windowId: targetWindowId,
          index: targetIndex,
        },
      },
      txid: params.txid || null,
    };
  }

  let normalizedParams = params;
  if (params.windowId != null) {
    const resolvedWindowId = resolveWindowIdFromParams(snapshot, params.windowId);
    normalizedParams = { ...params, windowId: resolvedWindowId ?? undefined };
  }

  const target = resolveMoveTarget(snapshot, normalizedParams);
  if ((target as { error?: Record<string, unknown> }).error) {
    throw (target as { error: Record<string, unknown> }).error;
  }

  const targetWindowId = (target as { windowId: number }).windowId;
  let targetIndex = (target as { index: number }).index;
  const sourceIndex = normalizeTabIndex(sourceTab.index);
  if (Number.isFinite(sourceIndex) && sourceWindow.windowId === targetWindowId && sourceIndex < targetIndex) {
    targetIndex -= 1;
  }

  const moved = await chrome.tabs.move(tabId, { windowId: targetWindowId, index: targetIndex });
  return {
    tabId,
    from: { windowId: sourceWindow.windowId, index: sourceTab.index },
    to: { windowId: targetWindowId, index: (moved as chrome.tabs.Tab).index },
    summary: { movedTabs: 1 },
    undo: {
      action: "move-tab",
      tabId,
      from: {
        windowId: sourceWindow.windowId,
        index: sourceTab.index,
        groupId: sourceTab.groupId,
        groupTitle: sourceTab.groupTitle,
        groupColor: sourceTab.groupColor,
        groupCollapsed: sourceTab.groupCollapsed ?? null,
      },
      to: {
        windowId: targetWindowId,
        index: (moved as chrome.tabs.Tab).index,
      },
    },
    txid: params.txid || null,
  };
}

async function moveGroup(params: Record<string, unknown>) {
  const groupId = Number.isFinite(params.groupId as number) ? Number(params.groupId) : null;
  const groupTitle = typeof params.groupTitle === "string" ? params.groupTitle.trim() : "";
  if (!groupId && !groupTitle) {
    throw new Error("Missing group identifier");
  }

  const snapshot = await getTabSnapshot();
  const windowIdParam = params.windowId != null ? resolveWindowIdFromParams(snapshot, params.windowId) ?? undefined : undefined;

  const resolvedGroup = groupId != null
    ? resolveGroupById(snapshot, groupId)
    : resolveGroupByTitle(snapshot, groupTitle, windowIdParam);
  if ((resolvedGroup as { error?: Record<string, unknown> }).error) {
    throw (resolvedGroup as { error: Record<string, unknown> }).error;
  }
  const source = (resolvedGroup as { match: GroupMatch }).match;
  if (!source.tabs.length) {
    throw new Error("Group has no tabs to move");
  }

  const newWindow = params.newWindow === true;
  const hasTarget = Number.isFinite(params.beforeTabId as number)
    || Number.isFinite(params.afterTabId as number)
    || (typeof params.beforeGroupTitle === "string" && params.beforeGroupTitle.trim())
    || (typeof params.afterGroupTitle === "string" && params.afterGroupTitle.trim());
  if (newWindow) {
    if (hasTarget) {
      throw new Error("Cannot combine --new-window with --before/--after");
    }
    const tabIds = source.tabs.map((tab) => tab.tabId).filter((id) => typeof id === "number") as number[];
    const [firstTabId, ...restTabIds] = tabIds;
    if (!firstTabId) {
      throw new Error("Group has no tabs to move");
    }
    const createdWindow = await chrome.windows.create({ tabId: firstTabId, focused: false });
    const targetWindowId = createdWindow.id as number;

    if (restTabIds.length > 0) {
      await chrome.tabs.move(restTabIds, { windowId: targetWindowId, index: -1 });
    }

    let newGroupId: number | null = null;
    try {
      newGroupId = await chrome.tabs.group({ tabIds, createProperties: { windowId: targetWindowId } });
      await chrome.tabGroups.update(newGroupId, {
        title: (source.group.title as string) || "",
        color: (source.group.color as chrome.tabGroups.ColorEnum) || "grey",
        collapsed: (source.group.collapsed as boolean | undefined) || false,
      });
    } catch (error) {
      log("Failed to regroup tabs", error);
    }

    const undoTabs = source.tabs
      .map((tab) => ({
        tabId: tab.tabId,
        windowId: tab.windowId,
        index: tab.index,
        groupId: tab.groupId,
        groupTitle: tab.groupTitle,
        groupColor: tab.groupColor,
        groupCollapsed: source.group.collapsed ?? null,
      }))
      .filter((tab) => typeof tab.tabId === "number") as Array<Record<string, unknown>>;

    return {
      groupId: source.group.groupId,
      windowId: source.windowId,
      movedToWindowId: targetWindowId,
      newGroupId,
      summary: { movedTabs: tabIds.length },
      undo: {
        action: "move-group",
        groupId: source.group.groupId,
        windowId: source.windowId,
        movedToWindowId: targetWindowId,
        groupTitle: source.group.title ?? null,
        groupColor: source.group.color ?? null,
        groupCollapsed: source.group.collapsed ?? null,
        tabs: undoTabs,
      },
      txid: params.txid || null,
    };
  }

  const target = resolveMoveTarget(snapshot, params);
  if ((target as { error?: Record<string, unknown> }).error) {
    throw (target as { error: Record<string, unknown> }).error;
  }

  if ((target as { anchor?: { type: string; tabId?: number; groupId?: number } }).anchor?.type === "tab") {
    const anchorTabId = (target as { anchor: { tabId: number } }).anchor.tabId;
    if (source.tabs.some((tab) => tab.tabId === anchorTabId)) {
      throw new Error("Target tab is within the source group");
    }
  }
  if ((target as { anchor?: { type: string; groupId?: number } }).anchor?.type === "group") {
    const anchorGroupId = (target as { anchor: { groupId: number } }).anchor.groupId;
    if (anchorGroupId === source.group.groupId && source.windowId === (target as { windowId: number }).windowId) {
      throw new Error("Target group matches source group");
    }
  }

  const tabIds = source.tabs.map((tab) => tab.tabId).filter((id) => typeof id === "number") as number[];
  const indices = source.tabs
    .map((tab) => normalizeTabIndex(tab.index))
    .filter((value): value is number => value != null);
  const minIndex = Math.min(...indices);
  const maxIndex = Math.max(...indices);
  const targetWindowId = (target as { windowId: number }).windowId;
  let targetIndex = (target as { index: number }).index;
  if (source.windowId === targetWindowId && targetIndex > maxIndex) {
    targetIndex -= tabIds.length;
  }

  const moved = await chrome.tabs.move(tabIds, { windowId: targetWindowId, index: targetIndex });
  const movedList = Array.isArray(moved) ? moved : [moved];

  let newGroupId: number | null = null;
  if (targetWindowId !== source.windowId) {
    try {
      const movedIds = movedList.map((tab) => tab.id as number).filter((id) => typeof id === "number");
      if (movedIds.length > 0) {
        newGroupId = await chrome.tabs.group({ tabIds: movedIds, createProperties: { windowId: targetWindowId } });
        await chrome.tabGroups.update(newGroupId, {
          title: (source.group.title as string) || "",
          color: (source.group.color as chrome.tabGroups.ColorEnum) || "grey",
          collapsed: (source.group.collapsed as boolean | undefined) || false,
        });
      }
    } catch (error) {
      log("Failed to regroup tabs", error);
    }
  }

  const undoTabs = source.tabs
    .map((tab) => ({
      tabId: tab.tabId,
      windowId: tab.windowId,
      index: tab.index,
      groupId: tab.groupId,
      groupTitle: tab.groupTitle,
      groupColor: tab.groupColor,
      groupCollapsed: source.group.collapsed ?? null,
    }))
    .filter((tab) => typeof tab.tabId === "number") as Array<Record<string, unknown>>;

  return {
    groupId: source.group.groupId,
    windowId: source.windowId,
    movedToWindowId: targetWindowId,
    newGroupId,
    summary: { movedTabs: tabIds.length },
    undo: {
      action: "move-group",
      groupId: source.group.groupId,
      windowId: source.windowId,
      movedToWindowId: targetWindowId,
      groupTitle: source.group.title ?? null,
      groupColor: source.group.color ?? null,
      groupCollapsed: source.group.collapsed ?? null,
      tabs: undoTabs,
    },
    txid: params.txid || null,
  };
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

async function listGroups(params: Record<string, unknown>) {
  const snapshot = await getTabSnapshot();
  const windows = snapshot.windows as WindowSnapshot[];
  const windowLabels = buildWindowLabels(snapshot as { windows: Array<{ windowId: number }> });
  const windowIdParam = params.windowId != null ? resolveWindowIdFromParams(snapshot, params.windowId) : null;
  if (windowIdParam && !windows.some((win) => win.windowId === windowIdParam)) {
    throw new Error("Window not found");
  }

  const groups: Array<Record<string, unknown>> = [];
  for (const win of windows) {
    if (windowIdParam && win.windowId !== windowIdParam) {
      continue;
    }
    const counts = new Map<number, number>();
    for (const tab of win.tabs) {
      const groupId = tab.groupId as number;
      if (typeof groupId === "number" && groupId !== -1) {
        counts.set(groupId, (counts.get(groupId) || 0) + 1);
      }
    }
    for (const group of win.groups) {
      const groupId = group.groupId as number;
      groups.push({
        windowId: win.windowId,
        windowLabel: windowLabels.get(win.windowId) ?? null,
        groupId,
        title: group.title ?? null,
        color: group.color ?? null,
        collapsed: group.collapsed ?? null,
        tabCount: counts.get(groupId) || 0,
      });
    }
  }

  return { groups };
}

async function groupUpdate(params: Record<string, unknown>) {
  const groupId = Number.isFinite(params.groupId as number) ? Number(params.groupId) : null;
  const groupTitle = typeof params.groupTitle === "string" ? params.groupTitle.trim() : "";
  if (!groupId && !groupTitle) {
    throw new Error("Missing group identifier");
  }

  const snapshot = await getTabSnapshot();
  const windows = snapshot.windows as WindowSnapshot[];
  const windowIdParam = params.windowId != null ? resolveWindowIdFromParams(snapshot, params.windowId) : null;
  if (windowIdParam && !windows.some((win) => win.windowId === windowIdParam)) {
    throw new Error("Window not found");
  }

  let match: GroupMatch;
  if (groupId != null) {
    const resolved = resolveGroupById(snapshot, groupId);
    if ((resolved as { error?: Record<string, unknown> }).error) {
      throw (resolved as { error: Record<string, unknown> }).error;
    }
    match = (resolved as { match: GroupMatch }).match;
    if (windowIdParam && windowIdParam !== match.windowId) {
      throw new Error("Group is not in the specified window");
    }
  } else {
    const resolved = resolveGroupByTitle(snapshot, groupTitle, windowIdParam || undefined);
    if ((resolved as { error?: Record<string, unknown> }).error) {
      throw (resolved as { error: Record<string, unknown> }).error;
    }
    match = (resolved as { match: GroupMatch }).match;
  }

  const update: chrome.tabGroups.UpdateProperties = {};
  if (typeof params.title === "string") {
    update.title = params.title;
  }
  if (typeof params.color === "string" && params.color.trim()) {
    update.color = params.color.trim() as chrome.tabGroups.ColorEnum;
  }
  if (typeof params.collapsed === "boolean") {
    update.collapsed = params.collapsed;
  }
  if (!Object.keys(update).length) {
    throw new Error("Missing group update fields");
  }

  const updated = await chrome.tabGroups.update(match.group.groupId as number, update);
  return {
    groupId: updated.id,
    windowId: updated.windowId,
    title: updated.title,
    color: updated.color,
    collapsed: updated.collapsed,
    undo: {
      action: "group-update",
      groupId: updated.id,
      windowId: match.windowId,
      previous: {
        title: match.group.title ?? null,
        color: match.group.color ?? null,
        collapsed: match.group.collapsed ?? null,
      },
    },
    txid: params.txid || null,
  };
}

async function groupUngroup(params: Record<string, unknown>) {
  const groupId = Number.isFinite(params.groupId as number) ? Number(params.groupId) : null;
  const groupTitle = typeof params.groupTitle === "string" ? params.groupTitle.trim() : "";
  if (!groupId && !groupTitle) {
    throw new Error("Missing group identifier");
  }

  const snapshot = await getTabSnapshot();
  const windows = snapshot.windows as WindowSnapshot[];
  const windowIdParam = params.windowId != null ? resolveWindowIdFromParams(snapshot, params.windowId) : null;
  if (windowIdParam && !windows.some((win) => win.windowId === windowIdParam)) {
    throw new Error("Window not found");
  }

  let match: GroupMatch;
  if (groupId != null) {
    const resolved = resolveGroupById(snapshot, groupId);
    if ((resolved as { error?: Record<string, unknown> }).error) {
      throw (resolved as { error: Record<string, unknown> }).error;
    }
    match = (resolved as { match: GroupMatch }).match;
    if (windowIdParam && windowIdParam !== match.windowId) {
      throw new Error("Group is not in the specified window");
    }
  } else {
    const resolved = resolveGroupByTitle(snapshot, groupTitle, windowIdParam || undefined);
    if ((resolved as { error?: Record<string, unknown> }).error) {
      throw (resolved as { error: Record<string, unknown> }).error;
    }
    match = (resolved as { match: GroupMatch }).match;
  }

  const undoTabs = match.tabs
    .map((tab) => ({
      tabId: tab.tabId,
      windowId: tab.windowId,
      index: tab.index,
      groupId: tab.groupId,
      groupTitle: tab.groupTitle,
      groupColor: tab.groupColor,
      groupCollapsed: match.group.collapsed ?? null,
    }))
    .filter((tab) => typeof tab.tabId === "number") as Array<Record<string, unknown>>;

  const tabIds = match.tabs
    .map((tab) => tab.tabId)
    .filter((tabId) => typeof tabId === "number") as number[];
  if (tabIds.length) {
    await chrome.tabs.ungroup(tabIds);
  }

  return {
    groupId: match.group.groupId,
    groupTitle: match.group.title || null,
    windowId: match.windowId,
    summary: {
      ungroupedTabs: tabIds.length,
    },
    undo: {
      action: "group-ungroup",
      groupId: match.group.groupId,
      windowId: match.windowId,
      groupTitle: match.group.title || null,
      groupColor: match.group.color || null,
      groupCollapsed: match.group.collapsed ?? null,
      tabs: undoTabs,
    },
    txid: params.txid || null,
  };
}

async function groupAssign(params: Record<string, unknown>) {
  const rawTabIds = Array.isArray(params.tabIds) ? params.tabIds.map(Number) : [];
  const tabIds = rawTabIds.filter((id) => Number.isFinite(id));
  if (!tabIds.length) {
    throw new Error("Missing tabIds");
  }

  const groupId = Number.isFinite(params.groupId as number) ? Number(params.groupId) : null;
  const groupTitle = typeof params.groupTitle === "string" ? params.groupTitle.trim() : "";
  if (!groupId && !groupTitle) {
    throw new Error("Missing group identifier");
  }

  const snapshot = await getTabSnapshot();
  const windows = snapshot.windows as WindowSnapshot[];
  const windowIdParam = params.windowId != null ? resolveWindowIdFromParams(snapshot, params.windowId) : null;
  if (windowIdParam && !windows.some((win) => win.windowId === windowIdParam)) {
    throw new Error("Window not found");
  }

  const tabIndex = new Map<number, { tab: Record<string, unknown>; windowId: number }>();
  for (const win of windows) {
    for (const tab of win.tabs) {
      if (typeof tab.tabId === "number") {
        tabIndex.set(tab.tabId, { tab, windowId: win.windowId });
      }
    }
  }

  const skipped: Array<Record<string, unknown>> = [];
  const resolvedTabIds: number[] = [];
  const sourceWindows = new Set<number>();
  const undoTabs: Array<Record<string, unknown>> = [];
  for (const tabId of tabIds) {
    const entry = tabIndex.get(tabId);
    if (!entry) {
      skipped.push({ tabId, reason: "not_found" });
      continue;
    }
    resolvedTabIds.push(tabId);
    sourceWindows.add(entry.windowId);
    const tab = entry.tab;
    undoTabs.push({
      tabId,
      windowId: entry.windowId,
      index: tab.index,
      groupId: tab.groupId,
      groupTitle: tab.groupTitle,
      groupColor: tab.groupColor,
      groupCollapsed: tab.groupCollapsed ?? null,
    });
  }

  if (!resolvedTabIds.length) {
    throw new Error("No matching tabs found");
  }

  let targetGroupId: number | null = null;
  let targetWindowId: number | null = null;
  let targetTitle: string | null = null;
  let created = false;

  if (groupId != null) {
    const resolved = resolveGroupById(snapshot, groupId);
    if ((resolved as { error?: Record<string, unknown> }).error) {
      throw (resolved as { error: Record<string, unknown> }).error;
    }
    const match = (resolved as { match: GroupMatch }).match;
    targetGroupId = match.group.groupId as number;
    targetWindowId = match.windowId;
    targetTitle = typeof match.group.title === "string" ? match.group.title : null;
    if (windowIdParam && windowIdParam !== targetWindowId) {
      throw new Error("Group is not in the specified window");
    }
  } else {
    const resolved = resolveGroupByTitle(snapshot, groupTitle, windowIdParam || undefined);
    if ((resolved as { error?: Record<string, unknown> }).error) {
      const error = (resolved as { error: Record<string, unknown> }).error;
      if (error.message === "No matching group title found" && params.create === true) {
        targetWindowId = windowIdParam || (sourceWindows.size === 1 ? Array.from(sourceWindows)[0] : null);
        if (!targetWindowId) {
          throw new Error("Multiple source windows. Provide --window to create a new group.");
        }
        targetTitle = groupTitle;
        created = true;
      } else {
        throw error;
      }
    } else {
      const match = (resolved as { match: GroupMatch }).match;
      targetGroupId = match.group.groupId as number;
      targetWindowId = match.windowId;
      targetTitle = typeof match.group.title === "string" && match.group.title ? match.group.title : groupTitle;
    }
  }

  if (!targetWindowId) {
    throw new Error("Target window not found");
  }

  const moveIds = resolvedTabIds.filter((tabId) => {
    const entry = tabIndex.get(tabId);
    return entry && entry.windowId !== targetWindowId;
  });

  if (moveIds.length > 0) {
    await chrome.tabs.move(moveIds, { windowId: targetWindowId, index: -1 });
  }

  let assignedGroupId: number | null = targetGroupId;
  if (targetGroupId != null) {
    await chrome.tabs.group({ groupId: targetGroupId, tabIds: resolvedTabIds });
  } else {
    assignedGroupId = await chrome.tabs.group({ tabIds: resolvedTabIds, createProperties: { windowId: targetWindowId } });
    const update: chrome.tabGroups.UpdateProperties = {};
    if (targetTitle) {
      update.title = targetTitle;
    }
    if (typeof params.color === "string" && params.color.trim()) {
      update.color = params.color.trim() as chrome.tabGroups.ColorEnum;
    }
    if (typeof params.collapsed === "boolean") {
      update.collapsed = params.collapsed;
    }
    if (Object.keys(update).length > 0) {
      try {
        await chrome.tabGroups.update(assignedGroupId, update);
      } catch (error) {
        log("Failed to update group", error);
      }
    }
    created = true;
  }

  return {
    groupId: assignedGroupId,
    groupTitle: targetTitle || groupTitle || null,
    windowId: targetWindowId,
    created,
    summary: {
      movedTabs: moveIds.length,
      groupedTabs: resolvedTabIds.length,
      skippedTabs: skipped.length,
    },
    skipped,
    undo: {
      action: "group-assign",
      groupId: assignedGroupId,
      groupTitle: targetTitle || groupTitle || null,
      groupColor: typeof params.color === "string" && params.color.trim() ? params.color.trim() : null,
      groupCollapsed: typeof params.collapsed === "boolean" ? params.collapsed : null,
      created,
      tabs: undoTabs,
    },
    txid: params.txid || null,
  };
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

function normalizeTabIndex(value: unknown) {
  const index = Number(value);
  return Number.isFinite(index) ? index : null;
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

async function ensureWindow(windowId?: number) {
  if (windowId) {
    try {
      const existing = await chrome.windows.get(windowId);
      if (existing) {
        return windowId;
      }
    } catch {
      // fall through to create
    }
  }

  const created = await chrome.windows.create({ focused: false });
  return created.id as number;
}

async function restoreTabsFromUndo(entries: Array<AnyRecord>) {
  const skipped: Array<AnyRecord> = [];
  const restored: Array<{ tabId: number; entry: AnyRecord; targetWindowId: number }> = [];
  const windowMap = new Map<number, number>();

  for (const entry of entries) {
    const tabId = Number(entry.tabId);
    if (!Number.isFinite(tabId)) {
      skipped.push({ tabId: entry.tabId, reason: "missing_tab" });
      continue;
    }

    const sourceWindowId = Number(entry.windowId);
    if (!Number.isFinite(sourceWindowId)) {
      skipped.push({ tabId, reason: "missing_window" });
      continue;
    }

    let targetWindowId = windowMap.get(sourceWindowId);
    if (!targetWindowId) {
      targetWindowId = await ensureWindow(sourceWindowId);
      windowMap.set(sourceWindowId, targetWindowId);
    }

    try {
      await chrome.tabs.move(tabId, { windowId: targetWindowId, index: -1 });
      restored.push({ tabId, entry, targetWindowId });
    } catch {
      skipped.push({ tabId, reason: "move_failed" });
    }
  }

  const groupsByWindow = new Map<string, {
    windowId: number;
    groupId: number | null;
    groupTitle: string | null;
    groupColor: string | null;
    groupCollapsed: boolean | null;
    tabIds: number[];
  }>();

  for (const item of restored) {
    const entry = item.entry;
    const rawGroupId = typeof entry.groupId === "number" ? entry.groupId : null;
    const groupId = rawGroupId != null && rawGroupId !== -1 ? rawGroupId : null;
    const groupTitle = typeof entry.groupTitle === "string" ? entry.groupTitle : null;
    if (!groupId && !groupTitle) {
      continue;
    }

    const groupKey = groupId != null ? `id:${groupId}` : `title:${groupTitle}`;
    const key = `${item.targetWindowId}:${groupKey}`;
    if (!groupsByWindow.has(key)) {
      groupsByWindow.set(key, {
        windowId: item.targetWindowId,
        groupId,
        groupTitle,
        groupColor: typeof entry.groupColor === "string" ? entry.groupColor : null,
        groupCollapsed: typeof entry.groupCollapsed === "boolean" ? entry.groupCollapsed : null,
        tabIds: [],
      });
    }
    groupsByWindow.get(key)?.tabIds.push(item.tabId);
  }

  for (const group of groupsByWindow.values()) {
    if (group.tabIds.length === 0) {
      continue;
    }

    let targetGroupId: number | null = null;
    if (group.groupId != null) {
      try {
        targetGroupId = await chrome.tabs.group({ groupId: group.groupId, tabIds: group.tabIds });
      } catch {
        targetGroupId = null;
      }
    }

    if (targetGroupId == null) {
      try {
        targetGroupId = await chrome.tabs.group({
          tabIds: group.tabIds,
          createProperties: { windowId: group.windowId },
        });
      } catch (error) {
        log("Failed to regroup tabs", error);
        continue;
      }
    }

    const update: chrome.tabGroups.UpdateProperties = {};
    if (typeof group.groupTitle === "string") {
      update.title = group.groupTitle;
    }
    if (typeof group.groupColor === "string" && group.groupColor) {
      update.color = group.groupColor as chrome.tabGroups.ColorEnum;
    }
    if (typeof group.groupCollapsed === "boolean") {
      update.collapsed = group.groupCollapsed;
    }
    if (Object.keys(update).length > 0) {
      try {
        await chrome.tabGroups.update(targetGroupId, update);
      } catch (error) {
        log("Failed to update restored group", error);
      }
    }
  }

  const orderByWindow = new Map<number, Array<{ tabId: number; index: number }>>();
  for (const item of restored) {
    const index = Number(item.entry.index);
    if (!Number.isFinite(index)) {
      continue;
    }
    if (!orderByWindow.has(item.targetWindowId)) {
      orderByWindow.set(item.targetWindowId, []);
    }
    orderByWindow.get(item.targetWindowId)?.push({ tabId: item.tabId, index });
  }

  for (const [targetWindowId, items] of orderByWindow.entries()) {
    const ordered = [...items].sort((a, b) => a.index - b.index);
    for (const item of ordered) {
      try {
        await chrome.tabs.move(item.tabId, { windowId: targetWindowId, index: item.index });
      } catch {
        // ignore ordering failures
      }
    }
  }

  return {
    summary: {
      restoredTabs: restored.length,
      skippedTabs: skipped.length,
    },
    skipped,
  };
}

async function undoGroupUpdate(undo: AnyRecord) {
  const groupId = Number(undo.groupId);
  if (!Number.isFinite(groupId)) {
    return {
      summary: { restoredGroups: 0, skippedGroups: 1 },
      skipped: [{ groupId: undo.groupId, reason: "missing_group" }],
    };
  }

  const previous = (undo.previous as AnyRecord) || {};
  const update: chrome.tabGroups.UpdateProperties = {};
  if (typeof previous.title === "string") {
    update.title = previous.title;
  }
  if (typeof previous.color === "string" && previous.color) {
    update.color = previous.color as chrome.tabGroups.ColorEnum;
  }
  if (typeof previous.collapsed === "boolean") {
    update.collapsed = previous.collapsed;
  }
  if (!Object.keys(update).length) {
    return {
      summary: { restoredGroups: 0, skippedGroups: 1 },
      skipped: [{ groupId, reason: "missing_values" }],
    };
  }

  try {
    await chrome.tabGroups.update(groupId, update);
    return {
      summary: { restoredGroups: 1, skippedGroups: 0 },
      skipped: [],
    };
  } catch {
    return {
      summary: { restoredGroups: 0, skippedGroups: 1 },
      skipped: [{ groupId, reason: "update_failed" }],
    };
  }
}

async function undoGroupUngroup(undo: AnyRecord) {
  const tabs = (undo.tabs as Array<AnyRecord>) || [];
  return await restoreTabsFromUndo(tabs);
}

async function undoGroupAssign(undo: AnyRecord) {
  const tabs = (undo.tabs as Array<AnyRecord>) || [];
  return await restoreTabsFromUndo(tabs);
}

async function undoMoveTab(undo: AnyRecord) {
  const from = (undo.from as AnyRecord) || {};
  const entry = {
    tabId: undo.tabId,
    windowId: from.windowId,
    index: from.index,
    groupId: from.groupId,
    groupTitle: from.groupTitle,
    groupColor: from.groupColor,
    groupCollapsed: from.groupCollapsed,
  };
  return await restoreTabsFromUndo([entry]);
}

async function undoMoveGroup(undo: AnyRecord) {
  const tabs = (undo.tabs as Array<AnyRecord>) || [];
  return await restoreTabsFromUndo(tabs);
}

async function undoMergeWindow(undo: AnyRecord) {
  const tabs = (undo.tabs as Array<AnyRecord>) || [];
  return await restoreTabsFromUndo(tabs);
}

async function undoArchive(undo: AnyRecord) {
  const tabs = (undo.tabs as Array<AnyRecord>) || [];
  const restored: Array<{ tabId: number; targetWindowId: number }> = [];
  const skipped: Array<AnyRecord> = [];

  const windowMap = new Map<number, number>();
  for (const entry of tabs) {
    if (!entry.tabId) {
      skipped.push({ tabId: entry.tabId, reason: "missing_tab" });
      continue;
    }

    let targetWindowId = windowMap.get(entry.from.windowId as number);
    if (!targetWindowId) {
      targetWindowId = await ensureWindow(entry.from.windowId as number);
      windowMap.set(entry.from.windowId as number, targetWindowId);
    }

    try {
      await chrome.tabs.move(entry.tabId as number, { windowId: targetWindowId, index: -1 });
      restored.push({ tabId: entry.tabId as number, targetWindowId });
    } catch {
      skipped.push({ tabId: entry.tabId, reason: "move_failed" });
    }
  }

  const restoredSet = new Set(restored.map((item) => item.tabId));

  const groupsByWindow = new Map<string, Array<AnyRecord>>();
  for (const entry of tabs) {
    if (!restoredSet.has(entry.tabId as number)) {
      continue;
    }
    const targetWindowId = windowMap.get(entry.from.windowId as number) || (entry.from.windowId as number);
    const hasGroupId = entry.from.groupId != null && entry.from.groupId !== -1;
    const hasGroupTitle = entry.from.groupTitle != null;
    if (!hasGroupId && !hasGroupTitle) {
      continue;
    }
    const groupKey = hasGroupId ? entry.from.groupId : entry.from.groupTitle;
    const key = `${targetWindowId}:${groupKey}`;
    if (!groupsByWindow.has(key)) {
      groupsByWindow.set(key, []);
    }
    groupsByWindow.get(key)?.push(entry);
  }

  for (const [key, groupTabs] of groupsByWindow.entries()) {
    const [windowIdPart] = key.split(":");
    const targetWindowId = Number(windowIdPart);
    const tabIds = groupTabs.map((entry) => entry.tabId).filter(Boolean) as number[];
    if (!tabIds.length) {
      continue;
    }
    try {
      const groupId = await chrome.tabs.group({ tabIds, createProperties: { windowId: targetWindowId } });
      await chrome.tabGroups.update(groupId, {
        title: (groupTabs[0].from.groupTitle as string) || "",
        color: (groupTabs[0].from.groupColor as chrome.tabGroups.ColorEnum) || "grey",
        collapsed: groupTabs[0].from.groupCollapsed as boolean | undefined || false,
      });
    } catch (error) {
      log("Failed to recreate group", error);
    }
  }

  for (const [originalWindowId, targetWindowId] of windowMap.entries()) {
    const windowTabs = tabs
      .filter((entry) => entry.from.windowId === originalWindowId && restoredSet.has(entry.tabId as number))
      .sort((a, b) => (a.from.index as number) - (b.from.index as number));
    for (const entry of windowTabs) {
      if (!entry.tabId) {
        continue;
      }
      try {
        await chrome.tabs.move(entry.tabId as number, { windowId: targetWindowId, index: entry.from.index as number });
      } catch {
        // ignore ordering failures
      }
    }

    const activeTab = windowTabs.find((entry) => entry.active);
    if (activeTab && activeTab.tabId) {
      try {
        await chrome.tabs.update(activeTab.tabId as number, { active: true });
      } catch {
        // ignore
      }
    }
  }

  return {
    summary: {
      restoredTabs: restored.length,
      skippedTabs: skipped.length,
    },
    skipped,
  };
}

async function undoClose(undo: AnyRecord) {
  const tabs = (undo.tabs as Array<AnyRecord>) || [];
  const restored: Array<{ tabId: number; entry: AnyRecord }> = [];
  const skipped: Array<AnyRecord> = [];

  const windowMap = new Map<number, number>();
  for (const entry of tabs) {
    if (!entry.url) {
      skipped.push({ url: entry.url, reason: "missing_url" });
      continue;
    }
    let targetWindowId = windowMap.get(entry.from.windowId as number);
    if (!targetWindowId) {
      targetWindowId = await ensureWindow(entry.from.windowId as number);
      windowMap.set(entry.from.windowId as number, targetWindowId);
    }

    try {
      const created = await chrome.tabs.create({
        windowId: targetWindowId,
        url: entry.url as string,
        active: false,
        pinned: entry.pinned as boolean,
      });
      restored.push({ tabId: created.id as number, entry });
    } catch {
      skipped.push({ url: entry.url, reason: "create_failed" });
    }
  }

  const groupsByWindow = new Map<string, Array<AnyRecord>>();
  for (const item of restored) {
    const entry = item.entry;
    const targetWindowId = windowMap.get(entry.from.windowId as number) || (entry.from.windowId as number);
    const hasGroupId = entry.from.groupId != null && entry.from.groupId !== -1;
    const hasGroupTitle = entry.from.groupTitle != null;
    if (!hasGroupId && !hasGroupTitle) {
      continue;
    }
    const groupKey = hasGroupId ? entry.from.groupId : entry.from.groupTitle;
    const key = `${targetWindowId}:${groupKey}`;
    if (!groupsByWindow.has(key)) {
      groupsByWindow.set(key, []);
    }
    groupsByWindow.get(key)?.push({ tabId: item.tabId, entry });
  }

  for (const [key, groupTabs] of groupsByWindow.entries()) {
    const [windowIdPart] = key.split(":");
    const targetWindowId = Number(windowIdPart);
    const tabIds = groupTabs.map((item) => item.tabId).filter(Boolean) as number[];
    if (!tabIds.length) {
      continue;
    }
    try {
      const groupId = await chrome.tabs.group({ tabIds, createProperties: { windowId: targetWindowId } });
      await chrome.tabGroups.update(groupId, {
        title: (groupTabs[0].entry.from.groupTitle as string) || "",
        color: (groupTabs[0].entry.from.groupColor as chrome.tabGroups.ColorEnum) || "grey",
        collapsed: (groupTabs[0].entry.from.groupCollapsed as boolean | undefined) || false,
      });
    } catch (error) {
      log("Failed to recreate group", error);
    }
  }

  for (const [originalWindowId, targetWindowId] of windowMap.entries()) {
    const windowTabs = restored
      .map((item) => ({ tabId: item.tabId, entry: item.entry }))
      .filter((item) => item.entry.from.windowId === originalWindowId)
      .sort((a, b) => (a.entry.from.index as number) - (b.entry.from.index as number));

    for (const item of windowTabs) {
      try {
        await chrome.tabs.move(item.tabId, { windowId: targetWindowId, index: item.entry.from.index as number });
      } catch {
        // ignore ordering failures
      }
    }

    const activeTab = windowTabs.find((item) => item.entry.active);
    if (activeTab) {
      try {
        await chrome.tabs.update(activeTab.tabId as number, { active: true });
      } catch {
        // ignore
      }
    }
  }

  return {
    summary: {
      restoredTabs: restored.length,
      skippedTabs: skipped.length,
    },
    skipped,
  };
}

async function undoTransaction(params: Record<string, unknown>) {
  if (!params.record || !(params.record as Record<string, unknown>).undo) {
    throw new Error("Undo record missing");
  }

  const undo = (params.record as Record<string, unknown>).undo as Record<string, unknown>;
  if (undo.action === "archive") {
    return await undoArchive(undo);
  }
  if (undo.action === "close") {
    return await undoClose(undo);
  }
  if (undo.action === "group-update") {
    return await undoGroupUpdate(undo);
  }
  if (undo.action === "group-ungroup") {
    return await undoGroupUngroup(undo);
  }
  if (undo.action === "group-assign") {
    return await undoGroupAssign(undo);
  }
  if (undo.action === "move-tab") {
    return await undoMoveTab(undo);
  }
  if (undo.action === "move-group") {
    return await undoMoveGroup(undo);
  }
  if (undo.action === "merge-window") {
    return await undoMergeWindow(undo);
  }

  throw new Error(`Unknown undo action: ${undo.action}`);
}
