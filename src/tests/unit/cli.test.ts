import assert from "node:assert/strict";
import fs from "fs";
import os from "os";
import path from "path";
import { spawn } from "node:child_process";
import test from "node:test";
import { startMockSocket, stopMockSocket } from "./socket";

const cliPath = path.resolve(__dirname, "../../cli/tabctl.js");
const testConfigHome = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-test-config-"));

async function runCli(args: string[], socketPath?: string, extraEnv?: Record<string, string>) {
  const env = { ...process.env };
  if (socketPath) {
    env.TABCTL_SOCKET = socketPath;
  }
  if (extraEnv) {
    Object.assign(env, extraEnv);
  }
  const hasCustomConfig = extraEnv && Object.prototype.hasOwnProperty.call(extraEnv, "XDG_CONFIG_HOME");
  if (!hasCustomConfig) {
    env.XDG_CONFIG_HOME = testConfigHome;
  }

  return new Promise<{ status: number | null; stdout: string; stderr: string }>((resolve, reject) => {
    const child = spawn(process.execPath, [cliPath, ...args], { env });
    let stdout = "";
    let stderr = "";

    const timeout = setTimeout(() => {
      child.kill("SIGKILL");
      reject(new Error("CLI timeout"));
    }, 2000);

    child.stdout.on("data", (data) => {
      stdout += data.toString();
    });

    child.stderr.on("data", (data) => {
      stderr += data.toString();
    });

    child.on("error", (error) => {
      clearTimeout(timeout);
      reject(error);
    });

    child.on("exit", (code) => {
      clearTimeout(timeout);
      resolve({ status: code, stdout, stderr });
    });
  });
}

test("list sends list action", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { value: "ok" },
  }));

  const result = await runCli(["list"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
  assert.equal(requests.length, 1);
  assert.equal(requests[0].action, "list");
});

test("close without confirm fails", async () => {
  const result = await runCli(["close", "--tab", "123"]);
  assert.equal(result.status, 1);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, false);
  assert.equal(output.error.message, "Direct close requires --confirm");
});

test("close --dry-run maps to analyze", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { candidates: [], totals: { tabs: 0, candidates: 0 } },
  }));

  const result = await runCli(["close", "--dry-run"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "analyze");
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
});

test("analyze passes tab ids and github options", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { candidates: [], totals: { tabs: 0, candidates: 0 } },
  }));

  const result = await runCli([
    "analyze",
    "--tab",
    "12",
    "--github",
    "--github-concurrency",
    "3",
    "--github-timeout-ms",
    "2500",
    "--progress",
  ], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "analyze");
  const params = requests[0].params as { tabIds?: number[]; checkGitHub?: boolean; githubConcurrency?: number; githubTimeoutMs?: number; progress?: boolean } | undefined;
  assert.deepEqual(params?.tabIds, [12]);
  assert.equal(params?.checkGitHub, true);
  assert.equal(params?.githubConcurrency, 3);
  assert.equal(params?.githubTimeoutMs, 2500);
  assert.equal(params?.progress, true);
});

test("analyze defaults to all scope", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { candidates: [], totals: { tabs: 0, candidates: 0 } },
  }));

  const result = await runCli(["analyze"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "analyze");
  const params = requests[0].params as { all?: boolean } | undefined;
  assert.equal(params?.all, true);
});

test("analyze passes scope selectors", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { candidates: [], totals: { tabs: 0, candidates: 0 } },
  }));

  const result = await runCli([
    "analyze",
    "--window",
    "2",
    "--group",
    "Work",
    "--group-id",
    "7",
    "--all",
  ], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "analyze");
  const params = requests[0].params as { windowId?: number; groupTitle?: string; groupId?: number; all?: boolean } | undefined;
  assert.equal(params?.windowId, 2);
  assert.equal(params?.groupTitle, "Work");
  assert.equal(params?.groupId, 7);
  assert.equal(params?.all, true);
});

