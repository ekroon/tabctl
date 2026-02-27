// Archive, close, and merge operations — extracted from background.ts (pure structural refactor).

type WindowSnapshot = import("./groups").WindowSnapshot;

import type { ExtensionDeps } from "./deps";

export async function ensureArchiveWindow(deps: Pick<ExtensionDeps, "getArchiveWindowId" | "setArchiveWindowId">) {
  const archiveWindowId = await deps.getArchiveWindowId();
  if (archiveWindowId) {
    try {
      await chrome.windows.get(archiveWindowId);
      return archiveWindowId;
    } catch {
      await deps.setArchiveWindowId(null);
    }
  }

  const created = await chrome.windows.create({ focused: false });
  await deps.setArchiveWindowId(created.id!);
  return created.id!;
}

export async function archiveTabs(params: Record<string, unknown>, deps: Pick<ExtensionDeps, "getTabSnapshot" | "selectTabsByScope" | "buildWindowLabels" | "resolveWindowIdFromParams" | "log" | "getArchiveWindowId" | "setArchiveWindowId">) {
  const snapshot = await deps.getTabSnapshot();
  const windowLabels = deps.buildWindowLabels(snapshot as unknown as { windows: Array<{ windowId: number }> });

  let windowsToProcess = snapshot.windows as Array<{ windowId: number; focused: boolean; state: string; tabs: Array<Record<string, unknown>>; groups: Array<Record<string, unknown>> }>;
  if (params.windowId) {
    const resolvedWindowId = deps.resolveWindowIdFromParams(snapshot, params.windowId);
    windowsToProcess = resolvedWindowId != null
      ? windowsToProcess.filter((win) => win.windowId === resolvedWindowId)
      : [];
  } else if (!params.all) {
    const focused = windowsToProcess.find((win) => win.focused);
    windowsToProcess = focused ? [focused] : [];
  }

  if (params.groupTitle || params.groupId || params.tabIds) {
    const selected = deps.selectTabsByScope(snapshot, params);
    if ((selected as { error?: Record<string, unknown> }).error) {
      throw (selected as { error: Record<string, unknown> }).error;
    }
    windowsToProcess = (snapshot.windows as Array<{ windowId: number; focused: boolean; state: string; tabs: Array<Record<string, unknown>>; groups: Array<Record<string, unknown>> }>)
      .map((win) => ({
        windowId: win.windowId,
        focused: win.focused,
        state: win.state,
        tabs: win.tabs.filter((tab) => (selected as { tabs: Array<Record<string, unknown>> }).tabs.some((sel) => sel.tabId === tab.tabId)),
        groups: win.groups,
      }))
      .filter((win) => win.tabs.length > 0);
  }

  if (windowsToProcess.length === 0) {
    const fullResult = {
      txid: params.txid || null,
      summary: { movedTabs: 0, movedGroups: 0, skippedTabs: 0 },
      archiveWindowId: null,
      skipped: [],
      undo: { action: "archive", tabs: [] },
    };
    return fullResult;
  }

  const archiveWindowId = await ensureArchiveWindow(deps);
  const undoTabs: Array<Record<string, unknown>> = [];
  const skipped: Array<Record<string, unknown>> = [];
  let movedGroups = 0;
  let movedTabs = 0;

  for (const window of windowsToProcess) {
    const groupsById = new Map<number, Record<string, unknown>>();
    for (const group of window.groups) {
      groupsById.set(group.groupId as number, group);
    }

    const groupedTabs = new Map<number, Array<Record<string, unknown>>>();
    const ungroupedTabs: Array<Record<string, unknown>> = [];

    for (const tab of window.tabs) {
      if (tab.tabId == null) {
        continue;
      }
      if (tab.groupId === -1) {
        ungroupedTabs.push(tab);
      } else {
        if (!groupedTabs.has(tab.groupId as number)) {
          groupedTabs.set(tab.groupId as number, []);
        }
        groupedTabs.get(tab.groupId as number)?.push(tab);
      }
    }

    const windowLabel = windowLabels.get(window.windowId) || `W${window.windowId}`;

    const plans: Array<Record<string, unknown>> = [];
    for (const [groupId, tabs] of groupedTabs.entries()) {
      const group = groupsById.get(groupId) || null;
      plans.push({
        windowId: window.windowId,
        windowLabel,
        group,
        tabs,
        isUngrouped: false,
      });
    }
    if (ungroupedTabs.length > 0) {
      plans.push({
        windowId: window.windowId,
        windowLabel,
        group: null,
        tabs: ungroupedTabs,
        isUngrouped: true,
      });
    }

    for (const plan of plans) {
      const tabIds = (plan.tabs as Array<Record<string, unknown>>).map((tab) => tab.tabId as number);
      if (tabIds.length === 0) {
        continue;
      }

      const tabById = new Map<number, Record<string, unknown>>();
      for (const tab of plan.tabs as Array<Record<string, unknown>>) {
        tabById.set(tab.tabId as number, tab);
      }

      let moved: chrome.tabs.Tab[] | chrome.tabs.Tab;
      try {
        moved = await chrome.tabs.move(tabIds, { windowId: archiveWindowId, index: -1 });
      } catch (error) {
        for (const tabId of tabIds) {
          skipped.push({ tabId, reason: "move_failed" });
        }
        deps.log("Failed to move tabs", error);
        continue;
      }

      const movedList = Array.isArray(moved) ? moved : [moved];
      const movedIds = movedList.map((tab) => tab.id as number);
      movedTabs += movedIds.length;

      for (const movedId of movedIds) {
        const tab = tabById.get(movedId);
        if (!tab) {
          continue;
        }
        undoTabs.push({
          tabId: tab.tabId,
          url: tab.url,
          title: tab.title,
          pinned: tab.pinned,
          active: tab.active,
          from: {
            windowId: tab.windowId,
            index: tab.index,
            groupId: tab.groupId,
            groupTitle: plan.group ? (plan.group as Record<string, unknown>).title : null,
            groupColor: plan.group ? (plan.group as Record<string, unknown>).color : null,
            groupCollapsed: plan.group ? (plan.group as Record<string, unknown>).collapsed : null,
          },
        });
      }

      const titleBase = plan.group && (plan.group as Record<string, unknown>).title
        ? (plan.group as Record<string, unknown>).title
        : plan.isUngrouped
          ? "Ungrouped"
          : "Group";
      const archiveTitle = `${plan.windowLabel} - ${titleBase}`;
      const groupColor = plan.group && (plan.group as Record<string, unknown>).color
        ? (plan.group as Record<string, unknown>).color
        : "grey";

      try {
        const newGroupId = await chrome.tabs.group({ tabIds: movedIds, createProperties: { windowId: archiveWindowId } });
        await chrome.tabGroups.update(newGroupId, { title: archiveTitle, color: groupColor as chrome.tabGroups.ColorEnum });
        movedGroups += 1;
      } catch (error) {
        deps.log("Failed to group archived tabs", error);
      }
    }
  }

  const fullResult = {
    txid: params.txid || null,
    summary: {
      movedTabs,
      movedGroups,
      skippedTabs: skipped.length,
    },
    archiveWindowId,
    skipped,
    undo: {
      action: "archive",
      tabs: undoTabs,
    },
  };
  return {
    txid: fullResult.txid,
    summary: fullResult.summary,
    archiveWindowId,
    skipped: fullResult.skipped,
    undo: fullResult.undo,
  };
}

