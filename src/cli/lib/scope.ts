import { errorOut } from "./output";
import { buildWindowLabelIndex, listGroupSummaries } from "./snapshot";
import type { Options, ScopeFlags, ScopeParams, SelectionResult } from "./types";

export function formatCliArgValue(value: unknown): string {
  const raw = String(value);
  if (!raw) {
    return raw;
  }
  if (/[\s"]/g.test(raw)) {
    const escaped = raw.replace(/"/g, '\\"');
    return `"${escaped}"`;
  }
  return raw;
}

export function buildScopeArgs(options: Options, includeAll: boolean): string[] {
  const args: string[] = [];
  if (includeAll) {
    args.push("--all");
    return args;
  }
  if (options.ungrouped === true) {
    args.push("--ungrouped");
  }
  if (Array.isArray(options.tab)) {
    for (const entry of options.tab as Array<unknown>) {
      if (typeof entry === "string" && entry.trim()) {
        args.push("--tab", formatCliArgValue(entry.trim()));
      }
    }
  }
  if (typeof options.group === "string" && options.group.trim()) {
    args.push("--group", formatCliArgValue(options.group.trim()));
  }
  if (options["group-id"] != null && options.ungrouped !== true) {
    args.push("--group-id", formatCliArgValue(options["group-id"]));
  }
  if (options.window != null) {
    const windowValue = normalizeWindowScope(options.window);
    args.push("--window", formatCliArgValue(windowValue));
  }
  return args;
}

export function resolveScopeFlags(options: Options): ScopeFlags {
  const tabIds = Array.isArray(options.tab)
    ? (options.tab as Array<unknown>).map(Number).filter(Number.isFinite)
    : [];
  const groupTitle = typeof options.group === "string" ? options.group.trim() : "";
  const ungrouped = options.ungrouped === true;
  const groupId = ungrouped ? -1 : (options["group-id"] != null ? Number(options["group-id"]) : null);
  const normalizedWindow = options.window != null ? normalizeWindowScope(options.window) : null;
  const windowId = normalizedWindow == null
    ? null
    : normalizedWindow;
  
  if (options["group-id"] != null && !Number.isFinite(groupId)) {
    errorOut("Invalid --group-id value");
  }
  if (ungrouped && options["group-id"] != null) {
    errorOut("--ungrouped cannot be combined with --group-id");
  }
  if (options.window != null && typeof windowId === "number" && !Number.isFinite(windowId)) {
    errorOut("Invalid --window value");
  }
  
  const hasScope = tabIds.length > 0
    || Boolean(groupTitle)
    || Number.isFinite(groupId)
    || (typeof windowId === "number" && Number.isFinite(windowId))
    || (typeof windowId === "string" && windowId.length > 0);
    
  return { tabIds, groupTitle, groupId, windowId, hasScope, ungrouped };
}

function normalizeWindowScope(value: unknown): string | number {
  if (typeof value === "string") {
    const trimmed = value.trim().toLowerCase();
    if (trimmed === "active") {
      return "active";
    }
    if (trimmed === "last-focused" || trimmed === "lastfocused") {
      return "last-focused";
    }
    if (trimmed === "new") {
      errorOut("--window new is only supported by open");
    }
    const numeric = Number(trimmed);
    if (Number.isFinite(numeric)) {
      return numeric;
    }
    errorOut("Invalid --window value");
  }
  return typeof value === "number" ? value : String(value);
}

export function extractScopeParams(options: Options): ScopeParams {
  const scope = resolveScopeFlags(options);
  return {
    tabIds: scope.tabIds.length ? scope.tabIds : undefined,
    groupTitle: scope.groupTitle || undefined,
    groupId: scope.groupId != null ? scope.groupId : undefined,
    windowId: scope.windowId != null ? scope.windowId : undefined,
    all: options.all === true,
  };
}

export function selectTabsFromSnapshot(
  snapshot: Record<string, unknown>,
  params: Record<string, unknown>
): SelectionResult {
  const windows = (snapshot.windows as Array<Record<string, unknown>>) || [];
  const allTabs = windows.flatMap((win) => (win.tabs as Array<Record<string, unknown>>) || []);

  if (params.tabIds && (params.tabIds as Array<number>).length) {
    const idSet = new Set((params.tabIds as Array<number>).map(Number));
    return { tabs: allTabs.filter((tab) => idSet.has(tab.tabId as number)) };
  }

  if (params.groupId != null) {
    const groupId = Number(params.groupId);
    return { tabs: allTabs.filter((tab) => tab.groupId === groupId) };
  }

  if (params.groupTitle) {
    const windowLabels = buildWindowLabelIndex(snapshot);
    const matches: Array<{ windowId: number; groupId: number; windowLabel: string | null }> = [];
    for (const win of windows) {
      const groups = (win.groups as Array<Record<string, unknown>>) || [];
      for (const group of groups) {
        if (group.title === params.groupTitle) {
          matches.push({
            windowId: win.windowId as number,
            windowLabel: windowLabels.get(win.windowId as number) ?? null,
            groupId: group.groupId as number,
          });
        }
      }
    }

    const availableGroups = listGroupSummaries(snapshot, windowLabels);

    if (matches.length === 0) {
      return {
        tabs: [],
        error: {
          message: "No matching group title found",
          hint: "Use tabctl group-list to see existing groups.",
          availableGroups,
        },
      };
    }

    if (matches.length > 1 && !params.windowId) {
      return {
        tabs: [],
        error: {
          message: "Group title is ambiguous. Provide a windowId.",
          hint: "Use --window to disambiguate group titles.",
          matches,
          availableGroups,
        },
      };
    }

    const resolvedWindowId = params.windowId != null
      ? resolveWindowIdFromSnapshot(snapshot, params.windowId)
      : null;
    const target = resolvedWindowId != null
      ? matches.find((match) => match.windowId === resolvedWindowId)
      : matches[0];

    if (!target) {
      return {
        tabs: [],
        error: {
          message: "Group title not found in specified window",
          hint: "Use tabctl group-list to see existing groups.",
          matches,
          availableGroups,
        },
      };
    }

    return { tabs: allTabs.filter((tab) => tab.groupId === target.groupId && tab.windowId === target.windowId) };
  }

  if (params.windowId != null) {
    const windowId = resolveWindowIdFromSnapshot(snapshot, params.windowId);
    if (!Number.isFinite(windowId)) {
      return { tabs: [] };
    }
    return { tabs: allTabs.filter((tab) => tab.windowId === windowId) };
  }

  if (params.all) {
    return { tabs: allTabs };
  }

  const focused = windows.find((win) => win.focused);
  return { tabs: focused ? ((focused.tabs as Array<Record<string, unknown>>) || []) : [] };
}

export function resolveWindowIdFromSnapshot(snapshot: Record<string, unknown>, value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) {
    return value;
  }
  const windows = (snapshot.windows as Array<Record<string, unknown>>) || [];
  if (typeof value === "string") {
    const normalized = value.trim().toLowerCase();
    if (normalized === "active") {
      const focused = windows.find((win) => win.focused === true);
      return typeof focused?.windowId === "number" ? focused.windowId : null;
    }
    if (normalized === "last-focused") {
      let bestWindowId: number | null = null;
      let bestFocusedAt = -Infinity;
      for (const win of windows) {
        const tabs = (win.tabs as Array<Record<string, unknown>>) || [];
        for (const tab of tabs) {
          const focusedAt = Number(tab.lastFocusedAt);
          if (!Number.isFinite(focusedAt)) {
            continue;
          }
          if (focusedAt > bestFocusedAt) {
            bestFocusedAt = focusedAt;
            bestWindowId = typeof win.windowId === "number" ? win.windowId : null;
          }
        }
      }
      return bestWindowId;
    }
  }
  return Number.isFinite(Number(value)) ? Number(value) : null;
}

export function filterGroupsByScope(
  groups: Array<Record<string, unknown>>,
  scope: ScopeFlags,
  snapshot: Record<string, unknown> | null,
  buildTabIndex: (snap: Record<string, unknown>) => Map<number, Record<string, unknown>>
): Array<Record<string, unknown>> {
  let filtered = groups;
  const allScope = !scope.hasScope;
  
  if (!allScope) {
    if (typeof scope.windowId === "number" && Number.isFinite(scope.windowId)) {
      filtered = filtered.filter((group) => group.windowId === scope.windowId);
    }
    if (Number.isFinite(scope.groupId)) {
      filtered = filtered.filter((group) => group.groupId === scope.groupId);
    }
    if (scope.groupTitle) {
      filtered = filtered.filter((group) => group.title === scope.groupTitle);
    }
    if (scope.tabIds.length > 0 && snapshot) {
      const tabIndex = buildTabIndex(snapshot);
      const groupIds = new Set<number>();
      for (const tabId of scope.tabIds) {
        const tab = tabIndex.get(tabId);
        if (!tab) {
          continue;
        }
        const groupId = tab.groupId as number;
        if (Number.isFinite(groupId) && groupId !== -1) {
          groupIds.add(groupId);
        }
      }
      filtered = filtered.filter((group) => groupIds.has(group.groupId as number));
    }
  }
  
  return filtered;
}
