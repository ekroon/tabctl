import { evaluateTab } from "./policy";
import { selectTabsFromSnapshot } from "./scope";
import type { PolicyContext } from "./policy";

export type PolicyFilterResult = {
  params: Record<string, unknown>;
  policyInfo: Record<string, unknown> | null;
  earlyResponse: Record<string, unknown> | null;
};

function mapProtectedTab(tab: Record<string, unknown>) {
  return {
    tabId: tab.tabId,
    windowId: tab.windowId,
    groupId: tab.groupId,
    groupTitle: tab.groupTitle,
    title: tab.title,
    url: tab.url,
    pinned: tab.pinned,
  };
}

export function applyPolicyFilter(
  command: string,
  params: Record<string, unknown>,
  snapshot: Record<string, unknown>,
  policyContext: PolicyContext,
  policySummary: Record<string, unknown>,
): PolicyFilterResult {
  const policy = policyContext.policy;

  const selection = selectTabsFromSnapshot(snapshot, params);
  if ((selection as { error?: Record<string, unknown> }).error) {
    return {
      params,
      policyInfo: null,
      earlyResponse: {
        ok: false,
        error: (selection as { error: Record<string, unknown> }).error,
      },
    };
  }

  const selectedTabs = (selection as { tabs: Array<Record<string, unknown>> }).tabs;
  const eligibleTabs = selectedTabs.filter((tab) => evaluateTab(tab, policy).eligible);
  const protectedTabs = selectedTabs.filter((tab) => !evaluateTab(tab, policy).eligible);
  const eligibleIds = eligibleTabs.map((tab) => tab.tabId).filter((id) => typeof id === "number") as number[];

  let earlyResponse: Record<string, unknown> | null = null;
  let policyInfo: Record<string, unknown> | null = null;
  let newParams = params;

  if (command === "focus" || command === "refresh") {
    if (!eligibleIds.length) {
      return {
        params,
        policyInfo: null,
        earlyResponse: {
          ok: false,
          error: { message: `Tab is protected by policy and cannot be ${command === "focus" ? "focused" : "refreshed"} via CLI` },
        },
      };
    }
    newParams = {
      tabId: eligibleIds[0],
    };
  } else if (command === "close" || command === "archive") {
    if (!eligibleIds.length) {
      earlyResponse = {
        ok: true,
        action: command,
        data: {
          summary: { eligible: 0, protected: protectedTabs.length },
          protected: protectedTabs.map(mapProtectedTab),
          policy: policySummary,
        },
      };
    } else if (command === "close") {
      newParams = {
        mode: "direct",
        confirmed: true,
        tabIds: eligibleIds,
      };
    } else if (command === "archive") {
      newParams = {
        tabIds: eligibleIds,
      };
    }

    policyInfo = {
      protected: protectedTabs.map(mapProtectedTab),
    };
  } else if (command === "move-tab" || command === "move-group" || command === "group-assign") {
    if (!eligibleIds.length || (command === "move-group" && protectedTabs.length > 0)) {
      earlyResponse = {
        ok: true,
        action: command,
        data: {
          summary: { eligible: eligibleIds.length, protected: protectedTabs.length },
          protected: protectedTabs.map(mapProtectedTab),
          policy: policySummary,
        },
      };
    } else if (command === "move-tab" || command === "group-assign") {
      newParams = {
        ...params,
        tabId: eligibleIds[0],
        tabIds: eligibleIds,
      };
    }

    policyInfo = {
      protected: protectedTabs.map(mapProtectedTab),
    };
  } else if (command === "merge-window") {
    if (!eligibleIds.length) {
      earlyResponse = {
        ok: true,
        action: command,
        data: {
          summary: { eligible: 0, protected: protectedTabs.length },
          protected: protectedTabs.map(mapProtectedTab),
          policy: policySummary,
        },
      };
    }

    newParams = {
      ...params,
      tabIds: eligibleIds,
    };

    policyInfo = {
      protected: protectedTabs.map(mapProtectedTab),
    };
  } else if (command === "group-update" || command === "group-ungroup") {
    if (!eligibleIds.length || protectedTabs.length > 0) {
      earlyResponse = {
        ok: true,
        action: command,
        data: {
          summary: { eligible: eligibleIds.length, protected: protectedTabs.length },
          protected: protectedTabs.map(mapProtectedTab),
          policy: policySummary,
        },
      };
    }

    policyInfo = {
      protected: protectedTabs.map(mapProtectedTab),
    };
  } else {
    if (!eligibleIds.length) {
      const generatedAt = Date.now();
      if (command === "analyze") {
        earlyResponse = {
          ok: true,
          action: command,
          data: {
            generatedAt,
            staleDays: params.staleDays || 0,
            totals: { tabs: 0, analyzed: 0, candidates: 0 },
            meta: { durationMs: 0, githubChecked: 0, githubTotal: 0, githubMatched: 0, githubTimeoutMs: params.githubTimeoutMs || 0 },
            candidates: [],
            analysisId: null,
            policy: policySummary,
          },
        };
      } else if (command === "screenshot") {
        earlyResponse = {
          ok: true,
          action: command,
          data: {
            generatedAt,
            entries: [],
            totals: { tabs: 0, tiles: 0 },
            meta: {
              durationMs: 0,
              mode: params.mode || "viewport",
              format: params.format || "png",
              tileMaxDim: params.tileMaxDim || null,
              maxBytes: params.maxBytes || null,
            },
            policy: policySummary,
          },
        };
      } else {
        earlyResponse = {
          ok: true,
          action: command,
          data: {
            generatedAt,
            entries: [],
            totals: { tabs: 0, signals: 0, tasks: 0 },
            meta: { durationMs: 0, signalTimeoutMs: params.signalTimeoutMs || 0, selectorCount: 0 },
            policy: policySummary,
          },
        };
      }
    } else {
      newParams = {
        ...params,
        tabIds: eligibleIds,
      };
    }
  }

  return {
    params: newParams,
    policyInfo,
    earlyResponse,
  };
}