test("analyze --window-title includes window title", async () => {
  const snapshot = {
    windows: [
      {
        windowId: 1,
        focused: true,
        tabs: [
          { tabId: 10, windowId: 1, active: true, title: "Window A", url: "https://example.com" },
          { tabId: 11, windowId: 1, active: false, title: "Duplicate", url: "https://example.com" },
        ],
        groups: [],
      },
    ],
  };
  const analyzeData = {
    generatedAt: 1700000000000,
    staleDays: 30,
    totals: { tabs: 2, analyzed: 2, candidates: 1 },
    meta: { durationMs: 0, githubChecked: 0, githubTotal: 0, githubMatched: 0, githubTimeoutMs: 4000 },
    candidates: [
      {
        tabId: 11,
        windowId: 1,
        groupId: -1,
        url: "https://example.com",
        title: "Duplicate",
        lastFocusedAt: null,
        reasons: [{ type: "duplicate", detail: "Matches tab 10" }],
        severity: "high",
      },
    ],
    analysisId: "analysis-1",
  };

  const { socketPath, server, sockets } = await startMockSocket((req) => {
    if (req.action === "list") {
      return { ok: true, action: req.action, requestId: req.id, data: snapshot };
    }
    return { ok: true, action: req.action, requestId: req.id, data: analyzeData };
  });

  const result = await runCli(["analyze", "--window-title"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
  assert.equal(output.data.candidates[0].windowTitle, "Window A");
});

test("report format md returns markdown content", async () => {
  const { socketPath, server, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: {
      generatedAt: 1700000000000,
      entries: [
        {
          windowId: 1,
          windowLabel: "W1",
          groupTitle: "Test",
          title: "Example",
          url: "https://example.com",
          description: "Desc",
        },
      ],
    },
  }));

  const result = await runCli(["report", "--format", "md"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
  assert.equal(output.data.format, "md");
  assert.match(output.data.content, /# Tab Report/);
});

test("inspect passes signal options", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { entries: [] },
  }));

  const result = await runCli([
    "inspect",
    "--tab",
    "42",
    "--signal",
    "page-meta",
    "--signal",
    "github-state",
    "--signal",
    "selector",
    "--selector",
    "price=.price",
    "--signal-concurrency",
    "2",
    "--signal-timeout-ms",
    "1500",
    "--progress",
  ], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "inspect");
  const params = requests[0].params as { tabIds?: number[]; signals?: string[]; selectorSpecs?: Array<Record<string, unknown>>; signalConcurrency?: number; signalTimeoutMs?: number; progress?: boolean } | undefined;
  assert.deepEqual(params?.tabIds, [42]);
  assert.deepEqual(params?.signals, ["page-meta", "github-state", "selector"]);
  assert.deepEqual(params?.selectorSpecs, [{ name: "price", selector: ".price" }]);
  assert.equal(params?.signalConcurrency, 2);
  assert.equal(params?.signalTimeoutMs, 1500);
  assert.equal(params?.progress, true);
});

test("focus passes tab id", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { tabId: 99, windowId: 1 },
  }));

  const result = await runCli(["focus", "--tab", "99"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "focus");
  const params = requests[0].params as { tabId?: number } | undefined;
  assert.equal(params?.tabId, 99);
});

test("open passes urls and window selectors", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { summary: { createdTabs: 2 } },
  }));

  const result = await runCli([
    "open",
    "--url",
    "https://nos.nl",
    "--url",
    "https://nu.nl",
    "--group",
    "News",
    "--after-group",
    "Microsoft 365 migration",
    "--window",
    "3",
    "--window-group",
    "Graceful loader",
    "--window-tab",
    "42",
    "--window-url",
    "mail.google.com",
  ], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "open");
  const params = requests[0].params as {
    urls?: string[];
    groupTitle?: string;
    afterGroupTitle?: string;
    windowId?: number;
    windowGroupTitle?: string;
    windowTabId?: number;
    windowUrl?: string;
  } | undefined;
  assert.deepEqual(params?.urls, ["https://nos.nl", "https://nu.nl"]);
  assert.equal(params?.groupTitle, "News");
  assert.equal(params?.afterGroupTitle, "Microsoft 365 migration");
  assert.equal(params?.windowId, 3);
  assert.equal(params?.windowGroupTitle, "Graceful loader");
  assert.equal(params?.windowTabId, 42);
  assert.equal(params?.windowUrl, "mail.google.com");
});

