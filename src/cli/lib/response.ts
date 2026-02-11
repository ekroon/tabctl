import fs from "node:fs";
import path from "node:path";
import { renderCsv, renderMarkdown } from "./report";
import { evaluateTab, type Policy } from "./policy";
import { printJson, errorOut } from "./output";
import { resolveScopeFlags, buildScopeArgs } from "./scope";
import { resolvePagination } from "./pagination";
import { buildTabIndex, buildWindowTitleIndex } from "./snapshot";
import type { Options } from "./types";

/**
 * Annotate inspect/report/screenshot entries with policy info and apply pagination.
 */
export function annotateEntries(
  data: Record<string, unknown>,
  options: Options,
  command: string,
  policyEnabled: boolean,
  policy: Policy | null,
  snapshot: Record<string, unknown> | null,
): void {
  const entries = data.entries as Array<Record<string, unknown>>;
  const tabIndex = snapshot ? buildTabIndex(snapshot) : null;
  const annotated = entries.map((entry) => {
    const tab = tabIndex?.get(entry.tabId as number) || entry;
    const { eligible, protectedReasons } = evaluateTab(tab, policy);
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

/**
 * Annotate analyze candidates with policy info and optional window titles.
 */
export function annotateCandidates(
  data: Record<string, unknown>,
  policy: Policy | null,
  includeWindowTitle: boolean,
  snapshot: Record<string, unknown> | null,
): void {
  const candidates = data.candidates as Array<Record<string, unknown>>;
  const tabIndex = snapshot ? buildTabIndex(snapshot) : null;
  const windowTitleIndex = snapshot && includeWindowTitle
    ? buildWindowTitleIndex(snapshot, policy)
    : null;
  data.candidates = candidates.map((candidate) => {
    const tab = tabIndex?.get(candidate.tabId as number) || candidate;
    const { eligible, protectedReasons } = evaluateTab(tab, policy);
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

/**
 * Build dedupe output from analyze response, optionally closing tabs.
 */
export function buildDedupeOutput(
  response: Record<string, unknown>,
  includeStale: boolean,
  closeData: Record<string, unknown> | null,
  confirmed: boolean,
): Record<string, unknown> {
  const data = (response.data as Record<string, unknown>) || {};
  const candidates = Array.isArray(data.candidates) ? (data.candidates as Array<Record<string, unknown>>) : [];
  const planned = candidates.filter((candidate) => {
    const reasons = Array.isArray(candidate.reasons) ? (candidate.reasons as Array<Record<string, unknown>>) : [];
    const hasDuplicate = reasons.some((reason) => reason.type === "duplicate");
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

  const closeSummary = closeData?.summary as Record<string, unknown> | undefined;
  const closedTabs = Number(closeSummary?.closedTabs ?? 0);
  const skippedTabs = Number(closeSummary?.skippedTabs ?? 0);
  return {
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
      nextCommand: confirmed
        ? null
        : (planTabIds.length > 0 && data.analysisId ? `tabctl close --apply ${data.analysisId} --confirm` : null),
      policy: data.policy,
      policyInfo: data.policyInfo,
    },
  };
}

/**
 * Extract dedupe plan tab IDs and expected URLs from analyze response.
 */
export function extractDedupePlan(
  response: Record<string, unknown>,
  includeStale: boolean,
): { planTabIds: number[]; expectedUrls: Record<string, string> } {
  const data = (response.data as Record<string, unknown>) || {};
  const candidates = Array.isArray(data.candidates) ? (data.candidates as Array<Record<string, unknown>>) : [];
  const planned = candidates.filter((candidate) => {
    const reasons = Array.isArray(candidate.reasons) ? (candidate.reasons as Array<Record<string, unknown>>) : [];
    const hasDuplicate = reasons.some((reason) => reason.type === "duplicate");
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

  return { planTabIds, expectedUrls };
}

/**
 * Format report output based on format option.
 */
export function formatReport(
  response: Record<string, unknown>,
  options: Options,
  prettyOutput: boolean,
): { printed: true } {
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
    return { printed: true };
  }

  if (format === "json") {
    printJson({ ok: true, data: { format, entries, ...(page ? { page } : {}) } }, prettyOutput);
    return { printed: true };
  }

  printJson({ ok: true, data: { format, entries, content, ...(page ? { page } : {}) } }, prettyOutput);
  return { printed: true };
}

/**
 * Write screenshot tiles to disk and return sanitized output.
 */
export function writeScreenshots(
  response: Record<string, unknown>,
  options: Options,
  prettyOutput: boolean,
): { printed: true } {
  const data = response.data as { entries?: Array<Record<string, unknown>> } | undefined;
  const entries = data?.entries || [];
  const page = data && "page" in data ? (data.page as Record<string, unknown> | undefined) : undefined;
  const outDir = options.out
    ? String(options.out)
    : path.join(process.cwd(), ".tabctl", "screenshots", String(Date.now()));
  fs.mkdirSync(outDir, { recursive: true });
  let filesWritten = 0;
  const sanitized = entries.map((entry) => {
    const tabId = entry.tabId as number | string | undefined;
    const tabDir = path.join(outDir, String(tabId ?? "unknown"));
    fs.mkdirSync(tabDir, { recursive: true });
    const tiles = Array.isArray(entry.tiles) ? (entry.tiles as Array<Record<string, unknown>>) : [];
    const sanitizedTiles = tiles.map((tile) => {
      const rawUrl = tile.dataUrl as string | undefined;
      const { dataUrl: _ignored, ...rest } = tile as Record<string, unknown>;
      if (!rawUrl) {
        return { ...rest, path: null, error: "missing_data" };
      }
      const match = rawUrl.match(/^data:(image\/png|image\/jpeg);base64,(.+)$/);
      if (!match) {
        return { ...rest, path: null, error: "invalid_data_url" };
      }
      const mime = match[1];
      const base64 = match[2];
      const ext = mime === "image/jpeg" ? "jpg" : "png";
      const index = Number.isFinite(tile.index as number) ? Number(tile.index) + 1 : filesWritten + 1;
      const total = Number.isFinite(tile.total as number) ? Number(tile.total) : null;
      const suffix = total && total > 1 ? `-of-${total}` : "";
      const filename = `screenshot-${index}${suffix}.${ext}`;
      const filePath = path.join(tabDir, filename);
      const buffer = Buffer.from(base64, "base64");
      fs.writeFileSync(filePath, buffer);
      filesWritten += 1;
      return {
        ...rest,
        path: filePath,
        bytes: buffer.length,
      };
    });
    return {
      ...entry,
      tiles: sanitizedTiles,
    };
  });
  printJson({ ok: true, data: { writtenTo: outDir, files: filesWritten, entries: sanitized, ...(page ? { page } : {}) } }, prettyOutput);
  return { printed: true };
}
