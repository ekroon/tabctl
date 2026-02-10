// Group management — extracted from background.ts (pure structural refactor).

export type WindowSnapshot = {
  windowId: number;
  focused: boolean;
  tabs: Array<Record<string, unknown>>;
  groups: Array<Record<string, unknown>>;
};

export type GroupMatch = {
  windowId: number;
  group: Record<string, unknown>;
  tabs: Array<Record<string, unknown>>;
};

export type GroupSummary = {
  windowId: number;
  windowLabel: string | null;
  groupId: number;
  title: string | null;
};

import type { ExtensionDeps } from "./deps";

export function getGroupTabs(windowSnapshot: WindowSnapshot, groupId: number) {
  return windowSnapshot.tabs
    .filter((tab) => tab.groupId === groupId)
    .sort((a, b) => (Number(a.index) || 0) - (Number(b.index) || 0));
}

export function listGroupSummaries(snapshot: { windows: Array<Record<string, unknown>> }, buildWindowLabels: (snapshot: { windows: Array<{ windowId: number }> }) => Map<number, string>, windowId?: number) {
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

export function summarizeGroupMatch(match: GroupMatch, windowLabels: Map<number, string>) {
  return {
    windowId: match.windowId,
    windowLabel: windowLabels.get(match.windowId) ?? null,
    groupId: match.group.groupId,
    title: typeof match.group.title === "string" ? match.group.title : null,
  };
}

export function findGroupMatches(snapshot: { windows: Array<Record<string, unknown>> }, groupTitle: string, windowId?: number) {
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

export function resolveGroupByTitle(snapshot: { windows: Array<Record<string, unknown>> }, buildWindowLabels: (snapshot: { windows: Array<{ windowId: number }> }) => Map<number, string>, groupTitle: string, windowId?: number) {
  const windowLabels = buildWindowLabels(snapshot as { windows: Array<{ windowId: number }> });
  const allMatches = findGroupMatches(snapshot, groupTitle);
  const matches = windowId ? allMatches.filter((match) => match.windowId === windowId) : allMatches;
  const availableGroups = listGroupSummaries(snapshot, buildWindowLabels);
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
        message: `Ambiguous group title: found ${matches.length} groups named "${groupTitle}". Use group-gather to merge duplicates, --group-id to target by ID, or --window to narrow scope.`,
        hint: "Use group-gather to merge duplicates, --group-id to target by ID, or --window to narrow scope.",
        matches: matches.map((match) => summarizeGroupMatch(match, windowLabels)),
        availableGroups,
      },
    };
  }
  return { match: matches[0] };
}

export function resolveGroupById(snapshot: { windows: Array<Record<string, unknown>> }, buildWindowLabels: (snapshot: { windows: Array<{ windowId: number }> }) => Map<number, string>, groupId: number) {
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
        availableGroups: listGroupSummaries(snapshot, buildWindowLabels),
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
        availableGroups: listGroupSummaries(snapshot, buildWindowLabels),
      },
    };
  }
  return { match: matches[0] };
}

