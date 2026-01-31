#!/usr/bin/env node
import fs from "fs";
import net from "net";
import os from "os";
import path from "path";
import { renderCsv, renderMarkdown } from "./lib/report";
import { annotateEntry, defaultPolicyPath, defaultPolicyTemplate, evaluateTab, loadPolicy, summarizePolicy, type Policy } from "./lib/policy";

const SOCKET_PATH = process.env.TABARCHIVE_SOCKET || path.join(os.homedir(), ".tabarchive", "tabarchive.sock");
const HOST_NAME = "com.erwinkroon.tabctl";
const HOST_DESCRIPTION = "Tab archive native host";
const EXTENSION_ID_PATTERN = /^[a-p]{32}$/;

type Options = {
  _: string[];
  [key: string]: unknown;
};

function createId() {
  return `req-${Date.now()}-${Math.random().toString(16).slice(2, 10)}`;
}

function parseArgs(argv: string[]) {
  const args = [...argv];
  let command: string | undefined;
  const options: Options = { _: [] };

  while (args.length > 0) {
    const arg = args.shift() as string;
    if (!arg.startsWith("--")) {
      if (!command) {
        command = arg;
        continue;
      }
      options._.push(arg);
      continue;
    }

    const key = arg.slice(2);
    if (["all", "pretty", "confirm", "dry-run", "github", "progress", "init", "help", "json", "window-title", "create", "collapsed", "expanded"].includes(key)) {
      options[key] = true;
      continue;
    }


    const value = args.shift();
    if (key === "signal") {
      if (!options.signal) {
        options.signal = [];
      }
      (options.signal as string[]).push(value as string);
      continue;
    }
    if (key === "tab") {
      if (!options.tab) {
        options.tab = [];
      }
      (options.tab as string[]).push(value as string);
      continue;
    }
    if (key === "url") {
      if (!options.url) {
        options.url = [];
      }
      (options.url as string[]).push(value as string);
      continue;
    }
    if (key === "selector") {
      if (!options.selector) {
        options.selector = [];
      }
      (options.selector as string[]).push(value as string);
      continue;
    }
    options[key] = value;
  }

  return { command, options };
}

function sendRequest(payload: Record<string, unknown>, onProgress?: (data: Record<string, unknown>) => void) {
  return new Promise<Record<string, unknown>>((resolve, reject) => {
    const client = net.createConnection(SOCKET_PATH);
    let buffer = "";

    client.on("connect", () => {
      client.write(`${JSON.stringify(payload)}\n`);
    });

    client.on("data", (data) => {
      buffer += data;
      let index;
      while ((index = buffer.indexOf("\n")) >= 0) {
        const line = buffer.slice(0, index).trim();
        buffer = buffer.slice(index + 1);
        if (!line) {
          continue;
        }
        let response: Record<string, unknown>;
        try {
          response = JSON.parse(line);
        } catch (error) {
          client.end();
          client.destroy();
          reject(error);
          return;
        }

        if (response.progress && onProgress) {
          onProgress(response);
          continue;
        }

        client.end();
        client.destroy();
        resolve(response);
        return;
      }
    });

    client.on("error", (error) => {
      reject(error);
    });
  });
}

async function fetchSnapshot() {
  const response = await sendRequest({ id: createId(), action: "list", params: {} });
  if (!response.ok) {
    return null;
  }
  return response.data as Record<string, unknown> | null;
}

function buildTabIndex(snapshot: Record<string, unknown>) {
  const tabIndex = new Map<number, Record<string, unknown>>();
  const windows = (snapshot.windows as Array<Record<string, unknown>>) || [];
  for (const window of windows) {
    const tabs = (window.tabs as Array<Record<string, unknown>>) || [];
    for (const tab of tabs) {
      if (typeof tab.tabId === "number") {
        tabIndex.set(tab.tabId, tab);
      }
    }
  }
  return tabIndex;
}

function buildWindowTitleIndex(snapshot: Record<string, unknown>, policy: Policy | null) {
  const windowTitleIndex = new Map<number, string | null>();
  const windows = (snapshot.windows as Array<Record<string, unknown>>) || [];
  for (const window of windows) {
    const windowId = window.windowId as number;
    if (typeof windowId !== "number") {
      continue;
    }
    const tabs = (window.tabs as Array<Record<string, unknown>>) || [];
    const activeTab = tabs.find((tab) => tab.active === true);
    if (!activeTab) {
      windowTitleIndex.set(windowId, null);
      continue;
    }
    const { eligible } = evaluateTab(activeTab, policy);
    if (!eligible) {
      windowTitleIndex.set(windowId, null);
      continue;
    }
    const title = typeof activeTab.title === "string" ? activeTab.title : null;
    windowTitleIndex.set(windowId, title);
  }
  return windowTitleIndex;
}

