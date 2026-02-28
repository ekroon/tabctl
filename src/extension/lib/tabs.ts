// Tab operations — extracted from background.ts (pure structural refactor).

import normalizeUrlLib from "normalize-url";

type WindowSnapshot = import("./groups").WindowSnapshot;

import type { ExtensionDeps } from "./deps";

export function getMostRecentFocusedWindowId(windows: WindowSnapshot[]) {
  let bestWindowId: number | null = null;
  let bestFocusedAt = -Infinity;
  for (const win of windows) {
    for (const tab of win.tabs) {
      const focusedAt = Number(tab.lastFocusedAt);
      if (!Number.isFinite(focusedAt)) {
        continue;
      }
      if (focusedAt > bestFocusedAt) {
        bestFocusedAt = focusedAt;
        bestWindowId = win.windowId;
      }
    }
  }
  return bestWindowId;
}

export function normalizeUrl(rawUrl: unknown): string | null {
  if (!rawUrl || typeof rawUrl !== "string") {
    return null;
  }
  try {
    return normalizeUrlLib(rawUrl, {
      stripHash: true,
      removeQueryParameters: [
        /^utm_\w+$/i,
        "fbclid",
        "gclid",
        "igshid",
        "mc_cid",
        "mc_eid",
        "ref",
        "ref_src",
        "ref_url",
        "si",
      ],
    });
  } catch {
    return null;
  }
}

export function normalizeTabIndex(value: unknown) {
  const index = Number(value);
  return Number.isFinite(index) ? index : null;
}

function shapeOpenResult(
  result: {
    windowId: number;
    groupId: number | null;
    created: Array<Record<string, unknown>>;
    skipped: Array<Record<string, unknown>>;
    summary: Record<string, unknown>;
  },
) {
  return {
    windowId: result.windowId,
    groupId: result.groupId,
    createdTabIds: result.created
      .map((tab) => tab.tabId)
      .filter((id): id is number => typeof id === "number"),
    skipped: result.skipped,
    summary: result.summary,
  };
}

function matchIncludes(value: unknown, needle: string) {
  if (!needle) {
    return false;
  }
  return typeof value === "string" && value.toLowerCase().includes(needle);
}

export function resolveOpenWindow(snapshot: { windows: Array<Record<string, unknown>> }, params: Record<string, unknown>) {
  const windows = snapshot.windows as WindowSnapshot[];
  if (!windows.length) {
    return { error: { message: "No windows available" } };
  }

  if (params.windowId != null) {
    if (typeof params.windowId === "string") {
      const normalized = params.windowId.trim().toLowerCase();
      if (normalized === "active") {
        const focused = windows.find((win) => win.focused);
        if (focused) {
          return { windowId: focused.windowId };
        }
        return { error: { message: "Active window not found" } };
      }
      if (normalized === "last-focused") {
        const lastFocused = getMostRecentFocusedWindowId(windows);
        if (lastFocused != null) {
          return { windowId: lastFocused };
        }
        return { error: { message: "Last focused window not found" } };
      }
      if (normalized === "new") {
        return { error: { message: "--window new is only supported by open" } };
      }
    }
    const windowId = Number(params.windowId);
    const found = windows.find((win) => win.windowId === windowId);
    if (!found) {
      return { error: { message: "Window not found" } };
    }
    return { windowId };
  }

  if (params.windowTabId != null) {
    const tabId = Number(params.windowTabId);
    const found = windows.find((win) => win.tabs.some((tab) => tab.tabId === tabId));
    if (!found) {
      return { error: { message: "Window not found for tab" } };
    }
    return { windowId: found.windowId };
  }

  let candidates = [...windows];
  let filtered = false;

  if (typeof params.afterGroupTitle === "string" && params.afterGroupTitle.trim()) {
    const groupTitle = params.afterGroupTitle.trim();
    candidates = candidates.filter((win) => win.groups.some((group) => group.title === groupTitle));
    filtered = true;
  }

  if (typeof params.windowGroupTitle === "string" && params.windowGroupTitle.trim()) {
    const groupTitle = params.windowGroupTitle.trim();
    candidates = candidates.filter((win) => win.groups.some((group) => group.title === groupTitle));
    filtered = true;
  }

  if (typeof params.windowUrl === "string" && params.windowUrl.trim()) {
    const needle = params.windowUrl.trim().toLowerCase();
    candidates = candidates.filter((win) => win.tabs.some((tab) => matchIncludes(tab.url, needle)));
    filtered = true;
  }

  if (filtered) {
    if (candidates.length === 1) {
      return { windowId: candidates[0].windowId };
    }
    if (candidates.length === 0) {
      return { error: { message: "No matching window found" } };
    }
    return { error: { message: "Multiple windows match selection. Provide --window to disambiguate." } };
  }

  const focused = windows.find((win) => win.focused);
  if (focused) {
    return { windowId: focused.windowId };
  }

  if (windows.length === 1) {
    return { windowId: windows[0].windowId };
  }

  const lastFocused = getMostRecentFocusedWindowId(windows);
  if (lastFocused != null) {
    return { windowId: lastFocused };
  }

  return { error: { message: "Multiple windows available. Provide --window to target one." } };
}

