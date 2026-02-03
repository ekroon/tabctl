/**
 * Command parameter builders.
 * These functions extract and validate command parameters from CLI options.
 */

import fs from "fs";
import { errorOut } from "../output";
import { normalizeGroupColor } from "../args";
import type { Options } from "../types";

// ============================================================================
// Analyze Command Parameters
// ============================================================================

export function buildAnalyzeParams(options: Options): Record<string, unknown> {
  if (options.ungrouped && options["group-id"]) {
    errorOut("--ungrouped cannot be combined with --group-id");
  }
  const windowValue = parseWindowScope(options.window, { allowNew: false });

  return {
    staleDays: options["stale-days"] ? Number(options["stale-days"]) : undefined,
    checkGitHub: Boolean(options.github),
    tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
    groupTitle: options.group,
    groupId: options.ungrouped ? -1 : (options["group-id"] ? Number(options["group-id"]) : undefined),
    windowId: windowValue,
    all: options.all === true,
    githubConcurrency: options["github-concurrency"] ? Number(options["github-concurrency"]) : undefined,
    githubTimeoutMs: options["github-timeout-ms"] ? Number(options["github-timeout-ms"]) : undefined,
    progress: Boolean(options.progress),
  };
}

// ============================================================================
// Inspect Command Parameters
// ============================================================================

export function buildInspectParams(options: Options): Record<string, unknown> {
  if (options.ungrouped && options["group-id"]) {
    errorOut("--ungrouped cannot be combined with --group-id");
  }
  const windowValue = parseWindowScope(options.window, { allowNew: false });

  const selectorAttr = options["selector-attr"] ? String(options["selector-attr"]).trim() : "";
  const allowedSelectorAttrs = new Set(["text", "href", "src", "href-url", "src-url"]);
  if (options["selector-attr"] && (!selectorAttr || !allowedSelectorAttrs.has(selectorAttr))) {
    errorOut("Invalid --selector-attr value (use text|href|src|href-url|src-url)");
  }

  let selectorSpecs: Array<Record<string, unknown>> | undefined;
  if (options.selector) {
    selectorSpecs = (options.selector as string[]).map((value) => {
      const trimmed = value.trim();
      if (trimmed.startsWith("{")) {
        try {
          const parsed = JSON.parse(trimmed) as Record<string, unknown>;
          if (parsed && typeof parsed.selector === "string" && parsed.selector.includes(":contains(")) {
            errorOut("Selector :contains() is not supported; use text filters instead.");
          }
          if (selectorAttr && parsed && typeof parsed === "object" && !Object.prototype.hasOwnProperty.call(parsed, "attr")) {
            return { ...parsed, attr: selectorAttr };
          }
          return parsed;
        } catch {
          errorOut(`Invalid selector JSON: ${trimmed}`);
        }
      }
      if (trimmed.includes(":contains(")) {
        errorOut("Selector :contains() is not supported; use text filters instead.");
      }
      if (trimmed.includes("=")) {
        const [name, selector] = trimmed.split(/=(.+)/);
        return selectorAttr ? { name, selector, attr: selectorAttr } : { name, selector };
      }
      return selectorAttr ? { selector: trimmed, attr: selectorAttr } : { selector: trimmed };
    }).filter(Boolean) as Array<Record<string, unknown>>;
  }

  let signalConfig: Record<string, unknown> | undefined;
  if (options["signal-config"]) {
    try {
      const configRaw = fs.readFileSync(String(options["signal-config"]), "utf8");
      signalConfig = JSON.parse(configRaw) as Record<string, unknown>;
    } catch {
      errorOut("Failed to read --signal-config file");
    }
  }

  return {
    all: Boolean(options.all),
    windowId: windowValue,
    groupTitle: options.group,
    groupId: options.ungrouped ? -1 : (options["group-id"] ? Number(options["group-id"]) : undefined),
    tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
    signals: options.signal ? (options.signal as string[]) : undefined,
    selectorSpecs,
    signalConfig,
    signalConcurrency: options["signal-concurrency"] ? Number(options["signal-concurrency"]) : undefined,
    signalTimeoutMs: options["signal-timeout-ms"] ? Number(options["signal-timeout-ms"]) : undefined,
    waitFor: parseWaitFor(options["wait-for"]),
    waitTimeoutMs: parseWaitTimeout(options["wait-timeout-ms"]),
    progress: Boolean(options.progress),
  };
}

// ============================================================================
// Focus/Refresh Command Parameters
// ============================================================================

