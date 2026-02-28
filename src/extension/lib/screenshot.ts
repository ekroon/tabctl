// Screenshot pipeline — extracted from background.ts (pure structural refactor).

export const SCREENSHOT_TILE_MAX_DIM = 2000;
export const SCREENSHOT_MAX_BYTES = 2_000_000;
export const SCREENSHOT_QUALITY = 80;
export const SCREENSHOT_SCROLL_DELAY_MS = 150;
export const SCREENSHOT_CAPTURE_DELAY_MS = 350;
export const SCREENSHOT_PROCESS_TIMEOUT_MS = 8000;

// Dependency contract — only the subset each function needs.
interface ScreenshotDeps {
  delay: (ms: number) => Promise<unknown>;
  executeWithTimeout: <T>(
    tabId: number,
    timeoutMs: number,
    func: (...args: Array<any>) => T,
    args?: Array<unknown>,
  ) => Promise<T | null>;
  isScriptableUrl: (url: unknown) => boolean;
  getTabSnapshot: () => Promise<{ generatedAt: number; windows: Array<Record<string, unknown>> }>;
  selectTabsByScope: (
    snapshot: { windows: Array<Record<string, unknown>> },
    params: Record<string, unknown>,
  ) => { tabs: Array<Record<string, unknown>>; error?: Record<string, unknown> };
  waitForTabReady: (tabId: number, params: Record<string, unknown>, fallbackTimeoutMs: number) => Promise<void>;
  sendProgress: (id: string, payload: Record<string, unknown>) => void;
}

export function estimateDataUrlBytes(dataUrl: string) {
  const commaIndex = dataUrl.indexOf(",");
  if (commaIndex < 0) {
    return dataUrl.length;
  }
  const base64 = dataUrl.slice(commaIndex + 1);
  const padding = base64.endsWith("==") ? 2 : base64.endsWith("=") ? 1 : 0;
  return Math.max(0, Math.floor((base64.length * 3) / 4) - padding);
}

export function arrayBufferToBase64(buffer: ArrayBuffer) {
  const bytes = new Uint8Array(buffer);
  let binary = "";
  for (let i = 0; i < bytes.length; i += 1) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary);
}

export async function resizeDataUrl(
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

export async function resizeDataUrlToMaxDim(
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

export async function cropDataUrl(
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

export async function ensureMaxBytes(
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

export async function constrainDataUrl(
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

export async function captureVisible(windowId: number, format: "png" | "jpeg", quality: number) {
  const options: chrome.tabs.CaptureVisibleTabOptions = { format };
  if (format === "jpeg") {
    options.quality = quality;
  }
  return chrome.tabs.captureVisibleTab(windowId, options);
}

export async function getPageMetrics(
  tabId: number,
  timeoutMs: number,
  deps: Pick<ScreenshotDeps, "executeWithTimeout">,
) {
  const result = await deps.executeWithTimeout(tabId, timeoutMs, () => {
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

export async function scrollToPosition(
  tabId: number,
  timeoutMs: number,
  x: number,
  y: number,
  deps: Pick<ScreenshotDeps, "executeWithTimeout">,
) {
  const result = await deps.executeWithTimeout(tabId, timeoutMs, (scrollX: number, scrollY: number) => {
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

export async function captureTabTiles(
  tab: Record<string, unknown>,
  options: {
    mode: "viewport" | "full";
    format: "png" | "jpeg";
    quality: number;
    tileMaxDim: number;
    maxBytes: number;
  },
  deps: Pick<ScreenshotDeps, "delay" | "executeWithTimeout">,
): Promise<Array<Record<string, unknown>>> {
  const tabId = tab.tabId as number;
  const windowId = tab.windowId as number;
  if (!Number.isFinite(tabId) || !Number.isFinite(windowId)) {
    throw new Error("Missing tab/window id");
  }

  const metrics = await getPageMetrics(tabId, SCREENSHOT_PROCESS_TIMEOUT_MS, deps);
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
      await deps.delay(SCREENSHOT_CAPTURE_DELAY_MS);
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
      await scrollToPosition(tabId, SCREENSHOT_PROCESS_TIMEOUT_MS, x, y, deps);
      await deps.delay(SCREENSHOT_SCROLL_DELAY_MS);
      const width = Math.min(stepX, maxX - x);
      const height = Math.min(stepY, maxY - y);
      try {
        await captureTile(x, y, width, height, tileCount);
      } catch (err) {
        const message = err instanceof Error ? err.message : "capture_failed";
        if (message.includes("MAX_CAPTURE_VISIBLE_TAB_CALLS_PER_SECOND")) {
          await deps.delay(1000);
          await captureTile(x, y, width, height, tileCount);
        } else {
          throw err;
        }
      }
    }
  }

  await scrollToPosition(tabId, SCREENSHOT_PROCESS_TIMEOUT_MS, startScrollX, startScrollY, deps);
  return tiles;
}

export async function screenshotTabs(
  params: Record<string, unknown>,
  requestId: string,
  deps: Pick<ScreenshotDeps, "delay" | "executeWithTimeout" | "isScriptableUrl" | "getTabSnapshot" | "selectTabsByScope" | "waitForTabReady" | "sendProgress">,
) {
  const snapshot = await deps.getTabSnapshot();
  const selection = deps.selectTabsByScope(snapshot, params) as { tabs: Array<Record<string, unknown>>; error?: Record<string, unknown> };
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

  for (let index = 0; index < tabs.length; index += 1) {
    const tab = tabs[index];
    const tabId = tab.tabId as number;
    const url = tab.url as string | undefined;
    if (!deps.isScriptableUrl(url)) {
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
        deps.sendProgress(requestId, { phase: "screenshot", processed: index + 1, total: tabs.length, tabId });
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
        await deps.delay(SCREENSHOT_SCROLL_DELAY_MS);
      }

      try {
        await deps.waitForTabReady(tabId, params, SCREENSHOT_PROCESS_TIMEOUT_MS);
        tiles = await captureTabTiles(tab, { mode, format, quality, tileMaxDim: adjustedTileMaxDim, maxBytes: adjustedMaxBytes }, deps);
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
      deps.sendProgress(requestId, { phase: "screenshot", processed: index + 1, total: tabs.length, tabId });
    }
  }

  const response: Record<string, unknown> = {
    totals: { tabs: tabs.length, tiles: totalTiles },
    entries,
  };
  return response;
}