test("open supports new window flag", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { summary: { createdTabs: 1 } },
  }));

  const result = await runCli([
    "open",
    "--new-window",
    "--url",
    "https://example.com",
  ], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "open");
  const params = requests[0].params as { newWindow?: boolean } | undefined;
  assert.equal(params?.newWindow, true);
});

test("group-list passes window option", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { groups: [] },
  }));

  const result = await runCli(["group-list", "--window", "3"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "group-list");
  const params = requests[0].params as { windowId?: number } | undefined;
  assert.equal(params?.windowId, 3);
});

test("group-update passes update options", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { groupId: 1 },
  }));

  const result = await runCli([
    "group-update",
    "--group",
    "Work",
    "--window",
    "2",
    "--title",
    "Work Items",
    "--color",
    "red",
    "--expanded",
  ], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "group-update");
  const params = requests[0].params as {
    groupTitle?: string;
    groupId?: number;
    windowId?: number;
    title?: string;
    color?: string;
    collapsed?: boolean;
  } | undefined;
  assert.equal(params?.groupTitle, "Work");
  assert.equal(params?.windowId, 2);
  assert.equal(params?.title, "Work Items");
  assert.equal(params?.color, "red");
  assert.equal(params?.collapsed, false);
});

test("group-ungroup passes group selectors", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { summary: { ungroupedTabs: 2 } },
  }));

  const result = await runCli([
    "group-ungroup",
    "--group-id",
    "99",
    "--window",
    "5",
  ], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "group-ungroup");
  const params = requests[0].params as { groupId?: number; windowId?: number } | undefined;
  assert.equal(params?.groupId, 99);
  assert.equal(params?.windowId, 5);
});

test("group-assign passes grouping options", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { summary: { groupedTabs: 2 } },
  }));

  const result = await runCli([
    "group-assign",
    "--tab",
    "12",
    "--tab",
    "15",
    "--group",
    "Research",
    "--window",
    "2",
    "--create",
    "--color",
    "blue",
    "--collapsed",
  ], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "group-assign");
  const params = requests[0].params as {
    tabIds?: number[];
    groupTitle?: string;
    groupId?: number;
    windowId?: number;
    create?: boolean;
    color?: string;
    collapsed?: boolean;
  } | undefined;
  assert.deepEqual(params?.tabIds, [12, 15]);
  assert.equal(params?.groupTitle, "Research");
  assert.equal(params?.windowId, 2);
  assert.equal(params?.create, true);
  assert.equal(params?.color, "blue");
  assert.equal(params?.collapsed, true);
});

test("move-tab supports new window flag", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { summary: { movedTabs: 1 } },
  }));

  const result = await runCli([
    "move-tab",
    "--tab",
    "12",
    "--new-window",
  ], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "move-tab");
  const params = requests[0].params as { newWindow?: boolean } | undefined;
  assert.equal(params?.newWindow, true);
});

test("move-group supports new window flag", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { summary: { movedTabs: 2 } },
  }));

  const result = await runCli([
    "move-group",
    "--group-id",
    "55",
    "--new-window",
  ], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "move-group");
  const params = requests[0].params as { newWindow?: boolean } | undefined;
  assert.equal(params?.newWindow, true);
});