export function buildFocusParams(options: Options): Record<string, unknown> {
  return {
    tabId: options.tab ? Number((options.tab as string[])[0]) : undefined,
    tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
  };
}

export function buildRefreshParams(options: Options): Record<string, unknown> {
  return {
    tabId: options.tab ? Number((options.tab as string[])[0]) : undefined,
    tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
  };
}

// ============================================================================
// Open Command Parameters
// ============================================================================

export function buildOpenParams(options: Options): Record<string, unknown> {
  const windowValue = parseWindowScope(options.window, { allowNew: true });
  const openNewWindow = options["new-window"] === true || windowValue === "new";
  if (options["before-tab"] != null && !Number.isFinite(Number(options["before-tab"]))) {
    errorOut("Invalid --before-tab value");
  }
  if (options["after-tab"] != null && !Number.isFinite(Number(options["after-tab"]))) {
    errorOut("Invalid --after-tab value");
  }
  if (options["before-tab"] != null && options["after-tab"] != null) {
    errorOut("Only one target position is allowed");
  }

  return {
    urls: options.url ? (options.url as string[]).map(String) : undefined,
    groupTitle: options.group,
    color: normalizeGroupColor(options.color),
    afterGroupTitle: options["after-group"],
    beforeTabId: options["before-tab"] ? Number(options["before-tab"]) : undefined,
    afterTabId: options["after-tab"] ? Number(options["after-tab"]) : undefined,
    windowId: windowValue === "new" ? undefined : windowValue,
    newWindow: openNewWindow,
    windowGroupTitle: options["window-group"],
    windowTabId: options["window-tab"] ? Number(options["window-tab"]) : undefined,
    windowUrl: options["window-url"],
  };
}

// ============================================================================
// Group Command Parameters
// ============================================================================

export function buildGroupUpdateParams(options: Options): Record<string, unknown> {
  const windowValue = parseWindowScope(options.window, { allowNew: false });
  return {
    groupTitle: options.group,
    groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
    windowId: windowValue,
    title: options.title,
    color: options.color,
    collapsed: options.collapsed === true ? true : options.expanded === true ? false : undefined,
  };
}

export function buildGroupUngroupParams(options: Options): Record<string, unknown> {
  const windowValue = parseWindowScope(options.window, { allowNew: false });
  return {
    groupTitle: options.group,
    groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
    windowId: windowValue,
  };
}

export function buildGroupAssignParams(options: Options): Record<string, unknown> {
  const windowValue = parseWindowScope(options.window, { allowNew: false });
  return {
    tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
    groupTitle: options.group,
    groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
    windowId: windowValue,
    create: Boolean(options.create),
    color: options.color,
    collapsed: options.collapsed === true ? true : options.expanded === true ? false : undefined,
  };
}

// ============================================================================
// Move Command Parameters
// ============================================================================

export function buildMoveTabParams(options: Options): Record<string, unknown> {
  const windowValue = parseWindowScope(options.window, { allowNew: false });
  return {
    tabId: options.tab ? Number((options.tab as string[])[0]) : undefined,
    tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
    beforeTabId: options["before-tab"] ? Number(options["before-tab"]) : undefined,
    afterTabId: options["after-tab"] ? Number(options["after-tab"]) : undefined,
    beforeGroupTitle: options["before-group"],
    afterGroupTitle: options["after-group"],
    windowId: windowValue,
    newWindow: options["new-window"] === true,
  };
}

export function buildMoveGroupParams(options: Options): Record<string, unknown> {
  const windowValue = parseWindowScope(options.window, { allowNew: false });
  return {
    groupTitle: options.group,
    groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
    beforeTabId: options["before-tab"] ? Number(options["before-tab"]) : undefined,
    afterTabId: options["after-tab"] ? Number(options["after-tab"]) : undefined,
    beforeGroupTitle: options["before-group"],
    afterGroupTitle: options["after-group"],
    windowId: windowValue,
    newWindow: options["new-window"] === true,
  };
}

export function buildMergeWindowParams(options: Options): Record<string, unknown> {
  return {
    fromWindowId: options.from ? Number(options.from) : undefined,
    toWindowId: options.to ? Number(options.to) : undefined,
    windowId: options.from ? Number(options.from) : undefined,
    closeSource: options["close-source"] === true,
    confirmed: options.confirm === true,
  };
}

// ============================================================================
// Archive/Close Command Parameters
// ============================================================================

