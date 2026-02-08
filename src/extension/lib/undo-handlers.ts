// Undo transaction handlers — extracted from background.ts (pure structural refactor).

type AnyRecord = Record<string, any>;

export interface UndoHandlerDeps {
  log: (...args: Array<unknown>) => void;
}

async function ensureWindow(windowId?: number) {
  if (windowId) {
    try {
      const existing = await chrome.windows.get(windowId);
      if (existing) {
        return windowId;
      }
    } catch {
      // fall through to create
    }
  }

  const created = await chrome.windows.create({ focused: false });
  return created.id as number;
}

async function restoreTabsFromUndo(entries: Array<AnyRecord>, deps: UndoHandlerDeps) {
  const skipped: Array<AnyRecord> = [];
  const restored: Array<{ tabId: number; entry: AnyRecord; targetWindowId: number }> = [];
  const windowMap = new Map<number, number>();

  for (const entry of entries) {
    const tabId = Number(entry.tabId);
    if (!Number.isFinite(tabId)) {
      skipped.push({ tabId: entry.tabId, reason: "missing_tab" });
      continue;
    }

    const sourceWindowId = Number(entry.windowId);
    if (!Number.isFinite(sourceWindowId)) {
      skipped.push({ tabId, reason: "missing_window" });
      continue;
    }

    let targetWindowId = windowMap.get(sourceWindowId);
    if (!targetWindowId) {
      targetWindowId = await ensureWindow(sourceWindowId);
      windowMap.set(sourceWindowId, targetWindowId);
    }

    try {
      await chrome.tabs.move(tabId, { windowId: targetWindowId, index: -1 });
      restored.push({ tabId, entry, targetWindowId });
    } catch {
      skipped.push({ tabId, reason: "move_failed" });
    }
  }

  const groupsByWindow = new Map<string, {
    windowId: number;
    groupId: number | null;
    groupTitle: string | null;
    groupColor: string | null;
    groupCollapsed: boolean | null;
    tabIds: number[];
  }>();

  for (const item of restored) {
    const entry = item.entry;
    const rawGroupId = typeof entry.groupId === "number" ? entry.groupId : null;
    const groupId = rawGroupId != null && rawGroupId !== -1 ? rawGroupId : null;
    const groupTitle = typeof entry.groupTitle === "string" ? entry.groupTitle : null;
    if (!groupId && !groupTitle) {
      continue;
    }

    const groupKey = groupId != null ? `id:${groupId}` : `title:${groupTitle}`;
    const key = `${item.targetWindowId}:${groupKey}`;
    if (!groupsByWindow.has(key)) {
      groupsByWindow.set(key, {
        windowId: item.targetWindowId,
        groupId,
        groupTitle,
        groupColor: typeof entry.groupColor === "string" ? entry.groupColor : null,
        groupCollapsed: typeof entry.groupCollapsed === "boolean" ? entry.groupCollapsed : null,
        tabIds: [],
      });
    }
    groupsByWindow.get(key)?.tabIds.push(item.tabId);
  }

  for (const group of groupsByWindow.values()) {
    if (group.tabIds.length === 0) {
      continue;
    }

    let targetGroupId: number | null = null;
    if (group.groupId != null) {
      try {
        targetGroupId = await chrome.tabs.group({ groupId: group.groupId, tabIds: group.tabIds });
      } catch {
        targetGroupId = null;
      }
    }

    if (targetGroupId == null) {
      try {
        targetGroupId = await chrome.tabs.group({
          tabIds: group.tabIds,
          createProperties: { windowId: group.windowId },
        });
      } catch (error) {
        deps.log("Failed to regroup tabs", error);
        continue;
      }
    }

    const update: chrome.tabGroups.UpdateProperties = {};
    if (typeof group.groupTitle === "string") {
      update.title = group.groupTitle;
    }
    if (typeof group.groupColor === "string" && group.groupColor) {
      update.color = group.groupColor as chrome.tabGroups.ColorEnum;
    }
    if (typeof group.groupCollapsed === "boolean") {
      update.collapsed = group.groupCollapsed;
    }
    if (Object.keys(update).length > 0) {
      try {
        await chrome.tabGroups.update(targetGroupId, update);
      } catch (error) {
        deps.log("Failed to update restored group", error);
      }
    }
  }

  const orderByWindow = new Map<number, Array<{ tabId: number; index: number }>>();
  for (const item of restored) {
    const index = Number(item.entry.index);
    if (!Number.isFinite(index)) {
      continue;
    }
    if (!orderByWindow.has(item.targetWindowId)) {
      orderByWindow.set(item.targetWindowId, []);
    }
    orderByWindow.get(item.targetWindowId)?.push({ tabId: item.tabId, index });
  }

  for (const [targetWindowId, items] of orderByWindow.entries()) {
    const ordered = [...items].sort((a, b) => a.index - b.index);
    for (const item of ordered) {
      try {
        await chrome.tabs.move(item.tabId, { windowId: targetWindowId, index: item.index });
      } catch {
        // ignore ordering failures
      }
    }
  }

  return {
    summary: {
      restoredTabs: restored.length,
      skippedTabs: skipped.length,
    },
    skipped,
  };
}