test("merge-window passes window ids and close source", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { summary: { movedTabs: 3 } },
  }));

  const result = await runCli([
    "merge-window",
    "--from",
    "1",
    "--to",
    "2",
    "--close-source",
    "--confirm",
  ], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "merge-window");
  const params = requests[0].params as {
    fromWindowId?: number;
    toWindowId?: number;
    windowId?: number;
    closeSource?: boolean;
    confirmed?: boolean;
  } | undefined;
  assert.equal(params?.fromWindowId, 1);
  assert.equal(params?.toWindowId, 2);
  assert.equal(params?.windowId, 1);
  assert.equal(params?.closeSource, true);
  assert.equal(params?.confirmed, true);
});

test("dedupe runs analyze then close on confirm", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => {
    if (req.action === "analyze") {
      return {
        ok: true,
        action: req.action,
        requestId: req.id,
        data: {
          generatedAt: Date.now(),
          staleDays: 30,
          totals: { tabs: 2, analyzed: 2, candidates: 1 },
          meta: { durationMs: 0, githubChecked: 0, githubTotal: 0, githubMatched: 0, githubTimeoutMs: 4000 },
          candidates: [
            {
              tabId: 12,
              windowId: 1,
              groupId: -1,
              url: "https://example.com",
              title: "Example",
              lastFocusedAt: null,
              reasons: [{ type: "duplicate", detail: "Matches tab 10" }],
              severity: "high",
            },
          ],
          analysisId: "analysis-1",
        },
      };
    }
    return {
      ok: true,
      action: req.action,
      requestId: req.id,
      data: { summary: { closedTabs: 1, skippedTabs: 0 } },
    };
  });

  const result = await runCli(["dedupe", "--confirm"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests.length, 2);
  assert.equal(requests[0].action, "analyze");
  assert.equal(requests[1].action, "close");
  const closeParams = requests[1].params as { tabIds?: number[]; expectedUrls?: Record<string, string> } | undefined;
  assert.deepEqual(closeParams?.tabIds, [12]);
  assert.equal(closeParams?.expectedUrls?.["12"], "https://example.com");
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.action, "dedupe");
  assert.equal(output.data?.summary?.closed, 1);
});

test("dedupe outputs nextCommand when not confirmed", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => {
    if (req.action === "analyze") {
      return {
        ok: true,
        action: req.action,
        requestId: req.id,
        data: {
          generatedAt: Date.now(),
          staleDays: 30,
          totals: { tabs: 2, analyzed: 2, candidates: 1 },
          meta: { durationMs: 0, githubChecked: 0, githubTotal: 0, githubMatched: 0, githubTimeoutMs: 4000 },
          candidates: [
            {
              tabId: 12,
              windowId: 1,
              groupId: -1,
              url: "https://example.com",
              title: "Example",
              lastFocusedAt: null,
              reasons: [{ type: "duplicate", detail: "Matches tab 10" }],
              severity: "high",
            },
          ],
          analysisId: "analysis-1",
        },
      };
    }
    return {
      ok: true,
      action: req.action,
      requestId: req.id,
      data: { summary: { closedTabs: 1, skippedTabs: 0 } },
    };
  });

  const result = await runCli(["dedupe"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests.length, 1);
  assert.equal(requests[0].action, "analyze");
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.action, "dedupe");
  assert.equal(output.data?.nextCommand, "tabctl close --apply analysis-1 --confirm");
});

test("move-tab passes target options", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { summary: { movedTabs: 1 } },
  }));

  const result = await runCli([
    "move-tab",
    "--tab",
    "12",
    "--after-tab",
    "99",
    "--window",
    "3",
  ], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "move-tab");
  const params = requests[0].params as {
    tabId?: number;
    tabIds?: number[];
    beforeTabId?: number;
    afterTabId?: number;
    beforeGroupTitle?: string;
    afterGroupTitle?: string;
    windowId?: number;
  } | undefined;
  assert.equal(params?.tabId, 12);
  assert.deepEqual(params?.tabIds, [12]);
  assert.equal(params?.afterTabId, 99);
  assert.equal(params?.windowId, 3);
});

