/**
 * List command handlers: list, group-list
 */

import { VERSION, BASE_VERSION, GIT_SHA, DIRTY } from "../constants";
import { printJson, errorOut, emitVersionWarnings } from "../output";
import { sendRequest, createRequestId, fetchSnapshot } from "../client";
import { resolveScopeFlags, buildScopeArgs, selectTabsFromSnapshot, filterGroupsByScope } from "../scope";
import { resolvePagination } from "../pagination";
import {
  filterSnapshotByPolicy,
  buildTabIndex,
  buildGroupsFromSnapshot,
  orderTabs,
  buildPagedSnapshot,
} from "../snapshot";
import { evaluateTab, type Policy } from "../policy";
import type { Options, PolicyContext } from "../types";

// ============================================================================
// List Command
// ============================================================================

export async function runList(
  options: Options,
  policyContext: PolicyContext,
  policySummary: Record<string, unknown>,
  prettyOutput: boolean
): Promise<void> {
  const response = await sendRequest({
    id: createRequestId(),
    action: "list",
    params: {},
    client: {
      component: "cli",
      version: VERSION,
      baseVersion: BASE_VERSION,
      gitSha: GIT_SHA,
      dirty: DIRTY,
    },
  });

  if (!response.ok) {
    printJson(response, prettyOutput);
    process.exit(1);
  }

  const data = response.data as Record<string, unknown>;
  if (data && Array.isArray(data.windows)) {
    const filtered = filterSnapshotByPolicy(data, policyContext.policy) as Record<string, unknown>;
    const scope = resolveScopeFlags(options);
    const allScope = options.all === true || !scope.hasScope;
    const listParams: Record<string, unknown> = allScope
      ? { all: true }
      : {
          tabIds: scope.tabIds.length ? scope.tabIds : undefined,
          groupTitle: scope.groupTitle || undefined,
          groupId: scope.groupId != null ? scope.groupId : undefined,
          windowId: scope.windowId != null ? scope.windowId : undefined,
        };
    const selection = selectTabsFromSnapshot(filtered, listParams);
    if (selection.error) {
      printJson({ ok: false, error: selection.error }, prettyOutput);
      process.exit(1);
    }
    const selectedTabs = selection.tabs || [];
    const tabIdSet = new Set(
      selectedTabs.map((tab) => tab.tabId).filter((id) => typeof id === "number") as number[]
    );
    const ordered = listParams.all
      ? orderTabs(filtered, null)
      : tabIdSet.size > 0
        ? orderTabs(filtered, tabIdSet)
        : [];
    const scopeArgs = buildScopeArgs(options, allScope);
    const pagination = resolvePagination(options, ordered.length, "list", scopeArgs);
    const start = pagination.offset;
    const end = pagination.offset + pagination.limit;
    const pagedTabs = ordered.slice(start, end);
    const pagedSnapshot = buildPagedSnapshot(filtered, pagedTabs) as Record<string, unknown>;
    data.windows = pagedSnapshot.windows as Array<Record<string, unknown>>;
    if (pagination.page) {
      data.page = pagination.page;
    }
    data.policy = policySummary;
  } else if (response.ok) {
    (response as Record<string, unknown>).policy = policySummary;
  }

  emitVersionWarnings(response, "list");
  printJson(response, prettyOutput);
}

// ============================================================================
// Group-List Command
// ============================================================================

export async function runGroupList(
  options: Options,
  policyContext: PolicyContext,
  policySummary: Record<string, unknown>,
  prettyOutput: boolean
): Promise<void> {
  const params: Record<string, unknown> = {
    windowId: options.all ? undefined : (options.window ? Number(options.window) : undefined),
  };

  const response = await sendRequest({
    id: createRequestId(),
    action: "group-list",
    params,
    client: {
      component: "cli",
      version: VERSION,
      baseVersion: BASE_VERSION,
      gitSha: GIT_SHA,
      dirty: DIRTY,
    },
  });

  if (!response.ok) {
    printJson(response, prettyOutput);
    process.exit(1);
  }

  const data = response.data as Record<string, unknown>;

  // Fallback to snapshot if groups missing
  if (data && (!Array.isArray(data.groups) || data.groups === null)) {
    const scope = resolveScopeFlags(options);
    const snapshot = await fetchSnapshot();
    if (snapshot) {
      const filteredSnapshot = filterSnapshotByPolicy(snapshot, policyContext.policy) as Record<string, unknown>;
      const scopeWindow = Number.isFinite(scope.windowId) ? scope.windowId : null;
      const groups = buildGroupsFromSnapshot(filteredSnapshot, scopeWindow);
      data.groups = filterGroupsByScope(groups, scope, filteredSnapshot, buildTabIndex);
    } else {
      data.groups = [];
    }
  }

  // Apply scope filtering and pagination
  if (data && Array.isArray(data.groups)) {
    let groups = data.groups as Array<Record<string, unknown>>;
    const scope = resolveScopeFlags(options);
    const allScope = options.all === true || !scope.hasScope;

    let snapshot: Record<string, unknown> | null = null;
    if (!allScope && scope.tabIds.length > 0) {
      snapshot = await fetchSnapshot();
      if (!snapshot) {
        errorOut("Failed to load tabs for group-list filtering");
      }
    }

    if (!allScope) {
      groups = filterGroupsByScope(groups, scope, snapshot, buildTabIndex);
    }

    const scopeArgs = buildScopeArgs(options, allScope);
    const pagination = resolvePagination(options, groups.length, "group-list", scopeArgs);
    const start = pagination.offset;
    const end = pagination.offset + pagination.limit;
    data.groups = groups.slice(start, end);
    if (pagination.page) {
      data.page = pagination.page;
    }
    data.policy = policySummary;
  } else if (response.ok) {
    (response as Record<string, unknown>).policy = policySummary;
  }

  emitVersionWarnings(response, "group-list");
  printJson(response, prettyOutput);
}
