/**
 * MCP tool definitions for tabctl.
 *
 * Architecture: "CLI-through" — each MCP tool maps 1:1 to an existing CLI
 * action. Tool handlers call sendRequest() with the same params the CLI uses,
 * then format the response as compact text suitable for LLM context windows.
 *
 * This avoids maintaining two separate APIs: the extension/host handlers are
 * the single source of truth, and both CLI and MCP share the same transport.
 */

import { z } from "zod";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { sendRequest, createRequestId } from "./client";
import { VERSION, BASE_VERSION, GIT_SHA, DIRTY } from "./constants";

// ============================================================================
// Constants
// ============================================================================

const MCP_DEFAULT_LIMIT = 20;
const MCP_MAX_LIMIT = 50;
const MAX_OUTPUT_CHARS = 4000;

// ============================================================================
// Helpers
// ============================================================================

function clientMeta() {
  return {
    component: "mcp",
    version: VERSION,
    baseVersion: BASE_VERSION,
    gitSha: GIT_SHA,
    dirty: DIRTY,
  };
}

async function callAction(
  action: string,
  params: Record<string, unknown>
): Promise<Record<string, unknown>> {
  return sendRequest({
    id: createRequestId(),
    action,
    params,
    client: clientMeta(),
  });
}

function clampLimit(value: number | undefined, max = MCP_MAX_LIMIT): number {
  if (value == null || value <= 0) return MCP_DEFAULT_LIMIT;
  return Math.min(value, max);
}

export function truncate(text: string, maxLen = MAX_OUTPUT_CHARS): string {
  if (text.length <= maxLen) return text;
  return text.slice(0, maxLen - 20) + "\n… (output truncated)";
}

function formatTab(tab: Record<string, unknown>): string {
  const id = tab.id ?? tab.tabId ?? "?";
  const title = String(tab.title || "Untitled").slice(0, 80);
  const url = String(tab.url || "").slice(0, 120);
  return `[Tab ${id}] ${title} — ${url}`;
}

function formatError(response: Record<string, unknown>): string {
  const err = response.error as Record<string, unknown> | undefined;
  if (err) {
    const msg = String(err.message || "Unknown error");
    const hint = err.hint ? `\nHint: ${err.hint}` : "";
    return `Error: ${msg}${hint}`;
  }
  return "Error: Unknown error";
}

// ============================================================================
// Response Formatters
// ============================================================================

export function formatListResponse(
  response: Record<string, unknown>,
  limit: number
): string {
  if (!response.ok) return formatError(response);

  const data = response.data as Record<string, unknown>;
  const windows = (data?.windows as Array<Record<string, unknown>>) || [];
  const lines: string[] = [];
  let count = 0;

  for (const win of windows) {
    const winId = win.id ?? win.windowId ?? "?";
    const tabs = (win.tabs as Array<Record<string, unknown>>) || [];
    lines.push(`Window ${winId} (${tabs.length} tabs):`);
    for (const tab of tabs) {
      if (count >= limit) break;
      lines.push(`  ${formatTab(tab)}`);
      count++;
    }
    if (count >= limit) break;
  }

  if (count === 0) {
    lines.push("No tabs found.");
  }

  const page = data?.page as Record<string, unknown> | undefined;
  if (page?.hasMore) {
    lines.push(`\nMore results available (offset: ${page.nextOffset})`);
  }

  return truncate(lines.join("\n"));
}

export function formatGroupListResponse(
  response: Record<string, unknown>,
  limit: number
): string {
  if (!response.ok) return formatError(response);

  const data = response.data as Record<string, unknown>;
  const groups = (data?.groups as Array<Record<string, unknown>>) || [];
  const lines: string[] = [];

  const shown = groups.slice(0, limit);
  for (const group of shown) {
    const id = group.id ?? group.groupId ?? "?";
    const title = String(group.title || "Untitled");
    const tabCount = group.tabCount ?? "?";
    const color = group.color ? ` (${group.color})` : "";
    const windowId = group.windowId ? ` in window ${group.windowId}` : "";
    lines.push(`[Group ${id}] ${title}${color} — ${tabCount} tabs${windowId}`);
  }

  if (lines.length === 0) {
    lines.push("No groups found.");
  }

  if (groups.length > limit) {
    lines.push(`\n… and ${groups.length - limit} more groups`);
  }

  return truncate(lines.join("\n"));
}