test("move-group passes target options", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { summary: { movedTabs: 2 } },
  }));

  const result = await runCli([
    "move-group",
    "--group",
    "News",
    "--before-group",
    "Microsoft 365 migration",
    "--window",
    "3",
  ], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "move-group");
  const params = requests[0].params as {
    groupTitle?: string;
    groupId?: number;
    beforeTabId?: number;
    afterTabId?: number;
    beforeGroupTitle?: string;
    afterGroupTitle?: string;
    windowId?: number;
  } | undefined;
  assert.equal(params?.groupTitle, "News");
  assert.equal(params?.beforeGroupTitle, "Microsoft 365 migration");
  assert.equal(params?.windowId, 3);
});

test("setup writes native host manifest", async () => {
  if (process.platform !== "darwin") {
    return;
  }
  const homeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-setup-"));
  const extensionId = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
  const nodePath = process.execPath;
  const result = await runCli([
    "setup",
    "--browser",
    "edge",
    "--extension-id",
    extensionId,
    "--node",
    nodePath,
  ], undefined, { HOME: homeDir, XDG_STATE_HOME: path.join(homeDir, ".local", "state") });

  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim()) as { ok: boolean; data: Record<string, unknown> };
  assert.equal(output.ok, true);

  const wrapperPath = path.join(homeDir, ".local", "state", "tabctl", "tabctl-host.sh");
  const manifestPath = path.join(
    homeDir,
    "Library",
    "Application Support",
    "Microsoft Edge",
    "NativeMessagingHosts",
    "com.erwinkroon.tabctl.json",
  );
  assert.equal(output.data.wrapperPath, wrapperPath);
  assert.equal(output.data.manifestPath, manifestPath);
  assert.ok(fs.existsSync(wrapperPath));
  assert.ok(fs.existsSync(manifestPath));

  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8")) as { name?: string; path?: string; allowed_origins?: string[] };
  assert.equal(manifest.name, "com.erwinkroon.tabctl");
  assert.equal(manifest.path, wrapperPath);
  assert.deepEqual(manifest.allowed_origins, [`chrome-extension://${extensionId}/`]);

  const wrapper = fs.readFileSync(wrapperPath, "utf8");
  const hostPath = path.resolve(__dirname, "../../host/host.js");
  assert.ok(wrapper.includes(nodePath));
  assert.ok(wrapper.includes(hostPath));
});

test("policy init creates default file", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-policy-init-"));
  const result = await runCli(["policy", "--init"], undefined, { XDG_CONFIG_HOME: dir });

  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
  const policyPath = path.join(dir, "tabctl", "policy.json");
  assert.ok(fs.existsSync(policyPath));
  const raw = fs.readFileSync(policyPath, "utf8");
  assert.match(raw, /"pinned"/);
});

test("help outputs plain text by default", async () => {
  const result = await runCli(["help"]);
  assert.equal(result.status, 0);
  assert.match(result.stdout, /tabctl - Edge tab management CLI/);
});

test("help supports --help flag", async () => {
  const result = await runCli(["--help"]);
  assert.equal(result.status, 0);
  assert.match(result.stdout, /tabctl - Edge tab management CLI/);
});