export async function listGroups(params: Record<string, unknown>, deps: Pick<ExtensionDeps, "getTabSnapshot" | "buildWindowLabels" | "resolveWindowIdFromParams" | "log">) {
  const snapshot = await deps.getTabSnapshot();
  const windows = snapshot.windows as WindowSnapshot[];
  const windowLabels = deps.buildWindowLabels(snapshot as unknown as { windows: Array<{ windowId: number }> });
  const windowIdParam = params.windowId != null ? deps.resolveWindowIdFromParams(snapshot, params.windowId) : null;
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

export async function groupUpdate(params: Record<string, unknown>, deps: Pick<ExtensionDeps, "getTabSnapshot" | "buildWindowLabels" | "resolveWindowIdFromParams">) {
  const groupId = Number.isFinite(params.groupId as number) ? Number(params.groupId) : null;
  const groupTitle = typeof params.groupTitle === "string" ? params.groupTitle.trim() : "";
  if (!groupId && !groupTitle) {
    throw new Error("Missing group identifier");
  }

  const snapshot = await deps.getTabSnapshot();
  const windows = snapshot.windows as WindowSnapshot[];
  const windowIdParam = params.windowId != null ? deps.resolveWindowIdFromParams(snapshot, params.windowId) : null;
  if (windowIdParam && !windows.some((win) => win.windowId === windowIdParam)) {
    throw new Error("Window not found");
  }

  let match: GroupMatch;
  if (groupId != null) {
    const resolved = resolveGroupById(snapshot, deps.buildWindowLabels, groupId);
    if ((resolved as { error?: Record<string, unknown> }).error) {
      throw (resolved as { error: Record<string, unknown> }).error;
    }
    match = (resolved as { match: GroupMatch }).match;
    if (windowIdParam && windowIdParam !== match.windowId) {
      throw new Error("Group is not in the specified window");
    }
  } else {
    const resolved = resolveGroupByTitle(snapshot, deps.buildWindowLabels, groupTitle, windowIdParam || undefined);
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

export async function groupUngroup(params: Record<string, unknown>, deps: Pick<ExtensionDeps, "getTabSnapshot" | "buildWindowLabels" | "resolveWindowIdFromParams">) {
  const groupId = Number.isFinite(params.groupId as number) ? Number(params.groupId) : null;
  const groupTitle = typeof params.groupTitle === "string" ? params.groupTitle.trim() : "";
  if (!groupId && !groupTitle) {
    throw new Error("Missing group identifier");
  }

  const snapshot = await deps.getTabSnapshot();
  const windows = snapshot.windows as WindowSnapshot[];
  const windowIdParam = params.windowId != null ? deps.resolveWindowIdFromParams(snapshot, params.windowId) : null;
  if (windowIdParam && !windows.some((win) => win.windowId === windowIdParam)) {
    throw new Error("Window not found");
  }

  let match: GroupMatch;
  if (groupId != null) {
    const resolved = resolveGroupById(snapshot, deps.buildWindowLabels, groupId);
    if ((resolved as { error?: Record<string, unknown> }).error) {
      throw (resolved as { error: Record<string, unknown> }).error;
    }
    match = (resolved as { match: GroupMatch }).match;
    if (windowIdParam && windowIdParam !== match.windowId) {
      throw new Error("Group is not in the specified window");
    }
  } else {
    const resolved = resolveGroupByTitle(snapshot, deps.buildWindowLabels, groupTitle, windowIdParam || undefined);
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

export async function groupAssign(params: Record<string, unknown>, deps: Pick<ExtensionDeps, "getTabSnapshot" | "buildWindowLabels" | "resolveWindowIdFromParams" | "log">) {
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

  const snapshot = await deps.getTabSnapshot();
  const windows = snapshot.windows as WindowSnapshot[];
  const windowIdParam = params.windowId != null ? deps.resolveWindowIdFromParams(snapshot, params.windowId) : null;
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
    const resolved = resolveGroupById(snapshot, deps.buildWindowLabels, groupId);
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
    const resolved = resolveGroupByTitle(snapshot, deps.buildWindowLabels, groupTitle, windowIdParam || undefined);
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
        deps.log("Failed to update group", error);
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

export async function groupGather(params: Record<string, unknown>, deps: Pick<ExtensionDeps, "getTabSnapshot" | "buildWindowLabels" | "resolveWindowIdFromParams" | "log">) {
  const snapshot = await deps.getTabSnapshot();
  const windows = snapshot.windows as WindowSnapshot[];
  const windowIdParam = params.windowId != null ? deps.resolveWindowIdFromParams(snapshot, params.windowId) : null;

  if (windowIdParam && !windows.some((win) => win.windowId === windowIdParam)) {
    throw new Error("Window not found");
  }

  const groupTitleFilter = typeof params.groupTitle === "string" ? params.groupTitle.trim() : "";
  const merged: Array<Record<string, unknown>> = [];
  const undoEntries: Array<Record<string, unknown>> = [];

  for (const win of windows) {
    if (windowIdParam && win.windowId !== windowIdParam) continue;

    const byTitle = new Map<string, Array<Record<string, unknown>>>();
    for (const group of win.groups) {
      const title = typeof group.title === "string" ? group.title : "";
      if (!title) continue;
      if (groupTitleFilter && title !== groupTitleFilter) continue;
      if (!byTitle.has(title)) byTitle.set(title, []);
      byTitle.get(title)!.push(group);
    }

    for (const [title, titleGroups] of byTitle) {
      if (titleGroups.length < 2) continue;

      const groupsWithIndex = titleGroups.map((g) => {
        const tabs = win.tabs.filter((t) => t.groupId === g.groupId);
        const minIndex = Math.min(
          ...tabs.map((t) => {
            const idx = Number(t.index);
            return Number.isFinite(idx) ? idx : Infinity;
          }),
        );
        return { group: g, tabs, minIndex };
      });
      groupsWithIndex.sort((a, b) => a.minIndex - b.minIndex);

      const primary = groupsWithIndex[0];
      const duplicates = groupsWithIndex.slice(1);
      let movedTabs = 0;

      for (const dup of duplicates) {
        const tabIds = dup.tabs
          .map((t) => t.tabId)
          .filter((id): id is number => typeof id === "number");

        if (tabIds.length > 0) {
          for (const tab of dup.tabs) {
            undoEntries.push({
              tabId: tab.tabId,
              windowId: win.windowId,
              index: tab.index,
              groupId: tab.groupId,
              groupTitle: tab.groupTitle,
              groupColor: tab.groupColor,
              groupCollapsed: dup.group.collapsed ?? null,
            });
          }

          await chrome.tabs.group({ groupId: primary.group.groupId as number, tabIds });
          movedTabs += tabIds.length;
        }
      }

      merged.push({
        windowId: win.windowId,
        groupTitle: title,
        primaryGroupId: primary.group.groupId,
        mergedGroupCount: duplicates.length,
        movedTabs,
      });
    }
  }

  return {
    merged,
    summary: {
      mergedGroups: merged.reduce((sum, m) => sum + (m.mergedGroupCount as number), 0),
      movedTabs: merged.reduce((sum, m) => sum + (m.movedTabs as number), 0),
    },
    undo: {
      action: "group-gather",
      tabs: undoEntries,
    },
    txid: params.txid || null,
  };
}
