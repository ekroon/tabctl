import { errorOut } from "./output";
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
    args.push("--window", formatCliArgValue(options.window));
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
  const windowId = options.window != null ? Number(options.window) : null;
  
  if (options["group-id"] != null && !Number.isFinite(groupId)) {
    errorOut("Invalid --group-id value");
  }
  if (ungrouped && options["group-id"] != null) {
    errorOut("--ungrouped cannot be combined with --group-id");
  }
  if (options.window != null && !Number.isFinite(windowId)) {
    errorOut("Invalid --window value");
  }
  
  const hasScope = tabIds.length > 0
    || Boolean(groupTitle)
    || Number.isFinite(groupId)
    || Number.isFinite(windowId);
    
  return { tabIds, groupTitle, groupId, windowId, hasScope, ungrouped };
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

// Helper to build window label index (needed by selectTabsFromSnapshot)
function buildWindowLabelIndex(snapshot: Record<string, unknown>): Map<number, string> {
  const windowLabels = new Map<number, string>();
  const windows = (snapshot.windows as Array<Record<string, unknown>>) || [];
  windows.forEach((win, index) => {
    const windowId = win.windowId as number;
    if (typeof windowId === "number") {
      windowLabels.set(windowId, `W${index + 1}`);
    }
  });
  return windowLabels;
}

// Helper to list group summaries (needed by selectTabsFromSnapshot)
function listGroupSummaries(
  snapshot: Record<string, unknown>,
  windowLabels: Map<number, string>
): Array<Record<string, unknown>> {
  const windows = (snapshot.windows as Array<Record<string, unknown>>) || [];
  const summaries: Array<Record<string, unknown>> = [];
  for (const win of windows) {
    const groups = (win.groups as Array<Record<string, unknown>>) || [];
    for (const group of groups) {
      summaries.push({
        windowId: win.windowId,
        windowLabel: windowLabels.get(win.windowId as number) ?? null,
        groupId: group.groupId,
        title: typeof group.title === "string" ? group.title : null,
      });
    }
  }
  return summaries;
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

    const target = params.windowId
      ? matches.find((match) => match.windowId === Number(params.windowId))
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
    const windowId = Number(params.windowId);
    return { tabs: allTabs.filter((tab) => tab.windowId === windowId) };
  }

  if (params.all) {
    return { tabs: allTabs };
  }

  const focused = windows.find((win) => win.focused);
  return { tabs: focused ? ((focused.tabs as Array<Record<string, unknown>>) || []) : [] };
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
    if (Number.isFinite(scope.windowId)) {
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