test("help supports json output", async () => {
  const result = await runCli(["help", "--json"]);
  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
  assert.ok(output.data?.commands);
  assert.equal(output.data?.version, "0.1.0");
  const options = output.data?.options as Record<string, string[]> | undefined;
  assert.ok(options);
  assert.ok(options?.analyze?.includes("--window-title (include active window title)"));
  assert.ok(options?.analyze?.includes("--group <name>"));
  assert.ok(options?.analyze?.includes("--group-id <id>"));
  assert.ok(options?.analyze?.includes("--window <id>"));
  assert.ok(options?.analyze?.includes("--all"));
  assert.ok(options?.dedupe?.includes("--include-stale"));
  assert.ok(options?.dedupe?.includes("--confirm"));
  assert.ok(options?.list?.includes("--groups (alias for group-list)"));
  assert.ok(options?.open?.includes("--url <url> (repeatable)"));
  assert.ok(options?.open?.includes("--after-group <name>"));
  assert.ok(options?.open?.includes("--new-window"));
  assert.ok(options?.["group-list"]?.includes("--window <id>"));
  assert.ok(options?.["group-update"]?.includes("--title <name>"));
  assert.ok(options?.["group-ungroup"]?.includes("--group-id <id>"));
  assert.ok(options?.["group-assign"]?.includes("--create"));
  assert.ok(options?.["move-tab"]?.includes("--after-tab <id>"));
  assert.ok(options?.["move-tab"]?.includes("--new-window"));
  assert.ok(options?.["move-group"]?.includes("--before-group <name>"));
  assert.ok(options?.["move-group"]?.includes("--new-window"));
  assert.ok(options?.["merge-window"]?.includes("--from <id>"));
  assert.ok(options?.["merge-window"]?.includes("--close-source"));
  assert.ok(options?.setup?.includes("--browser edge|chrome"));
  assert.ok(options?.report?.includes("--format json|md|csv"));
  assert.ok(options?.close?.includes("--dry-run"));
  assert.ok(options?.history?.includes("--limit <n>"));
  assert.ok(options?.version);
});

test("version outputs cli version", async () => {
  const result = await runCli(["version"]);
  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
  assert.equal(output.data?.version, "0.1.0");
  assert.equal(output.data?.component, "cli");
  assert.equal(output.data?.baseVersion, "0.1.0");
});

test("version includes dev sha when built", async () => {
  const result = await runCli(["version"]);
  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
  const version = output.data?.version as string | undefined;
  assert.ok(version);
  if (version && version.includes("-dev.")) {
    assert.match(version, /^0\.1\.0-dev\.[0-9a-f]{8}(\.dirty)?$/);
  } else {
    assert.equal(version, "0.1.0");
  }
  assert.equal(output.data?.baseVersion, "0.1.0");
});

test("undo sends undo action with txid", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { summary: { restoredTabs: 1 } },
  }));

  const result = await runCli(["undo", "tx-123"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests.length, 1);
  assert.equal(requests[0].action, "undo");
  const params = requests[0].params as { txid?: string } | undefined;
  assert.equal(params?.txid, "tx-123");
});

test("group-assign returns txid", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    version: "0.1.0",
    component: "host",
    data: {
      txid: "tx-group-assign",
      summary: { groupedTabs: 1 },
      extensionVersion: "0.1.0",
      extensionComponent: "extension",
      hostBaseVersion: "0.1.0",
    },
  }));

  const result = await runCli(["group-assign", "--tab", "42", "--group", "Work"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "group-assign");
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.data?.txid, "tx-group-assign");
  assert.equal(output.version, "0.1.0");
  assert.equal(output.component, "host");
  assert.equal(output.data?.extensionVersion, "0.1.0");
  assert.equal(output.data?.hostBaseVersion, "0.1.0");
});

test("move-tab returns txid", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    version: "0.1.0",
    component: "host",
    data: {
      txid: "tx-move-tab",
      summary: { movedTabs: 1 },
      extensionVersion: "0.1.0",
      extensionComponent: "extension",
      hostBaseVersion: "0.1.0",
    },
  }));

  const result = await runCli(["move-tab", "--tab", "12", "--after-tab", "13"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "move-tab");
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.data?.txid, "tx-move-tab");
  assert.equal(output.version, "0.1.0");
  assert.equal(output.component, "host");
  assert.equal(output.data?.extensionVersion, "0.1.0");
  assert.equal(output.data?.hostBaseVersion, "0.1.0");
});
