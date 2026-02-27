// Analysis/inspection pipeline — extracted from tabs.ts (pure structural refactor).

const content = require("./content") as typeof import("./content");
const { isScriptableUrl, extractPageMeta, extractSelectorSignal, waitForSettle, waitForTabReady } = content;
const tabs = require("./tabs") as typeof import("./tabs");
const { normalizeUrl } = tabs;

export const DEFAULT_STALE_DAYS = 30;
export const DESCRIPTION_MAX_LENGTH = 250;
export const SELECTOR_VALUE_MAX_LENGTH = 500;

import type { ExtensionDeps } from "./deps";

export async function analyzeTabs(params: Record<string, unknown>, requestId: string, deps: Pick<ExtensionDeps, "getTabSnapshot" | "selectTabsByScope" | "sendProgress">) {
  const staleDays = Number.isFinite(params.staleDays) ? params.staleDays : DEFAULT_STALE_DAYS;
  const progressEnabled = params.progress === true;
  const snapshot = await deps.getTabSnapshot();
  const selection = deps.selectTabsByScope(snapshot, params) as { tabs: Array<Record<string, unknown>>; error?: Record<string, unknown> };
  if (selection.error) {
    throw selection.error;
  }
  const selectedTabs = selection.tabs;
  const scopeTabs = selectedTabs;
  const now = Date.now();

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

  const candidates = Array.from(candidateMap.values()).map((entry) => {
    const reasons = entry.reasons;
    const severity = reasons.some((reason) => reason.type === "duplicate")
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

  const response: Record<string, unknown> = {
    staleDays,
    totals: {
      tabs: scopeTabs.length,
      analyzed: selectedTabs.length,
      candidates: candidates.length,
    },
    candidates,
  };
  return response;
}

export async function inspectTabs(params: Record<string, unknown>, requestId: string, deps: Pick<ExtensionDeps, "getTabSnapshot" | "selectTabsByScope" | "sendProgress">) {
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

  const snapshot = await deps.getTabSnapshot();
  const selection = deps.selectTabsByScope(snapshot, params) as { tabs: Array<Record<string, unknown>>; error?: Record<string, unknown> };
  if (selection.error) {
    throw selection.error;
  }

  const tabs = selection.tabs;
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

  const signalDefs: Array<{ id: string; match: (tab: Record<string, unknown>) => boolean; run: (tabId: number) => Promise<unknown> }> = [];
  for (const signalId of signalList) {
    if (signalId === "page-meta") {
      signalDefs.push({
        id: signalId,
        match: (tab) => isScriptableUrl(tab.url),
        run: async (tabId) => extractPageMeta(tabId, signalTimeoutMs, DESCRIPTION_MAX_LENGTH),
      });
    } else if (signalId === "selector") {
      signalDefs.push({
        id: signalId,
        match: (tab) => isScriptableUrl(tab.url),
        run: async (tabId) => extractSelectorSignal(tabId, normalizedSelectors, signalTimeoutMs, SELECTOR_VALUE_MAX_LENGTH),
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
      try {
        await waitForTabReady(tabId, params, signalTimeoutMs);
        result = await task.signal.run(tabId);
      } catch (err) {
        const message = err instanceof Error ? err.message : "signal_error";
        error = message;
      }
      const entry = entryMap.get(tabId) || { tab: task.tab, signals: {} };
      entry.signals[task.signal.id] = {
        ok: error === null,
        data: result,
        error,
      };
      entryMap.set(tabId, entry);

      completedTasks += 1;
      if (progressEnabled) {
        deps.sendProgress(requestId, {
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

  const response: Record<string, unknown> = {
    totals: {
      tabs: tabs.length,
      signals: signalDefs.length,
      tasks: totalTasks,
    },
    entries,
  };
  return response;
}