export async function undoGroupUpdate(undo: AnyRecord) {
  const groupId = Number(undo.groupId);
  if (!Number.isFinite(groupId)) {
    return {
      summary: { restoredGroups: 0, skippedGroups: 1 },
      skipped: [{ groupId: undo.groupId, reason: "missing_group" }],
    };
  }

  const previous = (undo.previous as AnyRecord) || {};
  const update: chrome.tabGroups.UpdateProperties = {};
  if (typeof previous.title === "string") {
    update.title = previous.title;
  }
  if (typeof previous.color === "string" && previous.color) {
    update.color = previous.color as chrome.tabGroups.ColorEnum;
  }
  if (typeof previous.collapsed === "boolean") {
    update.collapsed = previous.collapsed;
  }
  if (!Object.keys(update).length) {
    return {
      summary: { restoredGroups: 0, skippedGroups: 1 },
      skipped: [{ groupId, reason: "missing_values" }],
    };
  }

  try {
    await chrome.tabGroups.update(groupId, update);
    return {
      summary: { restoredGroups: 1, skippedGroups: 0 },
      skipped: [],
    };
  } catch {
    return {
      summary: { restoredGroups: 0, skippedGroups: 1 },
      skipped: [{ groupId, reason: "update_failed" }],
    };
  }
}

export async function undoGroupUngroup(undo: AnyRecord, deps: UndoHandlerDeps) {
  const tabs = (undo.tabs as Array<AnyRecord>) || [];
  return await restoreTabsFromUndo(tabs, deps);
}

export async function undoGroupAssign(undo: AnyRecord, deps: UndoHandlerDeps) {
  const tabs = (undo.tabs as Array<AnyRecord>) || [];
  return await restoreTabsFromUndo(tabs, deps);
}

export async function undoMoveTab(undo: AnyRecord, deps: UndoHandlerDeps) {
  const from = (undo.from as AnyRecord) || {};
  const entry = {
    tabId: undo.tabId,
    windowId: from.windowId,
    index: from.index,
    groupId: from.groupId,
    groupTitle: from.groupTitle,
    groupColor: from.groupColor,
    groupCollapsed: from.groupCollapsed,
  };
  return await restoreTabsFromUndo([entry], deps);
}

export async function undoMoveGroup(undo: AnyRecord, deps: UndoHandlerDeps) {
  const tabs = (undo.tabs as Array<AnyRecord>) || [];
  return await restoreTabsFromUndo(tabs, deps);
}

export async function undoMergeWindow(undo: AnyRecord, deps: UndoHandlerDeps) {
  const tabs = (undo.tabs as Array<AnyRecord>) || [];
  return await restoreTabsFromUndo(tabs, deps);
}

