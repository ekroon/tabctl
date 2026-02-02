import { evaluateTab, annotateEntry, type Policy } from "./policy";

export function buildTabIndex(snapshot: Record<string, unknown>): Map<number, Record<string, unknown>> {
  const tabIndex = new Map<number, Record<string, unknown>>();
  const windows = (snapshot.windows as Array<Record<string, unknown>>) || [];
  for (const window of windows) {
    const tabs = (window.tabs as Array<Record<string, unknown>>) || [];
    for (const tab of tabs) {
      if (typeof tab.tabId === "number") {
        tabIndex.set(tab.tabId, tab);
      }
    }
  }
  return tabIndex;
}

export function buildWindowTitleIndex(
  snapshot: Record<string, unknown>,
  policy: Policy | null
): Map<number, string | null> {
  const windowTitleIndex = new Map<number, string | null>();
  const windows = (snapshot.windows as Array<Record<string, unknown>>) || [];
  for (const window of windows) {
    const windowId = window.windowId as number;
    if (typeof windowId !== "number") {
      continue;
    }
    const tabs = (window.tabs as Array<Record<string, unknown>>) || [];
    const activeTab = tabs.find((tab) => tab.active === true);
    if (!activeTab) {
      windowTitleIndex.set(windowId, null);
      continue;
    }
    const { eligible } = evaluateTab(activeTab, policy);
    if (!eligible) {
      windowTitleIndex.set(windowId, null);
      continue;
    }
    const title = typeof activeTab.title === "string" ? activeTab.title : null;
    windowTitleIndex.set(windowId, title);
  }
  return windowTitleIndex;
}

export function buildWindowLabelIndex(snapshot: Record<string, unknown>): Map<number, string> {
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

export function buildGroupsFromSnapshot(
  snapshot: Record<string, unknown>,
  windowId: number | null
): Array<Record<string, unknown>> {
  const windows = (snapshot.windows as Array<Record<string, unknown>>) || [];
  const windowLabels = buildWindowLabelIndex(snapshot);
  const groups: Array<Record<string, unknown>> = [];
  
  for (const win of windows) {
    const winId = win.windowId as number;
    if (!Number.isFinite(winId)) {
      continue;
    }
    if (Number.isFinite(windowId) && winId !== windowId) {
      continue;
    }
    const counts = new Map<number, number>();
    const tabs = (win.tabs as Array<Record<string, unknown>>) || [];
    for (const tab of tabs) {
      const groupId = tab.groupId as number;
      if (typeof groupId === "number" && groupId !== -1) {
        counts.set(groupId, (counts.get(groupId) || 0) + 1);
      }
    }
    const windowGroups = (win.groups as Array<Record<string, unknown>>) || [];
    for (const group of windowGroups) {
      const groupId = group.groupId as number;
      if (!Number.isFinite(groupId)) {
        continue;
      }
      groups.push({
        windowId: winId,
        windowLabel: windowLabels.get(winId) ?? null,
        groupId,
        title: group.title ?? null,
        color: group.color ?? null,
        collapsed: group.collapsed ?? null,
        tabCount: counts.get(groupId) || 0,
      });
    }
  }
  return groups;
}

export function listGroupSummaries(
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

export function compareTabIndex(
  a: Record<string, unknown>,
  b: Record<string, unknown>
): number {
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
  if (aIndex === bIndex) {
    const aId = Number(a.tabId);
    const bId = Number(b.tabId);
    if (Number.isFinite(aId) && Number.isFinite(bId)) {
      return aId - bId;
    }
    return 0;
  }
  return aIndex - bIndex;
}

export function orderTabs(
  snapshot: Record<string, unknown>,
  tabFilter: Set<number> | null
): Array<Record<string, unknown>> {
  const windows = (snapshot.windows as Array<Record<string, unknown>>) || [];
  const ordered: Array<Record<string, unknown>> = [];
  for (const win of windows) {
    const tabs = ((win.tabs as Array<Record<string, unknown>>) || []).slice().sort(compareTabIndex);
    for (const tab of tabs) {
      const tabId = tab.tabId as number;
      if (!tabFilter || tabFilter.has(tabId)) {
        ordered.push(tab);
      }
    }
  }
  return ordered;
}

export function buildPagedSnapshot(
  snapshot: Record<string, unknown>,
  tabs: Array<Record<string, unknown>>
): Record<string, unknown> {
  const tabsByWindow = new Map<number, Array<Record<string, unknown>>>();
  const groupsByWindow = new Map<number, Set<number>>();
  
  for (const tab of tabs) {
    const windowId = tab.windowId as number;
    if (!Number.isFinite(windowId)) {
      continue;
    }
    if (!tabsByWindow.has(windowId)) {
      tabsByWindow.set(windowId, []);
      groupsByWindow.set(windowId, new Set());
    }
    tabsByWindow.get(windowId)?.push(tab);
    const groupId = tab.groupId as number;
    if (Number.isFinite(groupId) && groupId !== -1) {
      groupsByWindow.get(windowId)?.add(groupId);
    }
  }

  const windows = (snapshot.windows as Array<Record<string, unknown>>) || [];
  const pagedWindows: Array<Record<string, unknown>> = [];
  
  for (const win of windows) {
    const windowId = win.windowId as number;
    const windowTabs = tabsByWindow.get(windowId) || [];
    if (windowTabs.length === 0) {
      continue;
    }
    const allowedGroupIds = groupsByWindow.get(windowId) || new Set<number>();
    const groups = ((win.groups as Array<Record<string, unknown>>) || []).filter(
      (group) => allowedGroupIds.has(group.groupId as number)
    );
    pagedWindows.push({
      ...win,
      tabs: windowTabs,
      groups,
    });
  }

  return {
    ...snapshot,
    windows: pagedWindows,
  };
}

export function filterSnapshotByPolicy(
  snapshot: Record<string, unknown>,
  policy: Policy | null
): Record<string, unknown> {
  if (!policy) {
    return snapshot;
  }

  const windows = (snapshot.windows as Array<Record<string, unknown>>) || [];
  const filteredWindows = windows.map((win) => {
    const tabs = (win.tabs as Array<Record<string, unknown>>) || [];
    const eligibleTabs = tabs
      .filter((tab) => evaluateTab(tab, policy).eligible)
      .map((tab) => annotateEntry(tab, policy));

    const eligibleGroupIds = new Set(
      eligibleTabs
        .map((tab) => tab.groupId)
        .filter((groupId) => typeof groupId === "number" && groupId !== -1) as number[]
    );
    const groups = (win.groups as Array<Record<string, unknown>>) || [];
    const filteredGroups = groups.filter((group) => eligibleGroupIds.has(group.groupId as number));

    return {
      ...win,
      tabs: eligibleTabs,
      groups: filteredGroups,
    };
  }).filter((win) => (win.tabs as Array<Record<string, unknown>>).length > 0);

  return {
    ...snapshot,
    windows: filteredWindows,
  };
}
