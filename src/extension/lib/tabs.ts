// Tab operations — extracted from background.ts (pure structural refactor).

type WindowSnapshot = import("./groups").WindowSnapshot;

import type { ExtensionDeps } from "./deps";
import normalizeUrlLib from "normalize-url";

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
      stripWWW: false,
      removeTrailingSlash: false,
      removeSingleSlash: false,
      sortQueryParameters: true,
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

export async function openTabs(params: Record<string, unknown>, deps: Pick<ExtensionDeps, "getTabSnapshot" | "log">) {
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

  // When groupTitle is set and not forcing a new group, try to resolve
  // the target window via the group name so we land in the right window.
  if (groupTitle && !forceNewGroup && openParams.windowId == null && !openParams.windowGroupTitle && !openParams.windowTabId && !openParams.windowUrl) {
    const groupWindows = (snapshot.windows as WindowSnapshot[]).filter(
      (win) => win.groups.some((g) => g.title === groupTitle),
    );
    if (groupWindows.length === 1) {
      openParams = { ...openParams, windowId: groupWindows[0].windowId };
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
  const existingGroupUrls = new Set<string>();
  if (groupTitle && !forceNewGroup) {
    const matchingGroups = windowSnapshot.groups.filter((g) => g.title === groupTitle);
    if (matchingGroups.length > 1) {
      throw new Error(
        `Ambiguous group title "${groupTitle}": found ${matchingGroups.length} groups with the same name. Use --new-group to create a new group instead.`,
      );
    }
    const matchingGroup = matchingGroups[0];
    if (matchingGroup) {
      existingGroupId = matchingGroup.groupId as number;
      if (!allowDuplicates) {
        const groupTabs = windowSnapshot.tabs.filter((tab) => tab.groupId === existingGroupId);
        for (const tab of groupTabs) {
          const normalized = normalizeUrl(tab.url);
          if (normalized) {
            existingGroupUrls.add(normalized);
          }
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

  // When reusing an existing group and no explicit position given,
  // insert after the last tab in that group.
  if (existingGroupId != null && insertIndex == null && beforeTabId == null && afterTabId == null) {
    const groupTabs = windowSnapshot.tabs.filter((tab) => tab.groupId === existingGroupId);
    const indices = groupTabs
      .map((tab) => normalizeTabIndex(tab.index))
      .filter((value): value is number => value != null);
    if (indices.length) {
      insertIndex = Math.max(...indices) + 1;
    }
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
    // Skip duplicates when reusing a group
    if (!allowDuplicates && existingGroupId != null) {
      const normalized = normalizeUrl(url);
      if (normalized && existingGroupUrls.has(normalized)) {
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

  let groupId: number | null = null;
  if (groupTitle && created.length > 0) {
    try {
      const tabIds = created.map((tab) => tab.tabId as number).filter((id) => typeof id === "number");
      if (tabIds.length > 0) {
        if (existingGroupId != null) {
          // Reuse existing group
          await chrome.tabs.group({ groupId: existingGroupId, tabIds });
          groupId = existingGroupId;
        } else {
          // Create new group
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
  } else if (groupTitle && created.length === 0 && existingGroupId != null) {
    // All URLs were duplicates, still report the existing group
    groupId = existingGroupId;
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