export function formatAnalyzeResponse(
  response: Record<string, unknown>,
  limit: number
): string {
  if (!response.ok) return formatError(response);

  const data = response.data as Record<string, unknown>;
  const summary = data?.summary as Record<string, unknown> | undefined;
  const candidates = (data?.candidates as Array<Record<string, unknown>>) || [];
  const lines: string[] = [];

  if (summary) {
    lines.push("Analysis Summary:");
    for (const [key, value] of Object.entries(summary)) {
      lines.push(`  ${key}: ${value}`);
    }
    lines.push("");
  }

  const shown = candidates.slice(0, limit);
  if (shown.length > 0) {
    lines.push(`Top ${shown.length} candidates:`);
    for (const c of shown) {
      const reasons = (c.reasons as Array<Record<string, unknown>>) || [];
      const reasonStr = reasons.map((r) => r.type || r.reason).join(", ");
      lines.push(`  ${formatTab(c)} — ${reasonStr}`);
    }
  } else {
    lines.push("No candidates found.");
  }

  if (candidates.length > limit) {
    lines.push(`\n… and ${candidates.length - limit} more candidates`);
  }

  return truncate(lines.join("\n"));
}

export function formatInspectResponse(
  response: Record<string, unknown>
): string {
  if (!response.ok) return formatError(response);

  const data = response.data as Record<string, unknown>;
  const entries = (data?.entries as Array<Record<string, unknown>>) || [];
  const lines: string[] = [];

  for (const entry of entries) {
    const tab = entry.tab as Record<string, unknown> | undefined;
    if (tab) {
      lines.push(formatTab(tab));
    }
    const signals = entry.signals as Record<string, unknown> | undefined;
    if (signals) {
      for (const [signalName, signalData] of Object.entries(signals)) {
        const formatted = typeof signalData === "string"
          ? signalData.slice(0, 200)
          : JSON.stringify(signalData, null, 2).slice(0, 500);
        lines.push(`  ${signalName}: ${formatted}`);
      }
    }
  }

  if (lines.length === 0) {
    lines.push("No inspection results.");
  }

  return truncate(lines.join("\n"));
}

export function formatSimpleResponse(
  response: Record<string, unknown>,
  action: string
): string {
  if (!response.ok) return formatError(response);

  const data = response.data as Record<string, unknown> | undefined;
  if (data) {
    const summary = data.summary as Record<string, unknown> | undefined;
    if (summary) {
      const parts = Object.entries(summary).map(([k, v]) => `${k}: ${v}`);
      return `${action} succeeded.\n${parts.join("\n")}`;
    }
    if (data.txid) {
      return `${action} succeeded (txid: ${data.txid}). Use undo tool with this txid to reverse.`;
    }
  }

  return `${action} succeeded.`;
}

// ============================================================================
// Tool Registration
// ============================================================================