export async function undoArchive(undo: AnyRecord, deps: UndoHandlerDeps) {
  const tabs = (undo.tabs as Array<AnyRecord>) || [];
  const restored: Array<{ tabId: number; targetWindowId: number }> = [];
  const skipped: Array<AnyRecord> = [];

  const windowMap = new Map<number, number>();
  for (const entry of tabs) {
    if (!entry.tabId) {
      skipped.push({ tabId: entry.tabId, reason: "missing_tab" });
      continue;
    }

    let targetWindowId = windowMap.get(entry.from.windowId as number);
    if (!targetWindowId) {
      targetWindowId = await ensureWindow(entry.from.windowId as number);
      windowMap.set(entry.from.windowId as number, targetWindowId);
    }

    try {
      await chrome.tabs.move(entry.tabId as number, { windowId: targetWindowId, index: -1 });
      restored.push({ tabId: entry.tabId as number, targetWindowId });
    } catch {
      skipped.push({ tabId: entry.tabId, reason: "move_failed" });
    }
  }

  const restoredSet = new Set(restored.map((item) => item.tabId));

  const groupsByWindow = new Map<string, Array<AnyRecord>>();
  for (const entry of tabs) {
    if (!restoredSet.has(entry.tabId as number)) {
      continue;
    }
    const targetWindowId = windowMap.get(entry.from.windowId as number) || (entry.from.windowId as number);
    const hasGroupId = entry.from.groupId != null && entry.from.groupId !== -1;
    const hasGroupTitle = entry.from.groupTitle != null;
    if (!hasGroupId && !hasGroupTitle) {
      continue;
    }
    const groupKey = hasGroupId ? entry.from.groupId : entry.from.groupTitle;
    const key = `${targetWindowId}:${groupKey}`;
    if (!groupsByWindow.has(key)) {
      groupsByWindow.set(key, []);
    }
    groupsByWindow.get(key)?.push(entry);
  }

  for (const [key, groupTabs] of groupsByWindow.entries()) {
    const [windowIdPart] = key.split(":");
    const targetWindowId = Number(windowIdPart);
    const tabIds = groupTabs.map((entry) => entry.tabId).filter(Boolean) as number[];
    if (!tabIds.length) {
      continue;
    }
    try {
      const groupId = await chrome.tabs.group({ tabIds, createProperties: { windowId: targetWindowId } });
      await chrome.tabGroups.update(groupId, {
        title: (groupTabs[0].from.groupTitle as string) || "",
        color: (groupTabs[0].from.groupColor as chrome.tabGroups.ColorEnum) || "grey",
        collapsed: groupTabs[0].from.groupCollapsed as boolean | undefined || false,
      });
    } catch (error) {
      deps.log("Failed to recreate group", error);
    }
  }

  for (const [originalWindowId, targetWindowId] of windowMap.entries()) {
    const windowTabs = tabs
      .filter((entry) => entry.from.windowId === originalWindowId && restoredSet.has(entry.tabId as number))
      .sort((a, b) => (a.from.index as number) - (b.from.index as number));
    for (const entry of windowTabs) {
      if (!entry.tabId) {
        continue;
      }
      try {
        await chrome.tabs.move(entry.tabId as number, { windowId: targetWindowId, index: entry.from.index as number });
      } catch {
        // ignore ordering failures
      }
    }

    const activeTab = windowTabs.find((entry) => entry.active);
    if (activeTab && activeTab.tabId) {
      try {
        await chrome.tabs.update(activeTab.tabId as number, { active: true });
      } catch {
        // ignore
      }
    }
  }

  return {
    summary: {
      restoredTabs: restored.length,
      skippedTabs: skipped.length,
    },
    skipped,
  };
}

