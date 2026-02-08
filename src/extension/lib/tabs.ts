// Tab operations — extracted from background.ts (pure structural refactor).

type WindowSnapshot = import("./groups").WindowSnapshot;
type GroupMatch = import("./groups").GroupMatch;

export interface TabDeps {
  getTabSnapshot: () => Promise<{ generatedAt: number; windows: Array<Record<string, unknown>> }>;
  selectTabsByScope: (
    snapshot: { windows: Array<Record<string, unknown>> },
    params: Record<string, unknown>,
  ) => { tabs: Array<Record<string, unknown>>; error?: Record<string, unknown> };
  sendProgress: (id: string, payload: Record<string, unknown>) => void;
  log: (...args: Array<unknown>) => void;
  resolveWindowIdFromParams: (snapshot: { windows: Array<Record<string, unknown>> }, value: unknown) => number | null;
  resolveGroupByTitle: (snapshot: { windows: Array<Record<string, unknown>> }, groupTitle: string, windowId?: number) => { match?: GroupMatch; error?: Record<string, unknown> };
  resolveGroupById: (snapshot: { windows: Array<Record<string, unknown>> }, groupId: number) => { match?: GroupMatch; error?: Record<string, unknown> };
}

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

export function normalizeUrl(rawUrl: unknown) {
  if (!rawUrl || typeof rawUrl !== "string") {
    return null;
  }
  let url: URL;
  try {
    url = new URL(rawUrl);
  } catch {
    return null;
  }

  if (url.protocol !== "http:" && url.protocol !== "https:") {
    return null;
  }

  url.hash = "";
  const dropKeys = new Set([
    "fbclid",
    "gclid",
    "igshid",
    "mc_cid",
    "mc_eid",
    "ref",
    "ref_src",
    "ref_url",
    "utm_campaign",
    "utm_content",
    "utm_medium",
    "utm_source",
    "utm_term",
    "utm_name",
    "si",
  ]);
  for (const key of Array.from(url.searchParams.keys())) {
    if (key.startsWith("utm_") || dropKeys.has(key)) {
      url.searchParams.delete(key);
    }
  }

  const search = url.searchParams.toString();
  url.search = search ? `?${search}` : "";
  return url.toString();
}

export function normalizeTabIndex(value: unknown) {
  const index = Number(value);
  return Number.isFinite(index) ? index : null;
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

export function resolveMoveTarget(snapshot: { windows: Array<Record<string, unknown>> }, params: Record<string, unknown>, deps: Pick<TabDeps, "resolveGroupByTitle">) {
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
  const tabId = Number.isFinite(params.tabId as number)
    ? Number(params.tabId)
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

export async function openTabs(params: Record<string, unknown>, deps: Pick<TabDeps, "getTabSnapshot" | "log">) {
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

    return {
      windowId,
      groupId,
      groupTitle: groupTitle || null,
      afterGroupTitle: null,
      insertIndex: null,
      created,
      skipped,
      summary: {
        createdTabs: created.length,
        skippedUrls: skipped.length,
        grouped: Boolean(groupId),
      },
    };
  }

  const snapshot = await deps.getTabSnapshot();
  let openParams = params;
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

  let nextIndex = insertIndex;
  for (const url of urls) {
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

  return {
    windowId,
    groupId,
    groupTitle: groupTitle || null,
    afterGroupTitle: afterGroupTitle || null,
    insertIndex,
    created,
    skipped,
    summary: {
      createdTabs: created.length,
      skippedUrls: skipped.length,
      grouped: Boolean(groupId),
    },
  };
}

export async function moveTab(params: Record<string, unknown>, deps: Pick<TabDeps, "getTabSnapshot" | "resolveWindowIdFromParams" | "resolveGroupByTitle">) {
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

export async function moveGroup(params: Record<string, unknown>, deps: Pick<TabDeps, "getTabSnapshot" | "log" | "resolveWindowIdFromParams" | "resolveGroupByTitle" | "resolveGroupById">) {
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