export async function getTabsByIds(tabIds: Array<number>) {
  const results: Array<chrome.tabs.Tab | null> = [];
  for (const tabId of tabIds) {
    try {
      const tab = await chrome.tabs.get(tabId);
      results.push(tab);
    } catch {
      results.push(null);
    }
  }
  return results;
}

export async function closeTabs(params: Record<string, unknown>, deps: Pick<ExtensionDeps, "getTabSnapshot" | "selectTabsByScope">) {
  const mode = params.mode || "direct";
  if (mode === "direct" && !params.confirmed) {
    throw new Error("Direct close requires confirmation");
  }

  let tabIds = (params.tabIds as Array<number>) || [];
  if (!tabIds.length && (params.groupTitle || params.groupId || params.windowId)) {
    const snapshot = await deps.getTabSnapshot();
    const selection = deps.selectTabsByScope(snapshot, params) as { tabs: Array<Record<string, unknown>>; error?: Record<string, unknown> };
    if (selection.error) {
      throw selection.error;
    }
    tabIds = selection.tabs.map((tab) => tab.tabId as number);
  }

  if (!tabIds.length) {
    const fullResult = {
      txid: params.txid || null,
      summary: { closedTabs: 0, skippedTabs: 0 },
      skipped: [],
      undo: { action: "close", tabs: [] },
    };
    return fullResult;
  }

  const expectedUrls = (params.expectedUrls as Record<string, string>) || {};
  const tabInfos = await getTabsByIds(tabIds);
  const validTabs: Array<Record<string, unknown>> = [];
  const skipped: Array<Record<string, unknown>> = [];

  const groups = await chrome.tabGroups.query({});
  const groupById = new Map(groups.map((group) => [group.id, group]));

  for (let i = 0; i < tabIds.length; i += 1) {
    const tabId = tabIds[i];
    const tab = tabInfos[i];
    if (!tab) {
      skipped.push({ tabId, reason: "not_found" });
      continue;
    }

    const expected = expectedUrls[String(tabId)];
    if (expected && tab.url !== expected) {
      skipped.push({ tabId, reason: "url_mismatch" });
      continue;
    }

    const group = tab.groupId !== -1 ? groupById.get(tab.groupId) : null;
    validTabs.push({
      tabId,
      url: tab.url,
      title: tab.title,
      pinned: tab.pinned,
      active: tab.active,
      from: {
        windowId: tab.windowId,
        index: tab.index,
        groupId: tab.groupId,
        groupTitle: group ? group.title : null,
        groupColor: group ? group.color : null,
        groupCollapsed: group ? group.collapsed : null,
      },
    });
  }

  if (validTabs.length > 0) {
    await chrome.tabs.remove(validTabs.map((tab) => tab.tabId as number));
  }

  const fullResult = {
    txid: params.txid || null,
    summary: {
      closedTabs: validTabs.length,
      skippedTabs: skipped.length,
    },
    skipped,
    undo: {
      action: "close",
      tabs: validTabs.map((tab) => ({
        url: tab.url,
        title: tab.title,
        pinned: tab.pinned,
        active: tab.active,
        from: tab.from,
      })),
    },
  };
  return {
    txid: fullResult.txid,
    summary: fullResult.summary,
    skipped: fullResult.skipped,
    undo: fullResult.undo,
  };
}