export async function focusTab(params: Record<string, unknown>) {
  const tabIds = Array.isArray(params.tabIds) ? params.tabIds.map(Number) : [];
  const tabId = Number.isFinite(params.tabId as number)
    ? Number(params.tabId)
    : tabIds.length
      ? Number(tabIds[0])
      : null;

  if (!tabId) {
    throw new Error("Missing tabId");
  }

  const tab = await chrome.tabs.get(tabId);
  await chrome.windows.update(tab.windowId, { focused: true });
  await chrome.tabs.update(tabId, { active: true });

  return {
    tabId,
    windowId: tab.windowId,
  };
}

export async function refreshTabs(params: Record<string, unknown>) {
  const tabIds = Array.isArray(params.tabIds) ? params.tabIds.map(Number) : [];
  const tabId = Number.isFinite(params.tabId as number)
    ? Number(params.tabId)
    : tabIds.length
      ? Number(tabIds[0])
      : null;

  if (!tabId) {
    throw new Error("Missing tabId");
  }

  await chrome.tabs.reload(tabId);

  return {
    tabId,
    summary: { refreshedTabs: 1 },
  };
}

export async function openTabs(params: Record<string, unknown>, deps: Pick<ExtensionDeps, "getTabSnapshot" | "log" | "delay">) {
  const urls = Array.isArray(params.urls)
    ? params.urls.map((url) => (typeof url === "string" ? url.trim() : "")).filter(Boolean)
    : [];
  const groupTitle = typeof params.groupTitle === "string" ? params.groupTitle.trim() : "";
  const groupColor = typeof params.color === "string" ? params.color.trim() : "";
  const afterGroupTitle = typeof params.afterGroupTitle === "string" ? params.afterGroupTitle.trim() : "";
  const beforeTabId = Number.isFinite(params.beforeTabId as number) ? Number(params.beforeTabId) : null;
  const afterTabId = Number.isFinite(params.afterTabId as number) ? Number(params.afterTabId) : null;
  if (beforeTabId != null && afterTabId != null) {
    throw new Error("Only one target position is allowed");
  }
  const newWindow = params.newWindow === true;
  const forceNewGroup = params.newGroup === true;
  const allowDuplicates = params.allowDuplicates === true;
  if (!urls.length && !newWindow) {
    throw new Error("No URLs provided");
  }

  if (newWindow) {
    if (afterGroupTitle || beforeTabId || afterTabId) {
      throw new Error("Cannot use --before/--after with --new-window");
    }
    if (params.windowId != null || params.windowGroupTitle || params.windowTabId != null || params.windowUrl) {
      throw new Error("Cannot combine --new-window with window selectors");
    }

    const created: Array<Record<string, unknown>> = [];
    const skipped: Array<Record<string, unknown>> = [];

    const createdWindow = await chrome.windows.create({ focused: false });
    const windowId = createdWindow.id as number;
    let seedTabs = createdWindow.tabs;
    if (!seedTabs) {
      seedTabs = await chrome.tabs.query({ windowId });
    }
    const seedTabId = seedTabs.find((tab) => typeof tab.id === "number")?.id ?? null;

    for (const url of urls) {
      try {
        const tab = await chrome.tabs.create({ windowId, url, active: false });
        created.push({
          tabId: tab.id,
          windowId: tab.windowId,
          index: tab.index,
          url: tab.url,
          title: tab.title,
        });
      } catch (error) {
        skipped.push({ url, reason: "create_failed" });
      }
    }

    if (!urls.length && seedTabs.length) {
      const tab = seedTabs[0];
      created.push({
        tabId: tab.id,
        windowId: tab.windowId,
        index: tab.index,
        url: tab.url,
        title: tab.title,
      });
    }

    if (seedTabId && created.length > 0 && urls.length > 0) {
      try {
        await chrome.tabs.remove(seedTabId);
      } catch (error) {
        deps.log("Failed to remove seed tab", error);
      }
    }

    let groupId: number | null = null;
    if (groupTitle && created.length > 0) {
      try {
        const tabIds = created.map((tab) => tab.tabId as number).filter((id) => typeof id === "number");
        if (tabIds.length > 0) {
          groupId = await chrome.tabs.group({ tabIds, createProperties: { windowId } });
          const update: chrome.tabGroups.UpdateProperties = { title: groupTitle };
          if (groupColor) {
            update.color = groupColor as chrome.tabGroups.ColorEnum;
          }
          await chrome.tabGroups.update(groupId, update);
        }
      } catch (error) {
        deps.log("Failed to create group", error);
        groupId = null;
      }
    }

    return shapeOpenResult({
      windowId,
      groupId,
      created,
      skipped,
      summary: {
        createdTabs: created.length,
        skippedUrls: skipped.length,
        grouped: Boolean(groupId),
      },
    });
  }

  const snapshot = await deps.getTabSnapshot();
  let openParams = params;

  // Auto-resolve window by group name when no explicit window selector is provided
  if (groupTitle && !forceNewGroup && openParams.windowId == null && !openParams.windowGroupTitle && !openParams.windowTabId && !openParams.windowUrl) {
    const groupWindows = (snapshot.windows as WindowSnapshot[]).filter(
      (win) => win.groups.some((g) => g.title === groupTitle),
    );
    if (groupWindows.length === 1) {
      openParams = { ...openParams, windowId: groupWindows[0].windowId };
    }
  }

  if (params.windowId == null && (beforeTabId != null || afterTabId != null)) {
    const anchorId = beforeTabId != null ? beforeTabId : (afterTabId as number);
    const anchorWindow = (snapshot.windows as WindowSnapshot[])
      .find((win) => win.tabs.some((tab) => tab.tabId === anchorId));
    if (anchorWindow) {
      openParams = { ...params, windowId: anchorWindow.windowId };
    }
  }
  const selection = resolveOpenWindow(snapshot, openParams);
  if ((selection as { error?: Record<string, unknown> }).error) {
    throw (selection as { error: Record<string, unknown> }).error;
  }

  const windowId = (selection as { windowId: number }).windowId;
  const windowSnapshot = (snapshot.windows as WindowSnapshot[]).find((win) => win.windowId === windowId);
  if (!windowSnapshot) {
    throw new Error("Window snapshot unavailable");
  }

  // Resolve existing group for reuse
  let existingGroupId: number | null = null;
  const existingUrlSet = new Set<string>();
  if (groupTitle && !forceNewGroup) {
    const matchingGroups = windowSnapshot.groups.filter((g) => g.title === groupTitle);
    if (matchingGroups.length > 1) {
      throw new Error(
        `Ambiguous group title "${groupTitle}": found ${matchingGroups.length} groups with the same name. Use --new-group to force a new group, group-gather to merge, or --group-id to target by ID.`,
      );
    }
    if (matchingGroups.length === 1) {
      existingGroupId = matchingGroups[0].groupId as number;
      const existingTabs = windowSnapshot.tabs.filter((tab) => tab.groupId === existingGroupId);
      for (const tab of existingTabs) {
        const norm = normalizeUrl(tab.url);
        if (norm) {
          existingUrlSet.add(norm);
        }
      }
    }
  }

  const created: Array<Record<string, unknown>> = [];
  const skipped: Array<Record<string, unknown>> = [];
  let insertIndex: number | null = null;

  if (afterGroupTitle) {
    const targetGroup = windowSnapshot.groups.find((group) => group.title === afterGroupTitle);
    if (!targetGroup) {
      throw new Error("Group not found in target window");
    }
    const groupTabs = windowSnapshot.tabs.filter((tab) => tab.groupId === targetGroup.groupId);
    if (!groupTabs.length) {
      throw new Error("Group has no tabs to anchor insertion");
    }
    const indices = groupTabs
      .map((tab) => normalizeTabIndex(tab.index))
      .filter((value): value is number => value != null);
    if (!indices.length) {
      throw new Error("Group tabs missing indices");
    }
    insertIndex = Math.max(...indices) + 1;
  }

  if (beforeTabId != null || afterTabId != null) {
    if (afterGroupTitle) {
      throw new Error("Only one target position is allowed");
    }
    const anchorId = beforeTabId != null ? beforeTabId : afterTabId as number;
    const anchorTab = windowSnapshot.tabs.find((tab) => tab.tabId === anchorId);
    if (!anchorTab) {
      throw new Error("Anchor tab not found in target window");
    }
    const anchorIndex = normalizeTabIndex(anchorTab.index);
    if (!Number.isFinite(anchorIndex)) {
      throw new Error("Anchor tab index unavailable");
    }
    insertIndex = beforeTabId != null ? anchorIndex : anchorIndex + 1;
  }

  // Default insert position: append after existing group tabs
  if (existingGroupId != null && insertIndex == null && beforeTabId == null && afterTabId == null) {
    const groupTabs = windowSnapshot.tabs.filter((tab) => tab.groupId === existingGroupId);
    const indices = groupTabs
      .map((tab) => normalizeTabIndex(tab.index))
      .filter((value): value is number => value != null);
    if (indices.length) {
      insertIndex = Math.max(...indices) + 1;
    }
  }

  let nextIndex = insertIndex;
  for (const url of urls) {
    if (!allowDuplicates && existingGroupId != null) {
      const norm = normalizeUrl(url);
      if (norm && existingUrlSet.has(norm)) {
        skipped.push({ url, reason: "duplicate" });
        continue;
      }
    }
    try {
      const createOptions: chrome.tabs.CreateProperties = { windowId, url, active: false };
      if (nextIndex != null) {
        createOptions.index = nextIndex;
        nextIndex += 1;
      }
      const tab = await chrome.tabs.create(createOptions);
      created.push({
        tabId: tab.id,
        windowId: tab.windowId,
        index: tab.index,
        url: tab.url,
        title: tab.title,
      });
    } catch (error) {
      skipped.push({ url, reason: "create_failed" });
    }
  }
  const createdTabIds = new Set(
    created
      .map((tab) => tab.tabId)
      .filter((id): id is number => typeof id === "number"),
  );

  let groupId: number | null = null;
  if (groupTitle && created.length > 0) {
    try {
      const tabIds = created.map((tab) => tab.tabId as number).filter((id) => typeof id === "number");
      if (tabIds.length > 0) {
        if (existingGroupId != null) {
          // Reuse existing group
          groupId = await chrome.tabs.group({ groupId: existingGroupId, tabIds });
        } else {
          groupId = await chrome.tabs.group({ tabIds, createProperties: { windowId } });
          const update: chrome.tabGroups.UpdateProperties = { title: groupTitle };
          if (groupColor) {
            update.color = groupColor as chrome.tabGroups.ColorEnum;
          }
          await chrome.tabGroups.update(groupId, update);
        }
      }
    } catch (error) {
      deps.log("Failed to create group", error);
      groupId = null;
    }
  }

  // All-dupes case: report existing group even when no new tabs were created
  if (groupTitle && created.length === 0 && existingGroupId != null) {
    groupId = existingGroupId;
  }

  const targetGroupId = groupId ?? existingGroupId;
  if (targetGroupId != null && created.length > 0) {
    try {
      if (createdTabIds.size > 0) {
        const latestTabs = await chrome.tabs.query({ windowId });
        const missingGroupTabIds = latestTabs
          .filter((tab) => typeof tab.id === "number" && createdTabIds.has(tab.id) && tab.groupId !== targetGroupId)
          .map((tab) => tab.id as number);
        if (missingGroupTabIds.length > 0) {
          await chrome.tabs.group({ groupId: targetGroupId, tabIds: missingGroupTabIds });
        }
      }
    } catch (error) {
      deps.log("Failed to enforce grouping for newly opened tabs", error);
    }
  }

  try {
    const freshTabs = await chrome.tabs.query({ windowId });
    freshTabs.sort((a, b) => a.index - b.index);

    const firstUngroupedIndex = freshTabs.findIndex(t => (t.groupId ?? -1) === -1);
    if (firstUngroupedIndex >= 0) {
      let tabIdsToMove: number[] = [];
      if (existingGroupId != null) {
        // In reuse mode, only reorder tabs created by this operation.
        tabIdsToMove = freshTabs
          .filter((tab, i) => i > firstUngroupedIndex && typeof tab.id === "number" && createdTabIds.has(tab.id) && (tab.groupId ?? -1) !== -1)
          .map((tab) => tab.id as number);
      } else {
        tabIdsToMove = freshTabs
          .filter((tab, i) => i > firstUngroupedIndex && (tab.groupId ?? -1) !== -1)
          .map((tab) => tab.id!)
          .filter((id): id is number => typeof id === "number");
      }
      if (tabIdsToMove.length > 0) {
        await chrome.tabs.move(tabIdsToMove, { index: firstUngroupedIndex });
      }
    }
  } catch (err) {
    deps.log("Failed to reorder groups before ungrouped tabs", err);
  }
  if (targetGroupId != null && createdTabIds.size > 0) {
    try {
      const latestTabs = await chrome.tabs.query({ windowId });
      const lateUngroupedTabIds = latestTabs
        .filter((tab) => typeof tab.id === "number" && createdTabIds.has(tab.id) && tab.groupId !== targetGroupId)
        .map((tab) => tab.id as number);
      if (lateUngroupedTabIds.length > 0) {
        await chrome.tabs.group({ groupId: targetGroupId, tabIds: lateUngroupedTabIds });
      }
    } catch (error) {
      deps.log("Failed post-reorder grouping verification", error);
    }
  }
  if (targetGroupId != null && createdTabIds.size > 0) {
    try {
      await deps.delay(250);
      const delayedTabs = await chrome.tabs.query({ windowId });
      const delayedUngroupedTabIds = delayedTabs
        .filter((tab) => typeof tab.id === "number" && createdTabIds.has(tab.id) && tab.groupId !== targetGroupId)
        .map((tab) => tab.id as number);
      if (delayedUngroupedTabIds.length > 0) {
        await chrome.tabs.group({ groupId: targetGroupId, tabIds: delayedUngroupedTabIds });
      }
    } catch (error) {
      deps.log("Failed delayed grouping verification", error);
    }
  }

  return shapeOpenResult({
    windowId,
    groupId,
    created,
    skipped,
    summary: {
      createdTabs: created.length,
      skippedUrls: skipped.length,
      grouped: Boolean(groupId),
    },
  });
}
