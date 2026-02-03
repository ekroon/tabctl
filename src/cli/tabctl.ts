#!/usr/bin/env node
import fs from "fs";
import path from "path";
import { renderCsv, renderMarkdown } from "./lib/report";
import { evaluateTab, loadPolicy, summarizePolicy } from "./lib/policy";
import { VERSION, BASE_VERSION, GIT_SHA, DIRTY } from "./lib/constants";
import { printJson, errorOut, setupStdoutErrorHandling, emitVersionWarnings } from "./lib/output";
import { parseArgs, normalizeSignals, validateSignals } from "./lib/args";
import { sendRequest, createRequestId, fetchSnapshot } from "./lib/client";
import { resolveScopeFlags, buildScopeArgs, selectTabsFromSnapshot } from "./lib/scope";
import { resolvePagination } from "./lib/pagination";
import { buildTabIndex, buildWindowTitleIndex } from "./lib/snapshot";
import { printHelp } from "./lib/help";
import { runSetup, runSkillInstall, runVersion, runPolicy, runList, runGroupList, runPing, runHistory, runUndo } from "./lib/commands";
import {
  buildAnalyzeParams,
  buildInspectParams,
  buildFocusParams,
  buildRefreshParams,
  buildOpenParams,
  buildGroupUpdateParams,
  buildGroupUngroupParams,
  buildGroupAssignParams,
  buildMoveTabParams,
  buildMoveGroupParams,
  buildMergeWindowParams,
  buildArchiveParams,
  buildCloseParams,
  buildReportParams,
  buildScreenshotParams,
} from "./lib/commands";
import type { Options } from "./lib/types";

const createId = createRequestId;