export function buildArchiveParams(options: Options): Record<string, unknown> {
  if (options.ungrouped && options["group-id"]) {
    errorOut("--ungrouped cannot be combined with --group-id");
  }
  const windowValue = parseWindowScope(options.window, { allowNew: false });

  return {
    all: Boolean(options.all),
    windowId: windowValue,
    groupTitle: options.group,
    groupId: options.ungrouped ? -1 : (options["group-id"] ? Number(options["group-id"]) : undefined),
    tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
  };
}

export function buildCloseParams(options: Options): Record<string, unknown> {
  if (options.ungrouped && options["group-id"]) {
    errorOut("--ungrouped cannot be combined with --group-id");
  }
  const windowValue = parseWindowScope(options.window, { allowNew: false });

  if (options.apply) {
    return { mode: "apply", analysisId: options.apply };
  }

  if (!options.confirm) {
    errorOut("Direct close requires --confirm");
  }

  return {
    mode: "direct",
    confirmed: true,
    windowId: windowValue,
    groupTitle: options.group,
    groupId: options.ungrouped ? -1 : (options["group-id"] ? Number(options["group-id"]) : undefined),
    tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
  };
}

// ============================================================================
// Report Command Parameters
// ============================================================================

export function buildReportParams(options: Options): Record<string, unknown> {
  if (options.ungrouped && options["group-id"]) {
    errorOut("--ungrouped cannot be combined with --group-id");
  }
  const windowValue = parseWindowScope(options.window, { allowNew: false });

  return {
    all: Boolean(options.all),
    windowId: windowValue,
    groupTitle: options.group,
    groupId: options.ungrouped ? -1 : (options["group-id"] ? Number(options["group-id"]) : undefined),
    tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
  };
}

// ============================================================================
// Screenshot Command Parameters
// ============================================================================

export function buildScreenshotParams(options: Options): Record<string, unknown> {
  if (options.ungrouped && options["group-id"]) {
    errorOut("--ungrouped cannot be combined with --group-id");
  }
  const windowValue = parseWindowScope(options.window, { allowNew: false });

  const outDir = options.out != null ? String(options.out).trim() : "";
  if (options.out && !outDir) {
    errorOut("--out requires a directory path");
  }

  return {
    all: Boolean(options.all),
    windowId: windowValue,
    groupTitle: options.group,
    groupId: options.ungrouped ? -1 : (options["group-id"] ? Number(options["group-id"]) : undefined),
    tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
    mode: options.mode,
    format: options.format,
    quality: options.quality != null ? Number(options.quality) : undefined,
    tileMaxDim: options["tile-max-dim"] != null ? Number(options["tile-max-dim"]) : undefined,
    maxBytes: options["max-bytes"] != null ? Number(options["max-bytes"]) : undefined,
    waitFor: parseWaitFor(options["wait-for"]),
    waitTimeoutMs: parseWaitTimeout(options["wait-timeout-ms"]),
    outDir: outDir || undefined,
    progress: Boolean(options.progress),
  };
}

function parseWaitFor(value: unknown): string | undefined {
  if (value == null) {
    return undefined;
  }
  const normalized = String(value).trim().toLowerCase();
  if (!normalized) {
    return undefined;
  }
  if (normalized !== "load" && normalized !== "dom" && normalized !== "none") {
    errorOut("Invalid --wait-for value (use load|dom|none)");
  }
  return normalized;
}

function parseWaitTimeout(value: unknown): number | undefined {
  if (value == null) {
    return undefined;
  }
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    errorOut("Invalid --wait-timeout-ms value");
  }
  return Math.floor(parsed);
}

function parseWindowScope(
  value: unknown,
  { allowNew }: { allowNew: boolean }
): number | string | undefined {
  if (value == null) {
    return undefined;
  }
  if (typeof value === "string") {
    const trimmed = value.trim().toLowerCase();
    if (!trimmed) {
      return undefined;
    }
    if (trimmed === "active" || trimmed === "last-focused") {
      return trimmed;
    }
    if (trimmed === "new") {
      if (!allowNew) {
        errorOut("--window new is only supported by open");
      }
      return trimmed;
    }
  }
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) {
    errorOut("Invalid --window value");
  }
  return numeric;
}

// ============================================================================
// History/Undo Command Parameters
// ============================================================================

export function buildHistoryParams(options: Options): Record<string, unknown> {
  return {
    limit: options.limit ? Number(options.limit) : undefined,
  };
}

export function buildUndoParams(options: Options): Record<string, unknown> {
  return {
    txid: options._[0] || options.txid,
    latest: options.latest === true,
  };
}