function selectTabsFromSnapshot(snapshot: Record<string, unknown>, params: Record<string, unknown>) {
  const windows = (snapshot.windows as Array<Record<string, unknown>>) || [];
  const allTabs = windows.flatMap((win) => (win.tabs as Array<Record<string, unknown>>) || []);

  if (params.tabIds && (params.tabIds as Array<number>).length) {
    const idSet = new Set((params.tabIds as Array<number>).map(Number));
    return { tabs: allTabs.filter((tab) => idSet.has(tab.tabId as number)) };
  }

  if (params.groupId != null) {
    const groupId = Number(params.groupId);
    return { tabs: allTabs.filter((tab) => tab.groupId === groupId) };
  }

  if (params.groupTitle) {
    const matches: Array<{ windowId: number; groupId: number }> = [];
    for (const win of windows) {
      const groups = (win.groups as Array<Record<string, unknown>>) || [];
      for (const group of groups) {
        if (group.title === params.groupTitle) {
          matches.push({ windowId: win.windowId as number, groupId: group.groupId as number });
        }
      }
    }

    if (matches.length === 0) {
      return { tabs: [], error: { message: "No matching group title found" } };
    }

    if (matches.length > 1 && !params.windowId) {
      return { tabs: [], error: { message: "Group title is ambiguous. Provide a windowId." } };
    }

    const target = params.windowId
      ? matches.find((match) => match.windowId === Number(params.windowId))
      : matches[0];

    if (!target) {
      return { tabs: [], error: { message: "Group title not found in specified window" } };
    }

    return { tabs: allTabs.filter((tab) => tab.groupId === target.groupId) };
  }

  if (params.windowId != null) {
    const windowId = Number(params.windowId);
    return { tabs: allTabs.filter((tab) => tab.windowId === windowId) };
  }

  if (params.all) {
    return { tabs: allTabs };
  }

  const focused = windows.find((win) => win.focused);
  return { tabs: focused ? ((focused.tabs as Array<Record<string, unknown>>) || []) : [] };
}

function filterSnapshotByPolicy(snapshot: Record<string, unknown>, policy: Record<string, unknown> | null) {
  if (!policy) {
    return snapshot;
  }

  const windows = (snapshot.windows as Array<Record<string, unknown>>) || [];
  const filteredWindows = windows.map((win) => {
    const tabs = (win.tabs as Array<Record<string, unknown>>) || [];
    const eligibleTabs = tabs
      .filter((tab) => evaluateTab(tab, policy).eligible)
      .map((tab) => annotateEntry(tab, policy));

    const eligibleGroupIds = new Set(
      eligibleTabs
        .map((tab) => tab.groupId)
        .filter((groupId) => typeof groupId === "number" && groupId !== -1) as number[],
    );
    const groups = (win.groups as Array<Record<string, unknown>>) || [];
    const filteredGroups = groups.filter((group) => eligibleGroupIds.has(group.groupId as number));

    return {
      ...win,
      tabs: eligibleTabs,
      groups: filteredGroups,
    };
  }).filter((win) => (win.tabs as Array<Record<string, unknown>>).length > 0);

  return {
    ...snapshot,
    windows: filteredWindows,
  };
}

function printJson(payload: Record<string, unknown>, pretty = true) {
  const output = pretty ? JSON.stringify(payload, null, 2) : JSON.stringify(payload);
  process.stdout.write(`${output}\n`);
}

function resolveBrowser(value: unknown): "edge" | "chrome" | null {
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim().toLowerCase();
  if (trimmed === "edge" || trimmed === "chrome") {
    return trimmed;
  }
  return null;
}

function resolveExtensionId(options: Options) {
  const raw = typeof options["extension-id"] === "string"
    ? String(options["extension-id"])
    : (process.env.TABARCHIVE_EXTENSION_ID || "");
  const value = raw.trim().toLowerCase();
  if (!value) {
    errorOut("Missing --extension-id (or TABARCHIVE_EXTENSION_ID)");
  }
  if (!EXTENSION_ID_PATTERN.test(value)) {
    errorOut(`Extension ID looks unusual: ${raw}`);
  }
  return value;
}