export async function mergeWindow(params: Record<string, unknown>, deps: Pick<ExtensionDeps, "getTabSnapshot" | "log">) {
  const fromWindowId = Number.isFinite(params.fromWindowId as number)
    ? Number(params.fromWindowId)
    : Number(params.windowId);
  const toWindowId = Number.isFinite(params.toWindowId as number) ? Number(params.toWindowId) : null;
  if (!Number.isFinite(fromWindowId) || !Number.isFinite(toWindowId)) {
    throw new Error("Missing source or target window id");
  }
  if (fromWindowId === toWindowId) {
    throw new Error("Source and target windows must differ");
  }

  const snapshot = await deps.getTabSnapshot();
  const windows = snapshot.windows as WindowSnapshot[];
  const sourceWindow = windows.find((win) => win.windowId === fromWindowId);
  if (!sourceWindow) {
    throw new Error("Source window not found");
  }
  const targetWindow = windows.find((win) => win.windowId === toWindowId);
  if (!targetWindow) {
    throw new Error("Target window not found");
  }

  const rawTabIds = Array.isArray(params.tabIds) ? params.tabIds.map(Number) : [];
  const tabIdSet = new Set(rawTabIds.filter((id) => Number.isFinite(id)) as number[]);
  const skipped: Array<Record<string, unknown>> = [];
  let selectedTabs = sourceWindow.tabs;

  if (tabIdSet.size > 0) {
    const sourceTabIds = new Set(
      sourceWindow.tabs.map((tab) => tab.tabId).filter((id) => typeof id === "number") as number[],
    );
    for (const tabId of tabIdSet) {
      if (!sourceTabIds.has(tabId)) {
        skipped.push({ tabId, reason: "not_in_source" });
      }
    }
    selectedTabs = sourceWindow.tabs.filter((tab) => tabIdSet.has(tab.tabId as number));
  }

  if (selectedTabs.length === 0) {
    const fullResult = {
      fromWindowId,
      toWindowId,
      summary: { movedTabs: 0, movedGroups: 0, skippedTabs: skipped.length, closedSource: false },
      skipped,
      groups: [],
      undo: {
        action: "merge-window",
        fromWindowId,
        toWindowId,
        closedSource: false,
        tabs: [],
      },
    };
    return fullResult;
  }

  const orderedTabs = [...selectedTabs].sort((a, b) => {
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
    return aIndex - bIndex;
  });

  const groupById = new Map<number, Record<string, unknown>>();
  for (const group of sourceWindow.groups) {
    groupById.set(group.groupId as number, group);
  }

  const plans: Array<{ groupId: number | null; tabs: Array<Record<string, unknown>> }> = [];
  let currentPlan: { groupId: number | null; tabs: Array<Record<string, unknown>> } | null = null;
  for (const tab of orderedTabs) {
    const rawGroupId = tab.groupId as number;
    const groupId = typeof rawGroupId === "number" && rawGroupId !== -1 ? rawGroupId : null;
    if (!currentPlan || currentPlan.groupId !== groupId) {
      currentPlan = { groupId, tabs: [] };
      plans.push(currentPlan);
    }
    currentPlan.tabs.push(tab);
  }

  let movedTabs = 0;
  let movedGroups = 0;
  const groups: Array<Record<string, unknown>> = [];
  const undoTabs: Array<Record<string, unknown>> = [];

  for (const plan of plans) {
    const tabIds = plan.tabs.map((tab) => tab.tabId).filter((id) => typeof id === "number") as number[];
    if (!tabIds.length) {
      continue;
    }

    let moved: chrome.tabs.Tab[] | chrome.tabs.Tab;
    try {
      moved = await chrome.tabs.move(tabIds, { windowId: toWindowId!, index: -1 });
    } catch (error) {
      for (const tabId of tabIds) {
        skipped.push({ tabId, reason: "move_failed" });
      }
      deps.log("Failed to move tabs", error);
      continue;
    }

    const movedList = Array.isArray(moved) ? moved : [moved];
    const movedIds = movedList.map((tab) => tab.id as number).filter((id) => typeof id === "number");
    movedTabs += movedIds.length;

    for (const entry of plan.tabs) {
      if (typeof entry.tabId !== "number") {
        continue;
      }
      const meta = groupById.get(entry.groupId as number);
      undoTabs.push({
        tabId: entry.tabId,
        windowId: entry.windowId,
        index: entry.index,
        groupId: entry.groupId,
        groupTitle: entry.groupTitle,
        groupColor: entry.groupColor,
        groupCollapsed: meta ? meta.collapsed : null,
      });
    }

    if (plan.groupId != null && movedIds.length > 0) {
      movedGroups += 1;
      let newGroupId: number | null = null;
      try {
        newGroupId = await chrome.tabs.group({ tabIds: movedIds, createProperties: { windowId: toWindowId! } });
        const meta = groupById.get(plan.groupId);
        if (meta) {
          await chrome.tabGroups.update(newGroupId, {
            title: (meta.title as string) || "",
            color: (meta.color as chrome.tabGroups.ColorEnum) || "grey",
            collapsed: (meta.collapsed as boolean | undefined) || false,
          });
        }
      } catch (error) {
        deps.log("Failed to regroup tabs", error);
      }
      groups.push({ sourceGroupId: plan.groupId, newGroupId });
    }
  }

  let closedSource = false;
  if (params.closeSource === true) {
    try {
      const remainingTabs = await chrome.tabs.query({ windowId: fromWindowId });
      if (remainingTabs.length === 0) {
        await chrome.windows.remove(fromWindowId);
        closedSource = true;
      }
    } catch (error) {
      deps.log("Failed to close source window", error);
    }
  }

  const fullResult = {
    fromWindowId,
    toWindowId,
    summary: { movedTabs, movedGroups, skippedTabs: skipped.length, closedSource },
    skipped,
    groups,
    undo: {
      action: "merge-window",
      fromWindowId,
      toWindowId,
      closedSource,
      tabs: undoTabs,
    },
    txid: params.txid || null,
  };
  return {
    fromWindowId,
    toWindowId,
    summary: fullResult.summary,
    skipped: fullResult.skipped,
    groups: fullResult.groups,
    undo: fullResult.undo,
    txid: fullResult.txid,
  };
}