export async function undoClose(undo: AnyRecord, deps: UndoHandlerDeps) {
  const tabs = (undo.tabs as Array<AnyRecord>) || [];
  const restored: Array<{ tabId: number; entry: AnyRecord }> = [];
  const skipped: Array<AnyRecord> = [];

  const windowMap = new Map<number, number>();
  for (const entry of tabs) {
    if (!entry.url) {
      skipped.push({ url: entry.url, reason: "missing_url" });
      continue;
    }
    let targetWindowId = windowMap.get(entry.from.windowId as number);
    if (!targetWindowId) {
      targetWindowId = await ensureWindow(entry.from.windowId as number);
      windowMap.set(entry.from.windowId as number, targetWindowId);
    }

    try {
      const created = await chrome.tabs.create({
        windowId: targetWindowId,
        url: entry.url as string,
        active: false,
        pinned: entry.pinned as boolean,
      });
      restored.push({ tabId: created.id as number, entry });
    } catch {
      skipped.push({ url: entry.url, reason: "create_failed" });
    }
  }

  const groupsByWindow = new Map<string, Array<AnyRecord>>();
  for (const item of restored) {
    const entry = item.entry;
    const targetWindowId = windowMap.get(entry.from.windowId as number) || (entry.from.windowId as number);
    const hasGroupId = entry.from.groupId != null && entry.from.groupId !== -1;
    const hasGroupTitle = entry.from.groupTitle != null;
    if (!hasGroupId && !hasGroupTitle) {
      continue;
    }
    const groupKey = hasGroupId ? entry.from.groupId : entry.from.groupTitle;
    const key = `${targetWindowId}:${groupKey}`;
    if (!groupsByWindow.has(key)) {
      groupsByWindow.set(key, []);
    }
    groupsByWindow.get(key)?.push({ tabId: item.tabId, entry });
  }

  for (const [key, groupTabs] of groupsByWindow.entries()) {
    const [windowIdPart] = key.split(":");
    const targetWindowId = Number(windowIdPart);
    const tabIds = groupTabs.map((item) => item.tabId).filter(Boolean) as number[];
    if (!tabIds.length) {
      continue;
    }
    try {
      const groupId = await chrome.tabs.group({ tabIds, createProperties: { windowId: targetWindowId } });
      await chrome.tabGroups.update(groupId, {
        title: (groupTabs[0].entry.from.groupTitle as string) || "",
        color: (groupTabs[0].entry.from.groupColor as chrome.tabGroups.ColorEnum) || "grey",
        collapsed: (groupTabs[0].entry.from.groupCollapsed as boolean | undefined) || false,
      });
    } catch (error) {
      deps.log("Failed to recreate group", error);
    }
  }

  for (const [originalWindowId, targetWindowId] of windowMap.entries()) {
    const windowTabs = restored
      .map((item) => ({ tabId: item.tabId, entry: item.entry }))
      .filter((item) => item.entry.from.windowId === originalWindowId)
      .sort((a, b) => (a.entry.from.index as number) - (b.entry.from.index as number));

    for (const item of windowTabs) {
      try {
        await chrome.tabs.move(item.tabId, { windowId: targetWindowId, index: item.entry.from.index as number });
      } catch {
        // ignore ordering failures
      }
    }

    const activeTab = windowTabs.find((item) => item.entry.active);
    if (activeTab) {
      try {
        await chrome.tabs.update(activeTab.tabId as number, { active: true });
      } catch {
        // ignore
      }
    }
  }

  return {
    summary: {
      restoredTabs: restored.length,
      skippedTabs: skipped.length,
    },
    skipped,
  };
}

export async function undoTransaction(params: Record<string, unknown>, deps: UndoHandlerDeps) {
  if (!params.record || !(params.record as Record<string, unknown>).undo) {
    throw new Error("Undo record missing");
  }

  const undo = (params.record as Record<string, unknown>).undo as Record<string, unknown>;
  if (undo.action === "archive") {
    return await undoArchive(undo, deps);
  }
  if (undo.action === "close") {
    return await undoClose(undo, deps);
  }
  if (undo.action === "group-update") {
    return await undoGroupUpdate(undo);
  }
  if (undo.action === "group-ungroup") {
    return await undoGroupUngroup(undo, deps);
  }
  if (undo.action === "group-assign") {
    return await undoGroupAssign(undo, deps);
  }
  if (undo.action === "move-tab") {
    return await undoMoveTab(undo, deps);
  }
  if (undo.action === "move-group") {
    return await undoMoveGroup(undo, deps);
  }
  if (undo.action === "merge-window") {
    return await undoMergeWindow(undo, deps);
  }

  throw new Error(`Unknown undo action: ${undo.action}`);
}