function resolveNodePath(options: Options) {
  const raw = typeof options.node === "string"
    ? String(options.node)
    : (process.env.TABARCHIVE_NODE || process.execPath || "");
  const value = raw.trim();
  if (!value) {
    errorOut("Node binary not found. Set --node or TABARCHIVE_NODE.");
  }
  if (!path.isAbsolute(value)) {
    errorOut(`Node path must be absolute: ${value}`);
  }
  try {
    fs.accessSync(value, fs.constants.X_OK);
  } catch (error) {
    errorOut(`Node binary not executable: ${value}`);
  }
  return value;
}

function resolveHostPath() {
  const root = path.resolve(__dirname, "..");
  const hostPath = path.join(root, "host", "host.js");
  if (!fs.existsSync(hostPath)) {
    errorOut(`Host script not found at ${hostPath}. Run: npm run build`);
  }
  return hostPath;
}

function resolveManifestDir(browser: "edge" | "chrome") {
  const home = os.homedir();
  if (!home) {
    errorOut("Home directory not found.");
  }
  if (browser === "edge") {
    return path.join(home, "Library", "Application Support", "Microsoft Edge", "NativeMessagingHosts");
  }
  return path.join(home, "Library", "Application Support", "Google", "Chrome", "NativeMessagingHosts");
}

function writeWrapper(nodePath: string, hostPath: string) {
  const wrapperDir = path.join(os.homedir(), ".tabarchive");
  fs.mkdirSync(wrapperDir, { recursive: true, mode: 0o700 });
  const wrapperPath = path.join(wrapperDir, "tabarchive-host.sh");
  const escapedNode = nodePath.replace(/"/g, "\\\"");
  const escapedHost = hostPath.replace(/"/g, "\\\"");
  const wrapper = [
    "#!/usr/bin/env bash",
    "set -euo pipefail",
    `exec \"${escapedNode}\" \"${escapedHost}\"`,
    "",
  ].join("\n");
  fs.writeFileSync(wrapperPath, wrapper, "utf8");
  fs.chmodSync(wrapperPath, 0o700);
  return wrapperPath;
}

function runSetup(options: Options, prettyOutput: boolean) {
  if (process.platform !== "darwin") {
    errorOut("tabctl setup is only supported on macOS.");
  }

  const browser = resolveBrowser(options.browser);
  if (!browser) {
    errorOut("Missing or invalid --browser (edge|chrome)");
  }

  const extensionId = resolveExtensionId(options);
  const nodePath = resolveNodePath(options);
  const hostPath = resolveHostPath();
  const wrapperPath = writeWrapper(nodePath, hostPath);
  const manifestDir = resolveManifestDir(browser);
  fs.mkdirSync(manifestDir, { recursive: true });

  const manifestPath = path.join(manifestDir, `${HOST_NAME}.json`);
  const manifest = {
    name: HOST_NAME,
    description: HOST_DESCRIPTION,
    path: wrapperPath,
    type: "stdio",
    allowed_origins: [`chrome-extension://${extensionId}/`],
  };
  fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2), "utf8");

  printJson({
    ok: true,
    data: {
      browser,
      extensionId,
      manifestPath,
      wrapperPath,
      hostPath,
      nodePath,
    },
  }, prettyOutput);
}