async function main() {
  setupStdoutErrorHandling();
  let { command, options, warnings } = parseArgs(process.argv.slice(2));
  if (command === "dedupe" && (options as Record<string, unknown>).close) {
    errorOut("dedupe does not support --close; use --confirm or close --apply <analysisId>.");
  }
  const prettyOutput = options.pretty !== false;
  if (command === "groups" || command === "group") {
    command = "group-list";
  }
  if (command === "list" && options.groups === true) {
    command = "group-list";
  }
  if (options.format && command !== "report" && command !== "screenshot") {
    errorOut("Unknown option: --format");
  }
  if (Object.prototype.hasOwnProperty.call(options, "policy")) {
    errorOut("Custom policy path is not supported. Use XDG_CONFIG_HOME/tabctl/policy.json.");
  }

  if (command === "refresh") {
    const tabValues = Array.isArray(options.tab)
      ? (options.tab as string[]).map((value) => String(value).trim()).filter(Boolean)
      : [];
    if (tabValues.length === 0) {
      errorOut("refresh requires --tab");
    }
    if (tabValues.length > 1) {
      errorOut("refresh requires a single --tab");
    }
  }

  if (command === "open" && options.color && !options.group) {
    errorOut("--color requires --group");
  }

  if (command === "undo") {
    if (options.txid && options._.length > 0) {
      errorOut("undo requires a single txid (use positional arg or --txid)");
    }
    if (options.latest === true && options._.length > 0) {
      errorOut("undo --latest cannot be combined with a txid");
    }
    if (options.latest === true && options.txid) {
      errorOut("undo --latest cannot be combined with --txid");
    }
    if (options.txid && options._.length === 0) {
      options._ = [String(options.txid)];
    }
  }

  if (command === "inspect") {
    const selectorCount = Array.isArray(options.selector) ? options.selector.length : 0;
    if (selectorCount > 0) {
      const signalList = normalizeSignals(options.signal);
      if (!signalList.includes("selector")) {
        signalList.push("selector");
      }
      options.signal = signalList;
    }
    const signalList = normalizeSignals(options.signal);
    if (signalList.length > 0) {
      validateSignals(signalList);
      options.signal = signalList;
    }
  }

  if (command === "screenshot") {
    const mode = options.mode != null ? String(options.mode).trim().toLowerCase() : "viewport";
    const format = options.format != null ? String(options.format).trim().toLowerCase() : "png";
    if (mode !== "viewport" && mode !== "full") {
      errorOut("Invalid --mode value (use viewport or full)");
    }
    if (format !== "png" && format !== "jpeg") {
      errorOut("Invalid --format value (use png or jpeg)");
    }
    if (format === "jpeg" && options.quality == null) {
      options.quality = 80;
    }
    const qualityRaw = options.quality != null ? Number(options.quality) : null;
    if (qualityRaw != null && (!Number.isFinite(qualityRaw) || qualityRaw < 0 || qualityRaw > 100)) {
      errorOut("Invalid --quality value (use 0-100)");
    }
    if (qualityRaw != null && format !== "jpeg") {
      errorOut("--quality requires --format jpeg");
    }
    const tileMaxDimRaw = options["tile-max-dim"] != null ? Number(options["tile-max-dim"]) : null;
    if (tileMaxDimRaw != null && (!Number.isFinite(tileMaxDimRaw) || tileMaxDimRaw <= 0)) {
      errorOut("Invalid --tile-max-dim value");
    }
    const maxBytesRaw = options["max-bytes"] != null ? Number(options["max-bytes"]) : null;
    if (maxBytesRaw != null && (!Number.isFinite(maxBytesRaw) || maxBytesRaw <= 0)) {
      errorOut("Invalid --max-bytes value");
    }
    if (mode === "viewport" && options["tile-max-dim"] != null) {
      errorOut("--tile-max-dim requires --mode full");
    }
    if (mode === "viewport" && options["max-bytes"] != null) {
      errorOut("--max-bytes requires --mode full");
    }
    if (options.mode == null) {
      options.mode = "viewport";
    }
    if (options.format == null) {
      options.format = "png";
    }
  }

  const policyContext = loadPolicy();
  const policySummary = summarizePolicy(policyContext.policy, policyContext.path);
  const policyEnabled = policyContext.policy !== null;
  const enforcePolicy = policyEnabled;
  const includeWindowTitle = options["window-title"] === true;
  const includeStale = options["include-stale"] === true;
  let policySnapshot: Record<string, unknown> | null = null;
  const getPolicySnapshot = async () => {
    if (!policySnapshot) {
      policySnapshot = await fetchSnapshot();
    }
    return policySnapshot;
  };

  if (!command || command === "help" || options.help) {
    const helpTarget = command === "help"
      ? (options._.length > 0 ? String(options._[0]) : undefined)
      : command;
    printHelp(options.json === true, helpTarget);
    return;
  }

  if (warnings.length > 0) {
    for (const warning of warnings) {
      process.stderr.write(`[tabctl] warning: ${warning}\n`);
    }
  }

  if (command === "skill") {
    runSkillInstall(options, prettyOutput);
    return;
  }

  if (command === "setup") {
    runSetup(options, prettyOutput);
    return;
  }

  if (command === "version") {
    runVersion(prettyOutput);
    return;
  }

  if (command === "policy") {
    runPolicy(options, policyContext, prettyOutput);
    return;
  }

  if (command === "ping") {
    await runPing(prettyOutput);
    return;
  }

  if (command === "history") {
    await runHistory(options, prettyOutput);
    return;
  }

  if (command === "undo") {
    await runUndo(options, prettyOutput);
    return;
  }

  if (command === "list") {
    await runList(options, policyContext, policySummary, prettyOutput);
    return;
  }

  if (command === "group-list") {
    await runGroupList(options, policyContext, policySummary, prettyOutput);
    return;
  }


  let dedupeMode = false;
  if (command === "close" && options["dry-run"]) {
    command = "analyze";
  }
  if (command === "dedupe") {
    dedupeMode = true;
    command = "analyze";
  }

  let action = command;
  let params: Record<string, unknown> = {};
  let policyInfo: Record<string, unknown> | null = null;
  let earlyResponse: Record<string, unknown> | null = null;

  switch (command) {
    case "analyze":
      action = "analyze";
      params = buildAnalyzeParams(options);
      break;
    case "inspect":
      action = "inspect";
      params = buildInspectParams(options);
      break;
    case "focus":
      action = "focus";
      params = buildFocusParams(options);
      break;
    case "refresh":
      action = "refresh";
      params = buildRefreshParams(options);
      break;
    case "open":
      action = "open";
      params = buildOpenParams(options);
      break;
    case "group-update":
      action = "group-update";
      params = buildGroupUpdateParams(options);
      break;
    case "group-ungroup":
      action = "group-ungroup";
      params = buildGroupUngroupParams(options);
      break;
    case "group-assign":
      action = "group-assign";
      params = buildGroupAssignParams(options);
      break;
    case "move-tab":
      action = "move-tab";
      params = buildMoveTabParams(options);
      break;
    case "move-group":
      action = "move-group";
      params = buildMoveGroupParams(options);
      break;
    case "merge-window":
      action = "merge-window";
      params = buildMergeWindowParams(options);
      break;
    case "archive":
      action = "archive";
      params = buildArchiveParams(options);
      break;
    case "close":
      action = "close";
      params = buildCloseParams(options);
      break;
    case "report":
      action = "report";
      params = buildReportParams(options);
      break;
    case "screenshot":
      action = "screenshot";
      params = buildScreenshotParams(options);
      break;
    default:
      errorOut(`Unknown command: ${command}`);
  }

  if (command === "analyze") {
    const tabIds = (params as { tabIds?: number[] }).tabIds;
    const windowId = (params as { windowId?: number | string }).windowId;
    const hasScope = (Array.isArray(tabIds) && tabIds.length > 0)
      || Boolean((params as { groupTitle?: string }).groupTitle)
      || Number.isFinite((params as { groupId?: number }).groupId)
      || (typeof windowId === "number" && Number.isFinite(windowId))
      || (typeof windowId === "string" && windowId.length > 0)
      || (params as { all?: boolean }).all === true;
    if (!hasScope) {
      params = { ...params, all: true };
    }
  }

  if (command === "merge-window") {
    const fromWindowId = (params as { fromWindowId?: number }).fromWindowId;
    const toWindowId = (params as { toWindowId?: number }).toWindowId;
    if (!Number.isFinite(fromWindowId) || !Number.isFinite(toWindowId)) {
      errorOut("merge-window requires --from and --to window ids");
    }
    if (fromWindowId === toWindowId) {
      errorOut("merge-window --from and --to cannot be the same window");
    }
    if ((params as { closeSource?: boolean }).closeSource && !(params as { confirmed?: boolean }).confirmed) {
      errorOut("merge-window --close-source requires --confirm");
    }
  }

  if (enforcePolicy && ["analyze", "inspect", "report", "screenshot", "close", "archive", "focus", "refresh", "move-tab", "move-group", "group-assign", "group-update", "group-ungroup", "merge-window"].includes(command)) {
    if (command === "close" && options.apply) {
      errorOut("Policy blocks close --apply; use explicit tab targets.");
    }

    const snapshot = await getPolicySnapshot();
    if (!snapshot) {
      errorOut("Failed to load tabs for policy evaluation");
    }

    const selection = selectTabsFromSnapshot(snapshot, params);
    if ((selection as { error?: Record<string, unknown> }).error) {
      printJson({ ok: false, error: (selection as { error: Record<string, unknown> }).error }, prettyOutput);
      process.exit(1);
    }

    const selectedTabs = (selection as { tabs: Array<Record<string, unknown>> }).tabs;
    const eligibleTabs = selectedTabs.filter((tab) => evaluateTab(tab, policyContext.policy).eligible);
    const protectedTabs = selectedTabs.filter((tab) => !evaluateTab(tab, policyContext.policy).eligible);
    const eligibleIds = eligibleTabs.map((tab) => tab.tabId).filter((id) => typeof id === "number") as number[];

    if (command === "focus" || command === "refresh") {
      if (!eligibleIds.length) {
        errorOut(`Tab is protected by policy and cannot be ${command === "focus" ? "focused" : "refreshed"} via CLI`);
      }
      params = {
        tabId: eligibleIds[0],
      };
    } else if (command === "close" || command === "archive") {
      if (!eligibleIds.length) {
        earlyResponse = {
          ok: true,
          action: command,
          data: {
            summary: { eligible: 0, protected: protectedTabs.length },
            protected: protectedTabs.map((tab) => ({
              tabId: tab.tabId,
              windowId: tab.windowId,
              groupId: tab.groupId,
              groupTitle: tab.groupTitle,
              title: tab.title,
              url: tab.url,
              pinned: tab.pinned,
            })),
            policy: policySummary,
          },
        };
      } else if (command === "close") {
        params = {
          mode: "direct",
          confirmed: true,
          tabIds: eligibleIds,
        };
        } else if (command === "archive") {
          params = {
            tabIds: eligibleIds,
          };
        }

      policyInfo = {
        protected: protectedTabs.map((tab) => ({
          tabId: tab.tabId,
          windowId: tab.windowId,
          groupId: tab.groupId,
          groupTitle: tab.groupTitle,
          title: tab.title,
          url: tab.url,
          pinned: tab.pinned,
        })),
      };
    } else if (command === "move-tab" || command === "move-group" || command === "group-assign") {
      if (!eligibleIds.length || (command === "move-group" && protectedTabs.length > 0)) {
        earlyResponse = {
          ok: true,
          action: command,
          data: {
            summary: { eligible: eligibleIds.length, protected: protectedTabs.length },
            protected: protectedTabs.map((tab) => ({
              tabId: tab.tabId,
              windowId: tab.windowId,
              groupId: tab.groupId,
              groupTitle: tab.groupTitle,
              title: tab.title,
              url: tab.url,
              pinned: tab.pinned,
            })),
            policy: policySummary,
          },
        };
      } else if (command === "move-tab" || command === "group-assign") {
        params = {
          ...params,
          tabId: eligibleIds[0],
          tabIds: eligibleIds,
        };
      }

      policyInfo = {
        protected: protectedTabs.map((tab) => ({
          tabId: tab.tabId,
          windowId: tab.windowId,
          groupId: tab.groupId,
          groupTitle: tab.groupTitle,
          title: tab.title,
          url: tab.url,
          pinned: tab.pinned,
        })),
      };
      } else if (command === "merge-window") {
        if (!eligibleIds.length) {
          earlyResponse = {
            ok: true,
            action: command,
            data: {
              summary: { eligible: 0, protected: protectedTabs.length },
              protected: protectedTabs.map((tab) => ({
                tabId: tab.tabId,
                windowId: tab.windowId,
                groupId: tab.groupId,
                groupTitle: tab.groupTitle,
                title: tab.title,
                url: tab.url,
                pinned: tab.pinned,
              })),
              policy: policySummary,
            },
          };
        }

        params = {
          ...params,
          tabIds: eligibleIds,
        };

        policyInfo = {
          protected: protectedTabs.map((tab) => ({
            tabId: tab.tabId,
            windowId: tab.windowId,
            groupId: tab.groupId,
            groupTitle: tab.groupTitle,
            title: tab.title,
            url: tab.url,
            pinned: tab.pinned,
          })),
        };
      } else if (command === "group-update" || command === "group-ungroup") {
      if (!eligibleIds.length || protectedTabs.length > 0) {
        earlyResponse = {
          ok: true,
          action: command,
          data: {
            summary: { eligible: eligibleIds.length, protected: protectedTabs.length },
            protected: protectedTabs.map((tab) => ({
              tabId: tab.tabId,
              windowId: tab.windowId,
              groupId: tab.groupId,
              groupTitle: tab.groupTitle,
              title: tab.title,
              url: tab.url,
              pinned: tab.pinned,
            })),
            policy: policySummary,
          },
        };
      }

      policyInfo = {
        protected: protectedTabs.map((tab) => ({
          tabId: tab.tabId,
          windowId: tab.windowId,
          groupId: tab.groupId,
          groupTitle: tab.groupTitle,
          title: tab.title,
          url: tab.url,
          pinned: tab.pinned,
        })),
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
        params = {
          ...params,
          tabIds: eligibleIds,
        };
      }
    }
  }

  const request = {
    id: createId(),
    action,
    params,
    client: {
      component: "cli",
      version: VERSION,
      baseVersion: BASE_VERSION,
      gitSha: GIT_SHA,
      dirty: DIRTY,
    },
  };

  let response: Record<string, unknown>;
  const showProgress = options.progress === true;
  const startedAt = Date.now();
  let progressTimer: NodeJS.Timeout | null = null;
  if (showProgress) {
    progressTimer = setInterval(() => {
      const elapsed = Math.round((Date.now() - startedAt) / 1000);
      process.stderr.write(`[tabctl] waiting ${elapsed}s...\n`);
    }, 2000);
  }

  const onProgress = showProgress
    ? (message: Record<string, unknown>) => {
      const data = message.data as Record<string, unknown> | undefined;
      if (data?.phase === "github") {
        const processed = data.processed as number;
        const total = data.total as number;
        const matched = data.matched as number;
        process.stderr.write(`[tabctl] github ${processed}/${total} (matched ${matched})\n`);
      }
      if (data?.phase === "inspect") {
        const processed = data.processed as number;
        const total = data.total as number;
        const signalId = data.signalId as string;
        process.stderr.write(`[tabctl] inspect ${processed}/${total} (${signalId})\n`);
      }
      if (data?.phase === "screenshot") {
        const processed = data.processed as number;
        const total = data.total as number;
        process.stderr.write(`[tabctl] screenshot ${processed}/${total}\n`);
      }
    }
    : undefined;

  if (earlyResponse) {
    response = earlyResponse;
  } else {
    try {
      response = await sendRequest(request, onProgress);
    } catch (error) {
      if (progressTimer) {
        clearInterval(progressTimer);
      }
      const message = error instanceof Error ? error.message : "Unknown error";
      errorOut(`Failed to connect to host: ${message}`);
      return;
    } finally {
      if (progressTimer) {
        clearInterval(progressTimer);
      }
    }
  }

  if (progressTimer) {
    clearInterval(progressTimer);
  }

  if (!response) {
    errorOut("No response received");
  }

  if (!response.ok) {
    printJson(response, prettyOutput);
    process.exit(1);
  }

  if (response.data && typeof response.data === "object") {
    const data = response.data as Record<string, unknown>;
    if ((command === "inspect" || command === "report" || command === "screenshot") && Array.isArray(data.entries)) {
      let snapshot: Record<string, unknown> | null = null;
      if (policyEnabled) {
        snapshot = await getPolicySnapshot();
      }
      const tabIndex = snapshot ? buildTabIndex(snapshot) : null;
      const annotated = (data.entries as Array<Record<string, unknown>>).map((entry) => {
        const tab = tabIndex?.get(entry.tabId as number) || entry;
        const { eligible, protectedReasons } = evaluateTab(tab, policyContext.policy);
        return {
          ...entry,
          eligible,
          protectedReasons,
        };
      }).filter((entry) => entry.eligible !== false);
      const scope = resolveScopeFlags(options);
      const allScope = options.all === true || !scope.hasScope;
      const scopeArgs = buildScopeArgs(options, allScope);
      const pagination = resolvePagination(options, annotated.length, command, scopeArgs);
      const start = pagination.offset;
      const end = pagination.offset + pagination.limit;
      data.entries = annotated.slice(start, end);
      if (pagination.page) {
        data.page = pagination.page;
      }
    }

    if (command === "analyze" && Array.isArray(data.candidates)) {
      let snapshot: Record<string, unknown> | null = null;
      if (policyEnabled || includeWindowTitle) {
        snapshot = await getPolicySnapshot();
      }
      const tabIndex = snapshot ? buildTabIndex(snapshot) : null;
      const windowTitleIndex = snapshot && includeWindowTitle
        ? buildWindowTitleIndex(snapshot, policyContext.policy)
        : null;
      data.candidates = (data.candidates as Array<Record<string, unknown>>).map((candidate) => {
        const tab = tabIndex?.get(candidate.tabId as number) || candidate;
        const { eligible, protectedReasons } = evaluateTab(tab, policyContext.policy);
        const windowTitle = includeWindowTitle
          ? (windowTitleIndex?.get(candidate.windowId as number) ?? null)
          : undefined;
        return {
          ...candidate,
          eligible,
          protectedReasons,
          ...(includeWindowTitle ? { windowTitle } : {}),
        };
      }).filter((candidate) => candidate.eligible !== false);
    }

    data.policy = policySummary;
    if (policyInfo) {
      data.policyInfo = policyInfo;
    }
  } else if (response.ok) {
    response.policy = policySummary;
  }

  if (dedupeMode) {
    if (!response.ok) {
      printJson(response, prettyOutput);
      return;
    }

    const data = (response.data as Record<string, unknown>) || {};
    const candidates = Array.isArray(data.candidates) ? (data.candidates as Array<Record<string, unknown>>) : [];
    const planned = candidates.filter((candidate) => {
      const reasons = Array.isArray(candidate.reasons) ? (candidate.reasons as Array<Record<string, unknown>>) : [];
      const hasDuplicate = reasons.some((reason) => reason.type === "duplicate" || reason.type === "closed_issue");
      const hasStale = reasons.some((reason) => reason.type === "stale");
      return hasDuplicate || (includeStale && hasStale);
    });

    const planTabIds: number[] = [];
    const expectedUrls: Record<string, string> = {};
    for (const candidate of planned) {
      const tabId = candidate.tabId as number;
      if (!Number.isFinite(tabId)) {
        continue;
      }
      if (!planTabIds.includes(tabId)) {
        planTabIds.push(tabId);
      }
      if (typeof candidate.url === "string") {
        expectedUrls[String(tabId)] = candidate.url;
      }
    }

    let closeData: Record<string, unknown> | null = null;
    if (options.confirm === true && planTabIds.length > 0) {
      const closeResponse = await sendRequest({
        id: createId(),
        action: "close",
        params: {
          mode: "direct",
          confirmed: true,
          tabIds: planTabIds,
          expectedUrls,
        },
      });
      if (!closeResponse.ok) {
        printJson(closeResponse, prettyOutput);
        return;
      }
      closeData = (closeResponse.data as Record<string, unknown>) || {};
      closeData.policy = policySummary;
      if (policyInfo) {
        closeData.policyInfo = policyInfo;
      }
    }

    const closeSummary = closeData?.summary as Record<string, unknown> | undefined;
    const closedTabs = Number(closeSummary?.closedTabs ?? 0);
    const skippedTabs = Number(closeSummary?.skippedTabs ?? 0);
    const output = {
      ok: true,
      action: "dedupe",
      data: {
        analysisId: data.analysisId || null,
        summary: {
          candidates: candidates.length,
          planned: planTabIds.length,
          closed: Number.isFinite(closedTabs) ? closedTabs : 0,
          skipped: Number.isFinite(skippedTabs) ? skippedTabs : 0,
        },
        plan: {
          tabIds: planTabIds,
          candidates: planned,
        },
        close: closeData,
        nextCommand: options.confirm === true
          ? null
          : (planTabIds.length > 0 && data.analysisId ? `tabctl close --apply ${data.analysisId} --confirm` : null),
        policy: data.policy,
        policyInfo: data.policyInfo,
      },
    };
    printJson(output, prettyOutput);
    return;
  }

  if (command === "report") {
    const format = (options.format as string) || "json";
    const data = response.data as { entries?: Array<Record<string, unknown>>; generatedAt?: number } | undefined;
    const entries = data?.entries || [];
    const generatedAt = data?.generatedAt;
    const page = data && "page" in data ? (data.page as Record<string, unknown> | undefined) : undefined;
    let content = "";

    if (format === "json") {
      content = JSON.stringify({ generatedAt, entries }, null, 2);
    } else if (format === "csv") {
      content = renderCsv(entries);
    } else if (format === "md") {
      content = renderMarkdown(entries, generatedAt);
    } else {
      errorOut(`Unknown report format: ${format}`);
    }

    if (options.out) {
      fs.writeFileSync(String(options.out), content, "utf8");
      printJson({ ok: true, data: { writtenTo: options.out, format, count: entries.length, ...(page ? { page } : {}) } }, prettyOutput);
      return;
    }

    if (format === "json") {
      printJson({ ok: true, data: { format, entries, ...(page ? { page } : {}) } }, prettyOutput);
      return;
    }

    printJson({ ok: true, data: { format, entries, content, ...(page ? { page } : {}) } }, prettyOutput);
    return;
  }

   if (command === "screenshot") {
    const data = response.data as { entries?: Array<Record<string, unknown>> } | undefined;
    const entries = data?.entries || [];
    const page = data && "page" in data ? (data.page as Record<string, unknown> | undefined) : undefined;
    const outDir = options.out
      ? String(options.out)
      : path.join(process.cwd(), ".tabctl", "screenshots", String(Date.now()));
    fs.mkdirSync(outDir, { recursive: true });
    let filesWritten = 0;
    const sanitized = entries.map((entry) => {
        const tabId = entry.tabId as number | string | undefined;
        const tabDir = path.join(outDir, String(tabId ?? "unknown"));
        fs.mkdirSync(tabDir, { recursive: true });
        const tiles = Array.isArray(entry.tiles) ? (entry.tiles as Array<Record<string, unknown>>) : [];
        const sanitizedTiles = tiles.map((tile) => {
          const rawUrl = tile.dataUrl as string | undefined;
          const { dataUrl: _ignored, ...rest } = tile as Record<string, unknown>;
          if (!rawUrl) {
            return { ...rest, path: null, error: "missing_data" };
          }
          const match = rawUrl.match(/^data:(image\/png|image\/jpeg);base64,(.+)$/);
          if (!match) {
            return { ...rest, path: null, error: "invalid_data_url" };
          }
          const mime = match[1];
          const base64 = match[2];
          const ext = mime === "image/jpeg" ? "jpg" : "png";
          const index = Number.isFinite(tile.index as number) ? Number(tile.index) + 1 : filesWritten + 1;
          const total = Number.isFinite(tile.total as number) ? Number(tile.total) : null;
          const suffix = total && total > 1 ? `-of-${total}` : "";
          const filename = `screenshot-${index}${suffix}.${ext}`;
          const filePath = path.join(tabDir, filename);
          const buffer = Buffer.from(base64, "base64");
          fs.writeFileSync(filePath, buffer);
          filesWritten += 1;
          return {
            ...rest,
            path: filePath,
            bytes: buffer.length,
          };
        });
        return {
          ...entry,
          tiles: sanitizedTiles,
        };
      });
    printJson({ ok: true, data: { writtenTo: outDir, files: filesWritten, entries: sanitized, ...(page ? { page } : {}) } }, prettyOutput);
    return;
  }

  emitVersionWarnings(response, command);

  printJson(response, prettyOutput);
}

main().catch((error: Error) => {
  errorOut(error.message || "Unknown error");
});
