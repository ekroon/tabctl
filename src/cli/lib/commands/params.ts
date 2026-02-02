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

  return {
    staleDays: options["stale-days"] ? Number(options["stale-days"]) : undefined,
    checkGitHub: Boolean(options.github),
    tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
    groupTitle: options.group,
    groupId: options.ungrouped ? -1 : (options["group-id"] ? Number(options["group-id"]) : undefined),
    windowId: options.window ? Number(options.window) : undefined,
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

  let selectorSpecs: Array<Record<string, unknown>> | undefined;
  if (options.selector) {
    selectorSpecs = (options.selector as string[]).map((value) => {
      const trimmed = value.trim();
      if (trimmed.startsWith("{")) {
        try {
          return JSON.parse(trimmed) as Record<string, unknown>;
        } catch {
          errorOut(`Invalid selector JSON: ${trimmed}`);
        }
      }
      if (trimmed.includes("=")) {
        const [name, selector] = trimmed.split(/=(.+)/);
        return { name, selector };
      }
      return { selector: trimmed };
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
    windowId: options.window ? Number(options.window) : undefined,
    groupTitle: options.group,
    groupId: options.ungrouped ? -1 : (options["group-id"] ? Number(options["group-id"]) : undefined),
    tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
    signals: options.signal ? (options.signal as string[]) : undefined,
    selectorSpecs,
    signalConfig,
    signalConcurrency: options["signal-concurrency"] ? Number(options["signal-concurrency"]) : undefined,
    signalTimeoutMs: options["signal-timeout-ms"] ? Number(options["signal-timeout-ms"]) : undefined,
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
  return {
    urls: options.url ? (options.url as string[]).map(String) : undefined,
    groupTitle: options.group,
    color: normalizeGroupColor(options.color),
    afterGroupTitle: options["after-group"],
    windowId: options.window ? Number(options.window) : undefined,
    newWindow: options["new-window"] === true,
    windowGroupTitle: options["window-group"],
    windowTabId: options["window-tab"] ? Number(options["window-tab"]) : undefined,
    windowUrl: options["window-url"],
  };
}

// ============================================================================
// Group Command Parameters
// ============================================================================

export function buildGroupUpdateParams(options: Options): Record<string, unknown> {
  return {
    groupTitle: options.group,
    groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
    windowId: options.window ? Number(options.window) : undefined,
    title: options.title,
    color: options.color,
    collapsed: options.collapsed === true ? true : options.expanded === true ? false : undefined,
  };
}

export function buildGroupUngroupParams(options: Options): Record<string, unknown> {
  return {
    groupTitle: options.group,
    groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
    windowId: options.window ? Number(options.window) : undefined,
  };
}

export function buildGroupAssignParams(options: Options): Record<string, unknown> {
  return {
    tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
    groupTitle: options.group,
    groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
    windowId: options.window ? Number(options.window) : undefined,
    create: Boolean(options.create),
    color: options.color,
    collapsed: options.collapsed === true ? true : options.expanded === true ? false : undefined,
  };
}

// ============================================================================
// Move Command Parameters
// ============================================================================

export function buildMoveTabParams(options: Options): Record<string, unknown> {
  return {
    tabId: options.tab ? Number((options.tab as string[])[0]) : undefined,
    tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
    beforeTabId: options["before-tab"] ? Number(options["before-tab"]) : undefined,
    afterTabId: options["after-tab"] ? Number(options["after-tab"]) : undefined,
    beforeGroupTitle: options["before-group"],
    afterGroupTitle: options["after-group"],
    windowId: options.window ? Number(options.window) : undefined,
    newWindow: options["new-window"] === true,
  };
}

export function buildMoveGroupParams(options: Options): Record<string, unknown> {
  return {
    groupTitle: options.group,
    groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
    beforeTabId: options["before-tab"] ? Number(options["before-tab"]) : undefined,
    afterTabId: options["after-tab"] ? Number(options["after-tab"]) : undefined,
    beforeGroupTitle: options["before-group"],
    afterGroupTitle: options["after-group"],
    windowId: options.window ? Number(options.window) : undefined,
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

  return {
    all: Boolean(options.all),
    windowId: options.window ? Number(options.window) : undefined,
    groupTitle: options.group,
    groupId: options.ungrouped ? -1 : (options["group-id"] ? Number(options["group-id"]) : undefined),
    tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
  };
}

export function buildCloseParams(options: Options): Record<string, unknown> {
  if (options.ungrouped && options["group-id"]) {
    errorOut("--ungrouped cannot be combined with --group-id");
  }

  if (options.apply) {
    return { mode: "apply", analysisId: options.apply };
  }

  if (!options.confirm) {
    errorOut("Direct close requires --confirm");
  }

  return {
    mode: "direct",
    confirmed: true,
    windowId: options.window ? Number(options.window) : undefined,
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

  return {
    all: Boolean(options.all),
    windowId: options.window ? Number(options.window) : undefined,
    groupTitle: options.group,
    groupId: options.ungrouped ? -1 : (options["group-id"] ? Number(options["group-id"]) : undefined),
    tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
  };
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
