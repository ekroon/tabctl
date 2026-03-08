// Pure helper functions extracted from background.ts — independently testable, no chrome dependency.

export type IncognitoState = {
  incognitoWindowIds: Set<number>;
  incognitoTabIds: Set<number>;
  incognitoGroupIds: Set<number>;
};

export type SnapshotWindow = {
  windowId?: number;
  incognito?: boolean;
  tabs?: Array<{ tabId?: number }>;
  groups?: Array<{ groupId?: number }>;
};

export function parseVersionName(versionName: string): { gitSha: string | null; dirty: boolean } {
  const match = versionName.match(/-dev\.([0-9a-f]+)(\.dirty)?$/i);
  if (!match) {
    return { gitSha: null, dirty: false };
  }
  return { gitSha: match[1], dirty: Boolean(match[2]) };
}

export function requireFiniteId(value: unknown, name: string): number {
  const n = Number(value);
  if (!Number.isFinite(n)) throw new Error(`${name} must be a finite number, got: ${String(value)}`);
  return n;
}

export function inferIncognitoEvent(payload: Record<string, unknown>, state: IncognitoState): boolean {
  const tabId = typeof payload.tabId === "number" ? payload.tabId : null;
  if (tabId !== null && state.incognitoTabIds.has(tabId)) {
    return true;
  }
  const groupId = typeof payload.groupId === "number" ? payload.groupId : null;
  if (groupId !== null && state.incognitoGroupIds.has(groupId)) {
    return true;
  }
  const windowId = typeof payload.windowId === "number" ? payload.windowId : null;
  return windowId !== null && state.incognitoWindowIds.has(windowId);
}

export function normalizeEventPayload(
  kind: string,
  payload: Record<string, unknown>,
  state: IncognitoState,
): Record<string, unknown> {
  const event: Record<string, unknown> = {
    kind,
    occurredAt: Date.now(),
  };
  for (const [key, value] of Object.entries(payload)) {
    if (value === undefined) {
      continue;
    }
    event[key] = value;
  }
  if (event.incognito !== true && inferIncognitoEvent(payload, state)) {
    event.incognito = true;
  }
  return event;
}

export function updateIncognitoState(
  snapshot: { windows?: Array<SnapshotWindow> },
  state: IncognitoState,
): void {
  state.incognitoWindowIds.clear();
  state.incognitoTabIds.clear();
  state.incognitoGroupIds.clear();
  for (const window of snapshot.windows || []) {
    if (window.incognito !== true || typeof window.windowId !== "number") {
      continue;
    }
    state.incognitoWindowIds.add(window.windowId);
    for (const tab of window.tabs || []) {
      if (typeof tab.tabId === "number") {
        state.incognitoTabIds.add(tab.tabId);
      }
    }
    for (const group of window.groups || []) {
      if (typeof group.groupId === "number") {
        state.incognitoGroupIds.add(group.groupId);
      }
    }
  }
}