function buildHelpData() {
  return {
    commands: [
      "help",
      "list",
      "analyze",
      "inspect",
      "focus",
      "open",
      "group-assign",
      "move-tab",
      "move-group",
      "setup",
      "policy",
      "archive",
      "close",
      "report",
      "undo",
      "history",
      "ping",
    ],
    usage: "tabctl <command> [options]",
    options: {
      analyze: [
        "--stale-days <n>",
        "--github",
        "--github-concurrency <n>",
        "--github-timeout-ms <ms>",
        "--tab <id> (repeatable)",
        "--window-title (include active window title)",
        "--progress",
      ],
      inspect: [
        "--signal-config <path>",
        "--signal <id> (repeatable)",
        "--selector <name=css|json> (repeatable)",
        "--signal-concurrency <n>",
        "--signal-timeout-ms <ms>",
        "--tab <id> (repeatable)",
        "--group <name>",
        "--group-id <id>",
        "--window <id>",
        "--all",
        "--progress",
      ],
      focus: [
        "--tab <id>",
      ],
      open: [
        "--url <url> (repeatable)",
        "--group <name>",
        "--after-group <name>",
        "--window <id>",
        "--window-group <name>",
        "--window-tab <id>",
        "--window-url <substring>",
      ],
      "group-assign": [
        "--tab <id> (repeatable)",
        "--group <name>",
        "--group-id <id>",
        "--window <id>",
        "--create",
        "--color <name>",
        "--collapsed",
        "--expanded",
      ],
      "move-tab": [
        "--tab <id>",
        "--before-tab <id>",
        "--after-tab <id>",
        "--before-group <name>",
        "--after-group <name>",
        "--window <id>",
      ],
      "move-group": [
        "--group <name>",
        "--group-id <id>",
        "--before-tab <id>",
        "--after-tab <id>",
        "--before-group <name>",
        "--after-group <name>",
        "--window <id>",
      ],
      setup: [
        "--browser edge|chrome",
        "--extension-id <id>",
        "--node <path>",
      ],
      policy: [
        "--init",
      ],
      archive: [
        "--all",
        "--window <id>",
        "--group <name>",
        "--group-id <id>",
        "--tab <id> (repeatable)",
      ],
      close: [
        "--apply <analysisId>",
        "--tab <id> (repeatable)",
        "--group <name>",
        "--group-id <id>",
        "--window <id>",
        "--confirm",
        "--dry-run",
      ],
      report: [
        "--format json|md|csv",
        "--out <path>",
        "--tab <id> (repeatable)",
        "--group <name>",
        "--group-id <id>",
        "--window <id>",
        "--all",
      ],
      history: [
        "--limit <n>",
      ],
      global: [
        "--help",
        "--json",
      ],
    },
  };
}

function printHelp(jsonOutput: boolean) {
  const data = buildHelpData();
  if (jsonOutput) {
    printJson({ ok: true, data });
    return;
  }

  const lines: string[] = [];
  lines.push("tabctl - Edge tab management CLI");
  lines.push("");
  lines.push(`Usage: ${data.usage}`);
  lines.push("");
  lines.push("Commands:");
  lines.push(`  ${data.commands.join(", ")}`);
  lines.push("");
  lines.push("Options:");
  for (const [section, options] of Object.entries(data.options)) {
    lines.push(`  ${section}:`);
    for (const option of options as string[]) {
      lines.push(`    ${option}`);
    }
  }
  lines.push("");
  lines.push("Policy: $XDG_CONFIG_HOME/tabctl/policy.json (or ~/.config/tabctl/policy.json)");
  lines.push("Policy is enforced when the file exists; missing file means no policy.");
  process.stdout.write(lines.join("\n") + "\n");
}

function errorOut(message: string): never {
  printJson({ ok: false, error: { message } });
  process.exit(1);
  throw new Error(message);
}

