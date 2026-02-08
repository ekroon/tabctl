#!/usr/bin/env node
import { loadPolicy, summarizePolicy } from "./lib/policy";
import { VERSION, BASE_VERSION, GIT_SHA, DIRTY } from "./lib/constants";
import { printJson, errorOut, setupStdoutErrorHandling, emitVersionWarnings } from "./lib/output";
import { parseArgs, normalizeSignals, validateSignals } from "./lib/args";
import { sendRequest, createRequestId, fetchSnapshot } from "./lib/client";
import { printHelp } from "./lib/help";
import { annotateEntries, annotateCandidates, extractDedupePlan, buildDedupeOutput, formatReport, writeScreenshots } from "./lib/response";
import { applyPolicyFilter } from "./lib/policy-filter";
import { runSetup, runSkillInstall, runVersion, runPolicy, runList, runGroupList, runPing, runHistory, runUndo, runProfileList, runProfileShow, runProfileSwitch, runProfileRemove } from "./lib/commands";
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
  if (typeof options.profile === "string" && options.profile) {
    process.env.TABCTL_PROFILE = options.profile;
  }
  if (command === "dedupe" && (options as Record<string, unknown>).close) {
    errorOut("dedupe does not support --close; use --confirm or close --apply <analysisId>.");
  }
  const prettyOutput = options.pretty !== false;
  if (command === "groups" || command === "group") {
    command = "group-list";
  }
  if (command === "profile" || command === "profiles") {
    command = "profile-list";
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
    await runSetup(options, prettyOutput);
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

  if (command === "profile-list") {
    runProfileList(options, prettyOutput);
    return;
  }

  if (command === "profile-show") {
    runProfileShow(options, prettyOutput);
    return;
  }

  if (command === "profile-switch") {
    runProfileSwitch(options, prettyOutput);
    return;
  }

  if (command === "profile-remove") {
    runProfileRemove(options, prettyOutput);
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
    case "reload":
      action = "reload";
      params = {};
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

    const filterResult = applyPolicyFilter(command, params, snapshot, policyContext, policySummary);

    if (filterResult.earlyResponse && !filterResult.earlyResponse.ok) {
      printJson(filterResult.earlyResponse, prettyOutput);
      process.exit(1);
    }

    params = filterResult.params;
    policyInfo = filterResult.policyInfo;
    earlyResponse = filterResult.earlyResponse;
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
      annotateEntries(data, options, command, policyEnabled, policyContext.policy, snapshot);
    }

    if (command === "analyze" && Array.isArray(data.candidates)) {
      let snapshot: Record<string, unknown> | null = null;
      if (policyEnabled || includeWindowTitle) {
        snapshot = await getPolicySnapshot();
      }
      annotateCandidates(data, policyContext.policy, includeWindowTitle, snapshot);
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

    const { planTabIds, expectedUrls } = extractDedupePlan(response, includeStale);

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

    const output = buildDedupeOutput(response, includeStale, closeData, options.confirm === true);
    printJson(output, prettyOutput);
    return;
  }

  if (command === "report") {
    formatReport(response, options, prettyOutput);
    return;
  }

   if (command === "screenshot") {
    writeScreenshots(response, options, prettyOutput);
    return;
  }

  emitVersionWarnings(response, command);

  printJson(response, prettyOutput);
}

main().catch((error: Error) => {
  errorOut(error.message || "Unknown error");
});