export function registerTools(server: McpServer): void {
  // -- list_tabs --
  server.tool(
    "list_tabs",
    "List browser tabs. Returns tab IDs, titles, and URLs. Use windowId to scope to a specific window.",
    {
      windowId: z.number().optional().describe("Window ID to list tabs from"),
      groupTitle: z.string().optional().describe("Filter by group name"),
      limit: z.number().min(1).max(MCP_MAX_LIMIT).optional().describe("Max tabs to return (default 20, max 50)"),
      offset: z.number().min(0).optional().describe("Skip first N tabs for pagination"),
    },
    async (params) => {
      const limit = clampLimit(params.limit);
      const response = await callAction("list", {
        windowId: params.windowId,
        groupTitle: params.groupTitle,
        all: !params.windowId && !params.groupTitle,
      });
      return {
        content: [{ type: "text" as const, text: formatListResponse(response, limit) }],
      };
    }
  );

  // -- list_groups --
  server.tool(
    "list_groups",
    "List tab groups with their IDs, titles, colors, and tab counts.",
    {
      windowId: z.number().optional().describe("Window ID to list groups from"),
      limit: z.number().min(1).max(MCP_MAX_LIMIT).optional().describe("Max groups to return (default 20, max 50)"),
    },
    async (params) => {
      const limit = clampLimit(params.limit);
      const response = await callAction("group-list", {
        windowId: params.windowId,
      });
      return {
        content: [{ type: "text" as const, text: formatGroupListResponse(response, limit) }],
      };
    }
  );

  // -- analyze_tabs --
  server.tool(
    "analyze_tabs",
    "Analyze tabs for duplicates and stale content. Returns a summary and candidate tabs.",
    {
      windowId: z.number().optional().describe("Window ID to analyze"),
      staleDays: z.number().optional().describe("Days threshold for stale tabs"),
      limit: z.number().min(1).max(MCP_MAX_LIMIT).optional().describe("Max candidates to show (default 20, max 50)"),
    },
    async (params) => {
      const limit = clampLimit(params.limit);
      const response = await callAction("analyze", {
        windowId: params.windowId,
        staleDays: params.staleDays,
        all: !params.windowId,
      });
      return {
        content: [{ type: "text" as const, text: formatAnalyzeResponse(response, limit) }],
      };
    }
  );

  // -- inspect_tab --
  server.tool(
    "inspect_tab",
    "Extract metadata from a single tab. Signals: page-meta (description + h1), selector (CSS selectors).",
    {
      tabId: z.number().describe("Tab ID to inspect"),
      signal: z.enum(["page-meta", "selector"]).optional().describe("Signal to extract (default: page-meta)"),
      selector: z.string().optional().describe("CSS selector (when signal is 'selector'), e.g. 'links=a[href]'"),
    },
    async (params) => {
      const signals = [params.signal || "page-meta"];
      const actionParams: Record<string, unknown> = {
        tabIds: [params.tabId],
        signals,
      };
      if (params.selector && signals.includes("selector")) {
        const parts = params.selector.includes("=")
          ? params.selector.split(/=(.+)/)
          : [params.selector, params.selector];
        actionParams.selectorSpecs = [{ name: parts[0], selector: parts[1] || parts[0] }];
      }
      const response = await callAction("inspect", actionParams);
      return {
        content: [{ type: "text" as const, text: formatInspectResponse(response) }],
      };
    }
  );

  // -- ping --
  server.tool(
    "ping",
    "Test connection to the browser extension. Returns ok if connected.",
    {},
    async () => {
      const response = await callAction("ping", {});
      return {
        content: [{ type: "text" as const, text: formatSimpleResponse(response, "ping") }],
      };
    }
  );

  // -- history --
  server.tool(
    "history",
    "Show recent operation history with transaction IDs for undo.",
    {
      limit: z.number().min(1).max(MCP_MAX_LIMIT).optional().describe("Max entries (default 10)"),
    },
    async (params) => {
      const response = await callAction("history", {
        limit: params.limit || 10,
      });
      if (!response.ok) {
        return { content: [{ type: "text" as const, text: formatError(response) }] };
      }
      const data = response.data as Array<Record<string, unknown>> | undefined;
      if (!Array.isArray(data) || data.length === 0) {
        return { content: [{ type: "text" as const, text: "No history entries." }] };
      }
      const lines = data.map((entry) => {
        const txid = entry.txid || "?";
        const action = entry.action || "?";
        const summary = entry.summary as Record<string, unknown> | undefined;
        const summaryStr = summary
          ? Object.entries(summary).map(([k, v]) => `${k}=${v}`).join(", ")
          : "";
        return `${txid} — ${action} ${summaryStr}`.trim();
      });
      return {
        content: [{ type: "text" as const, text: truncate(lines.join("\n")) }],
      };
    }
  );

  // -- open_tabs --
  server.tool(
    "open_tabs",
    "Open new tabs with URLs, optionally in a named group.",
    {
      urls: z.array(z.string().url()).min(1).max(10).describe("URLs to open (max 10)"),
      group: z.string().optional().describe("Group name to add tabs to"),
      color: z.enum(["grey", "blue", "red", "yellow", "green", "pink", "purple", "cyan", "orange"]).optional().describe("Group color"),
      windowId: z.number().optional().describe("Target window ID"),
      newWindow: z.boolean().optional().describe("Open in a new window"),
    },
    async (params) => {
      const response = await callAction("open", {
        urls: params.urls,
        groupTitle: params.group,
        color: params.color,
        windowId: params.windowId,
        newWindow: params.newWindow,
      });
      return {
        content: [{ type: "text" as const, text: formatSimpleResponse(response, "open") }],
      };
    }
  );

  // -- focus_tab --
  server.tool(
    "focus_tab",
    "Focus (switch to) a specific tab by its ID.",
    {
      tabId: z.number().describe("Tab ID to focus"),
    },
    async (params) => {
      const response = await callAction("focus", {
        tabId: params.tabId,
        tabIds: [params.tabId],
      });
      return {
        content: [{ type: "text" as const, text: formatSimpleResponse(response, "focus") }],
      };
    }
  );

  // -- close_tabs --
  server.tool(
    "close_tabs",
    "Close specific tabs by their IDs. Maximum 10 tabs per call. Returns a transaction ID for undo.",
    {
      tabIds: z.array(z.number()).min(1).max(10).describe("Tab IDs to close (max 10)"),
    },
    async (params) => {
      const response = await callAction("close", {
        mode: "direct",
        confirmed: true,
        tabIds: params.tabIds,
      });
      return {
        content: [{ type: "text" as const, text: formatSimpleResponse(response, "close") }],
      };
    }
  );

  // -- group_update --
  server.tool(
    "group_update",
    "Update a tab group's title, color, or collapsed state.",
    {
      groupId: z.number().optional().describe("Group ID to update"),
      groupTitle: z.string().optional().describe("Target group by current title"),
      windowId: z.number().optional().describe("Window ID to disambiguate group title"),
      title: z.string().optional().describe("New group title"),
      color: z.enum(["grey", "blue", "red", "yellow", "green", "pink", "purple", "cyan", "orange"]).optional().describe("New group color"),
      collapsed: z.boolean().optional().describe("Collapse or expand the group"),
    },
    async (params) => {
      const response = await callAction("group-update", {
        groupId: params.groupId,
        groupTitle: params.groupTitle,
        windowId: params.windowId,
        title: params.title,
        color: params.color,
        collapsed: params.collapsed,
      });
      return {
        content: [{ type: "text" as const, text: formatSimpleResponse(response, "group-update") }],
      };
    }
  );

  // -- group_assign --
  server.tool(
    "group_assign",
    "Assign tabs to a group. Can create the group if it doesn't exist.",
    {
      tabIds: z.array(z.number()).min(1).max(20).describe("Tab IDs to assign"),
      groupTitle: z.string().optional().describe("Target group by title"),
      groupId: z.number().optional().describe("Target group by ID"),
      windowId: z.number().optional().describe("Window ID to disambiguate"),
      create: z.boolean().optional().describe("Create group if it doesn't exist"),
      color: z.enum(["grey", "blue", "red", "yellow", "green", "pink", "purple", "cyan", "orange"]).optional().describe("Group color if creating"),
    },
    async (params) => {
      const response = await callAction("group-assign", {
        tabIds: params.tabIds,
        groupTitle: params.groupTitle,
        groupId: params.groupId,
        windowId: params.windowId,
        create: params.create,
        color: params.color,
      });
      return {
        content: [{ type: "text" as const, text: formatSimpleResponse(response, "group-assign") }],
      };
    }
  );

  // -- undo --
  server.tool(
    "undo",
    "Undo a previous operation by transaction ID, or undo the most recent operation.",
    {
      txid: z.string().optional().describe("Transaction ID to undo (from history)"),
      latest: z.boolean().optional().describe("Undo the most recent operation"),
    },
    async (params) => {
      if (!params.txid && !params.latest) {
        return {
          content: [{ type: "text" as const, text: "Error: provide txid or set latest to true" }],
          isError: true,
        };
      }
      const response = await callAction("undo", {
        txid: params.txid,
        latest: params.latest,
      });
      return {
        content: [{ type: "text" as const, text: formatSimpleResponse(response, "undo") }],
      };
    }
  );
}