async function main() {
  let { command, options } = parseArgs(process.argv.slice(2));
  const prettyOutput = options.pretty !== false;
  if (Object.prototype.hasOwnProperty.call(options, "policy")) {
    errorOut("Custom policy path is not supported. Use XDG_CONFIG_HOME/tabctl/policy.json.");
  }

  const policyContext = loadPolicy();
  const policySummary = summarizePolicy(policyContext.policy, policyContext.path);
  const policyEnabled = policyContext.policy !== null;
  const enforcePolicy = policyEnabled;
  const includeWindowTitle = options["window-title"] === true;
  let policySnapshot: Record<string, unknown> | null = null;
  const getPolicySnapshot = async () => {
    if (!policySnapshot) {
      policySnapshot = await fetchSnapshot();
    }
    return policySnapshot;
  };

  if (!command || command === "help" || options.help) {
    printHelp(options.json === true);
    return;
  }

  if (command === "setup") {
    runSetup(options, prettyOutput);
    return;
  }

  if (command === "policy") {
    const policyPath = defaultPolicyPath();
    if (options.init) {
      if (fs.existsSync(policyPath)) {
        printJson({
          ok: true,
          data: {
            status: "exists",
            path: policyPath,
          },
        }, prettyOutput);
        return;
      }

      const dir = path.dirname(policyPath);
      fs.mkdirSync(dir, { recursive: true });
      fs.writeFileSync(policyPath, JSON.stringify(defaultPolicyTemplate(), null, 2), "utf8");
      printJson({
        ok: true,
        data: {
          status: "created",
          path: policyPath,
        },
      }, prettyOutput);
      return;
    }

    printJson({
      ok: true,
      data: {
        ...policySummary,
        path: policyPath,
      },
    }, prettyOutput);
    return;
  }

  if (command === "close" && options["dry-run"]) {
    command = "analyze";
  }

  let action = command;
  let params: Record<string, unknown> = {};
  let policyInfo: Record<string, unknown> | null = null;
  let earlyResponse: Record<string, unknown> | null = null;

  switch (command) {
    case "list":
      action = "list";
      break;
    case "ping":
      action = "ping";
      break;
    case "analyze":
      action = "analyze";
      params = {
        staleDays: options["stale-days"] ? Number(options["stale-days"]) : undefined,
        checkGitHub: Boolean(options.github),
        tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
        githubConcurrency: options["github-concurrency"] ? Number(options["github-concurrency"]) : undefined,
        githubTimeoutMs: options["github-timeout-ms"] ? Number(options["github-timeout-ms"]) : undefined,
        progress: Boolean(options.progress),
      };
      break;
    case "inspect":
      action = "inspect";
      let selectorSpecs: Array<Record<string, unknown>> | undefined;
      if (options.selector) {
        selectorSpecs = (options.selector as string[]).map((value) => {
          const trimmed = value.trim();
          if (trimmed.startsWith("{")) {
            try {
              return JSON.parse(trimmed) as Record<string, unknown>;
            } catch (error) {
              errorOut(`Invalid selector JSON: ${trimmed}`);
            }
          }
          if (trimmed.includes("=")) {
            const [name, selector] = trimmed.split(/=(.+)/);
            return { name, selector };
          }
          return { selector: trimmed };
        }).filter(Boolean) as Array<Record<string, unknown>>;
      }

      let signalConfig: Record<string, unknown> | undefined;
      if (options["signal-config"]) {
        try {
          const configRaw = fs.readFileSync(String(options["signal-config"]), "utf8");
          signalConfig = JSON.parse(configRaw) as Record<string, unknown>;
        } catch (error) {
          errorOut("Failed to read --signal-config file");
        }
      }

      params = {
        all: Boolean(options.all),
        windowId: options.window ? Number(options.window) : undefined,
        groupTitle: options.group,
        groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
        tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
        signals: options.signal ? (options.signal as string[]) : undefined,
        selectorSpecs,
        signalConfig,
        signalConcurrency: options["signal-concurrency"] ? Number(options["signal-concurrency"]) : undefined,
        signalTimeoutMs: options["signal-timeout-ms"] ? Number(options["signal-timeout-ms"]) : undefined,
        progress: Boolean(options.progress),
      };
      break;
    case "focus":
      action = "focus";
      params = {
        tabId: options.tab ? Number((options.tab as string[])[0]) : undefined,
        tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
      };
      break;
    case "open":
      action = "open";
      params = {
        urls: options.url ? (options.url as string[]).map(String) : undefined,
        groupTitle: options.group,
        afterGroupTitle: options["after-group"],
        windowId: options.window ? Number(options.window) : undefined,
        windowGroupTitle: options["window-group"],
        windowTabId: options["window-tab"] ? Number(options["window-tab"]) : undefined,
        windowUrl: options["window-url"],
      };
      break;
    case "group-assign":
      action = "group-assign";
      params = {
        tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
        groupTitle: options.group,
        groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
        windowId: options.window ? Number(options.window) : undefined,
        create: Boolean(options.create),
        color: options.color,
        collapsed: options.collapsed === true ? true : options.expanded === true ? false : undefined,
      };
      break;
    case "move-tab":
      action = "move-tab";
      params = {
        tabId: options.tab ? Number((options.tab as string[])[0]) : undefined,
        tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
        beforeTabId: options["before-tab"] ? Number(options["before-tab"]) : undefined,
        afterTabId: options["after-tab"] ? Number(options["after-tab"]) : undefined,
        beforeGroupTitle: options["before-group"],
        afterGroupTitle: options["after-group"],
        windowId: options.window ? Number(options.window) : undefined,
      };
      break;
    case "move-group":
      action = "move-group";
      params = {
        groupTitle: options.group,
        groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
        beforeTabId: options["before-tab"] ? Number(options["before-tab"]) : undefined,
        afterTabId: options["after-tab"] ? Number(options["after-tab"]) : undefined,
        beforeGroupTitle: options["before-group"],
        afterGroupTitle: options["after-group"],
        windowId: options.window ? Number(options.window) : undefined,
      };
      break;
    case "archive":
      action = "archive";
      params = {
        all: Boolean(options.all),
        windowId: options.window ? Number(options.window) : undefined,
        groupTitle: options.group,
        groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
        tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
      };
      break;
    case "close":
      action = "close";
      if (options.apply) {
        params = { mode: "apply", analysisId: options.apply };
      } else {
        if (!options.confirm) {
          errorOut("Direct close requires --confirm");
        }
        params = {
          mode: "direct",
          confirmed: true,
          windowId: options.window ? Number(options.window) : undefined,
          groupTitle: options.group,
          groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
          tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
        };
      }
      break;
    case "report":
      action = "report";
      params = {
        all: Boolean(options.all),
        windowId: options.window ? Number(options.window) : undefined,
        groupTitle: options.group,
        groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
        tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
      };
      break;
    case "undo":
      action = "undo";
      params = { txid: options._[0] };
      break;
    case "history":
      action = "history";
      params = { limit: options.limit ? Number(options.limit) : undefined };
      break;
    default:
      errorOut(`Unknown command: ${command}`);
  }

  if (enforcePolicy && ["analyze", "inspect", "report", "close", "archive", "focus", "move-tab", "move-group", "group-assign"].includes(command)) {
    if (command === "close" && options.apply) {
      errorOut("Policy blocks close --apply; use explicit tab targets.");
    }

    const snapshot = await getPolicySnapshot();
    if (!snapshot) {
      errorOut("Failed to load tabs for policy evaluation");
    }

    const selection = selectTabsFromSnapshot(snapshot, params);
    if ((selection as { error?: Record<string, unknown> }).error) {
      errorOut("Failed to resolve selection for policy evaluation");
    }

    const selectedTabs = (selection as { tabs: Array<Record<string, unknown>> }).tabs;
    const eligibleTabs = selectedTabs.filter((tab) => evaluateTab(tab, policyContext.policy).eligible);
    const protectedTabs = selectedTabs.filter((tab) => !evaluateTab(tab, policyContext.policy).eligible);
    const eligibleIds = eligibleTabs.map((tab) => tab.tabId).filter((id) => typeof id === "number") as number[];

    if (command === "focus") {
      if (!eligibleIds.length) {
        errorOut("Tab is protected by policy and cannot be focused via CLI");
      }
      params = {
        tabId: eligibleIds[0],
        tabIds: eligibleIds,
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

  const request = { id: createId(), action, params };

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
    if (command === "list" && Array.isArray(data.windows)) {
      const filtered = filterSnapshotByPolicy(data, policyContext.policy);
      response.data = filtered;
    }

    if ((command === "inspect" || command === "report") && Array.isArray(data.entries)) {
      let snapshot: Record<string, unknown> | null = null;
      if (policyEnabled) {
        snapshot = await getPolicySnapshot();
      }
      const tabIndex = snapshot ? buildTabIndex(snapshot) : null;
      data.entries = (data.entries as Array<Record<string, unknown>>).map((entry) => {
        const tab = tabIndex?.get(entry.tabId as number) || entry;
        const { eligible, protectedReasons } = evaluateTab(tab, policyContext.policy);
        return {
          ...entry,
          eligible,
          protectedReasons,
        };
      }).filter((entry) => entry.eligible !== false);
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

  if (command === "report") {
    const format = (options.format as string) || "json";
    const data = response.data as { entries?: Array<Record<string, unknown>>; generatedAt?: number } | undefined;
    const entries = data?.entries || [];
    const generatedAt = data?.generatedAt;
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
      printJson({ ok: true, data: { writtenTo: options.out, format, count: entries.length } }, prettyOutput);
      return;
    }

    if (format === "json") {
      printJson({ ok: true, data: { format, entries } }, prettyOutput);
      return;
    }

    printJson({ ok: true, data: { format, entries, content } }, prettyOutput);
    return;
  }

  printJson(response, prettyOutput);
}

main().catch((error: Error) => {
  errorOut(error.message || "Unknown error");
});
