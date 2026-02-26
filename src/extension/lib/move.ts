// Tab/group movement — extracted from tabs.ts (pure structural refactor).

const tabs = require("./tabs") as typeof import("./tabs");
const { normalizeTabIndex } = tabs;

type WindowSnapshot = import("./groups").WindowSnapshot;
type GroupMatch = import("./groups").GroupMatch;

import type { ExtensionDeps } from "./deps";

export function resolveMoveTarget(snapshot: { windows: Array<Record<string, unknown>> }, params: Record<string, unknown>, deps: Pick<ExtensionDeps, "resolveGroupByTitle">) {
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
  const resolved = deps.resolveGroupByTitle(snapshot, groupTitle, windowId);
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

export async function moveTab(params: Record<string, unknown>, deps: Pick<ExtensionDeps, "getTabSnapshot" | "resolveWindowIdFromParams" | "resolveGroupByTitle">) {
  const tabIds = Array.isArray(params.tabIds) ? params.tabIds.map(Number) : [];
  const tabId = Number.isFinite(params.tabId as number)
    ? Number(params.tabId)
    : tabIds.length
      ? Number(tabIds[0])
      : null;
  if (!tabId) {
    throw new Error("Missing tabId");
  }

  const snapshot = await deps.getTabSnapshot();

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
    const resolvedWindowId = deps.resolveWindowIdFromParams(snapshot, params.windowId);
    normalizedParams = { ...params, windowId: resolvedWindowId ?? undefined };
  }

  const target = resolveMoveTarget(snapshot, normalizedParams, deps);
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

export async function moveGroup(params: Record<string, unknown>, deps: Pick<ExtensionDeps, "getTabSnapshot" | "log" | "resolveWindowIdFromParams" | "resolveGroupByTitle" | "resolveGroupById">) {
  const groupId = Number.isFinite(params.groupId as number) ? Number(params.groupId) : null;
  const groupTitle = typeof params.groupTitle === "string" ? params.groupTitle.trim() : "";
  if (!groupId && !groupTitle) {
    throw new Error("Missing group identifier");
  }

  const snapshot = await deps.getTabSnapshot();
  const windowIdParam = params.windowId != null ? deps.resolveWindowIdFromParams(snapshot, params.windowId) ?? undefined : undefined;

  const resolvedGroup = groupId != null
    ? deps.resolveGroupById(snapshot, groupId)
    : deps.resolveGroupByTitle(snapshot, groupTitle, windowIdParam);
  if ((resolvedGroup as { error?: Record<string, unknown> }).error) {
    throw (resolvedGroup as { error: Record<string, unknown> }).error;
  }
  const source = (resolvedGroup as { match: GroupMatch }).match;
  if (!source.tabs.length) {
    throw new Error("Group has no tabs to move");
  }
  const ensureMovedTabsAreGrouped = async (
    movedTabIds: number[],
    targetWindowId: number,
    targetGroupId: number | null,
  ) => {
    if (!targetGroupId || movedTabIds.length === 0) {
      return;
    }
    const movedSet = new Set(movedTabIds);
    const verify = async (step: string) => {
      const tabs = await chrome.tabs.query({ windowId: targetWindowId });
      const missingGroupTabIds = tabs
        .filter((tab) => typeof tab.id === "number" && movedSet.has(tab.id) && tab.groupId !== targetGroupId)
        .map((tab) => tab.id as number);
      if (missingGroupTabIds.length > 0) {
        await chrome.tabs.group({ groupId: targetGroupId, tabIds: missingGroupTabIds });
      }
    };
    try {
      await verify("group-verify");
      await new Promise((resolve) => setTimeout(resolve, 250));
      await verify("group-verify-delayed");
      await new Promise((resolve) => setTimeout(resolve, 1500));
      await verify("group-verify-delayed-late");
    } catch (error) {
      deps.log("Failed to enforce moved group integrity", error);
    }
  };

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
      deps.log("Failed to regroup tabs", error);
    }
    await ensureMovedTabsAreGrouped(tabIds, targetWindowId, newGroupId);

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

  const target = resolveMoveTarget(snapshot, params, deps);
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
      deps.log("Failed to regroup tabs", error);
    }
  }
  await ensureMovedTabsAreGrouped(
    tabIds,
    targetWindowId,
    targetWindowId === source.windowId ? (source.group.groupId as number) : newGroupId,
  );

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
