import assert from "node:assert/strict";
import fs from "fs";
import os from "os";
import path from "path";
import { spawn } from "node:child_process";
import test from "node:test";
import { startMockSocket, stopMockSocket } from "./socket";

const cliPath = path.resolve(__dirname, "../../cli/tabctl.js");
const pkgVersion = JSON.parse(fs.readFileSync(path.resolve(__dirname, "../../package.json"), "utf8")).version;
const testConfigHome = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-test-config-"));

async function runCli(
  args: string[],
  socketPath?: string,
  extraEnv?: Record<string, string>,
  cliOverride?: string,
  npxOverride?: string,
) {
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
    const effectiveCli = cliOverride || cliPath;
    const effectiveEnv = { ...env };
    if (npxOverride) {
      effectiveEnv.PATH = `${path.dirname(npxOverride)}${path.delimiter}${env.PATH || ""}`;
    }
    const child = spawn(process.execPath, [effectiveCli, ...args], { env: effectiveEnv });
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

async function runCliWithStdin(
  args: string[],
  stdinData: string,
  extraEnv?: Record<string, string>,
) {
  const env = { ...process.env };
  if (extraEnv) {
    Object.assign(env, extraEnv);
  }
  const hasCustomConfig = extraEnv && Object.prototype.hasOwnProperty.call(extraEnv, "XDG_CONFIG_HOME");
  if (!hasCustomConfig) {
    env.XDG_CONFIG_HOME = testConfigHome;
  }

  return new Promise<{ status: number | null; stdout: string; stderr: string }>((resolve, reject) => {
    const child = spawn(process.execPath, [cliPath, ...args], { env, stdio: ["pipe", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";

    const timeout = setTimeout(() => {
      child.kill("SIGKILL");
      reject(new Error("CLI timeout"));
    }, 5000);

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

    child.stdin.write(stdinData);
    child.stdin.end();
  });
}

function assertVersion(version: string | undefined) {
  assert.ok(version);
  if (version && version.includes("-dev.")) {
    const re = new RegExp(`^${pkgVersion.replace(/\./g, "\\.")}-dev\\.[0-9a-f]{8}(\\.dirty)?$`);
    assert.match(version, re);
  } else {
    assert.equal(version, pkgVersion);
  }
}

test("list sends list action", async () => {
  const { socketPath, server, sockets, requests } = await startMockSocket((req) => ({
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

test("ping sends ping action", async () => {
  const { socketPath, server, sockets, requests } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { value: "pong" },
  }));

  const result = await runCli(["ping"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
  assert.equal(requests.length, 1);
  assert.equal(requests[0].action, "ping");
});

test("list paginates and filters by group", async () => {
  const snapshot = {
    windows: [
      {
        windowId: 1,
        focused: true,
        tabs: [
          { tabId: 1, windowId: 1, index: 0, groupId: 10, groupTitle: "Work", title: "A", url: "https://a" },
          { tabId: 2, windowId: 1, index: 1, groupId: 10, groupTitle: "Work", title: "B", url: "https://b" },
          { tabId: 3, windowId: 1, index: 2, groupId: -1, groupTitle: null, title: "C", url: "https://c" },
        ],
        groups: [
          { groupId: 10, title: "Work" },
        ],
      },
      {
        windowId: 2,
        focused: false,
        tabs: [
          { tabId: 4, windowId: 2, index: 0, groupId: 11, groupTitle: "Home", title: "D", url: "https://d" },
        ],
        groups: [
          { groupId: 11, title: "Home" },
        ],
      },
    ],
  };

  const { socketPath, server, requests, sockets } = await startMockSocket((req) => {
    if (req.action === "list") {
      return { ok: true, action: req.action, requestId: req.id, data: snapshot };
    }
    return { ok: true, action: req.action, requestId: req.id, data: {} };
  });

  const result = await runCli(["list", "--group", "Work", "--limit", "1"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "list");
  const output = JSON.parse(result.stdout.trim());
  const data = output.data as { windows?: Array<Record<string, unknown>>; page?: Record<string, unknown> };
  assert.ok(data.page);
  assert.equal(data.page?.limit, 1);
  assert.equal(data.page?.offset, 0);
  assert.equal(data.page?.returned, 1);
  assert.equal(data.page?.total, 2);
  assert.equal(data.page?.hasMore, true);
  assert.equal(data.page?.nextOffset, 1);
  assert.match(data.page?.hint as string, /tabctl list/);
  assert.match(data.page?.hint as string, /--group ("Work"|Work)/);
  assert.equal(data.windows?.length, 1);
  const tabs = (data.windows?.[0].tabs as Array<Record<string, unknown>>) || [];
  assert.equal(tabs.length, 1);
  assert.equal(tabs[0].tabId, 1);
});

test("list with --all ignores other scopes", async () => {
  const snapshot = {
    windows: [
      {
        windowId: 1,
        focused: true,
        tabs: [
          { tabId: 1, windowId: 1, index: 0, groupId: 10, groupTitle: "Work", title: "A", url: "https://a" },
          { tabId: 2, windowId: 1, index: 1, groupId: -1, groupTitle: null, title: "B", url: "https://b" },
        ],
        groups: [
          { groupId: 10, title: "Work" },
        ],
      },
    ],
  };

  const { socketPath, server, sockets } = await startMockSocket((req) => {
    if (req.action === "list") {
      return { ok: true, action: req.action, requestId: req.id, data: snapshot };
    }
    return { ok: true, action: req.action, requestId: req.id, data: {} };
  });

  const result = await runCli(["list", "--group", "Work", "--all", "--limit", "100"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  const data = output.data as { windows?: Array<Record<string, unknown>> };
  const tabs = ((data.windows?.[0].tabs as Array<Record<string, unknown>>) || []);
  assert.equal(tabs.length, 2);
});

test("list supports --ungrouped", async () => {
  const snapshot = {
    windows: [
      {
        windowId: 1,
        focused: true,
        tabs: [
          { tabId: 1, windowId: 1, index: 0, groupId: -1, groupTitle: null, title: "A", url: "https://a" },
          { tabId: 2, windowId: 1, index: 1, groupId: 10, groupTitle: "Work", title: "B", url: "https://b" },
        ],
        groups: [
          { groupId: 10, title: "Work" },
        ],
      },
    ],
  };

  const { socketPath, server, sockets } = await startMockSocket((req) => {
    if (req.action === "list") {
      return { ok: true, action: req.action, requestId: req.id, data: snapshot };
    }
    return { ok: true, action: req.action, requestId: req.id, data: {} };
  });

  const result = await runCli(["list", "--ungrouped", "--limit", "10"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  const data = output.data as { windows?: Array<Record<string, unknown>> };
  const tabs = ((data.windows?.[0].tabs as Array<Record<string, unknown>>) || []);
  assert.equal(tabs.length, 1);
  assert.equal(tabs[0].tabId, 1);
});

test("list rejects invalid window value", async () => {
  const result = await runCli(["list", "--window", "nope"]);
  assert.equal(result.status, 1);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, false);
  assert.match(output.error.message, /Invalid --window value/);
});

test("report supports --ungrouped", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { entries: [] },
  }));

  const result = await runCli(["report", "--ungrouped"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "report");
  const params = requests[0].params as { groupId?: number } | undefined;
  assert.equal(params?.groupId, -1);
});

test("report rejects --ungrouped with --group-id", async () => {
  const result = await runCli(["report", "--ungrouped", "--group-id", "3"]);
  assert.equal(result.status, 1);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, false);
  assert.equal(output.error.message, "--ungrouped cannot be combined with --group-id");
});

test("close without confirm fails", async () => {
  const result = await runCli(["close", "--tab", "123"]);
  assert.equal(result.status, 1);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, false);
  assert.equal(output.error.message, "Direct close requires --confirm");
});

test("close supports --ungrouped", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { summary: { closedTabs: 0, skippedTabs: 0 } },
  }));

  const result = await runCli(["close", "--ungrouped", "--confirm"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "close");
  const params = requests[0].params as { groupId?: number } | undefined;
  assert.equal(params?.groupId, -1);
});

test("close rejects --ungrouped with --group-id", async () => {
  const result = await runCli(["close", "--ungrouped", "--group-id", "3", "--confirm"]);
  assert.equal(result.status, 1);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, false);
  assert.equal(output.error.message, "--ungrouped cannot be combined with --group-id");
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

test("analyze supports --ungrouped", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { candidates: [], totals: { tabs: 0, candidates: 0 } },
  }));

  const result = await runCli(["analyze", "--ungrouped"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "analyze");
  const params = requests[0].params as { groupId?: number } | undefined;
  assert.equal(params?.groupId, -1);
});

test("analyze rejects --ungrouped with --group-id", async () => {
  const result = await runCli(["analyze", "--ungrouped", "--group-id", "3"]);
  assert.equal(result.status, 1);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, false);
  assert.equal(output.error.message, "--ungrouped cannot be combined with --group-id");
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
        {
          windowId: 1,
          windowLabel: "W1",
          groupTitle: "Test",
          title: "Example 2",
          url: "https://example.org",
          description: "Desc",
        },
      ],
    },
  }));

  const result = await runCli(["report", "--format", "md", "--limit", "1"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
  assert.equal(output.data.format, "md");
  assert.match(output.data.content, /# Tab Report/);
  assert.ok(output.data.page);
  assert.equal(output.data.page.limit, 1);
  assert.equal(output.data.page.total, 2);
});

test("inspect passes signal options", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { entries: [{ tabId: 42 }, { tabId: 43 }] },
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
    "--limit",
    "100",
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
  const output = JSON.parse(result.stdout.trim());
  assert.ok(output.data.page);
  assert.equal(output.data.page.limit, 100);
  assert.equal(output.data.page.total, 2);
});

test("inspect auto-adds selector signal", async () => {
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
    "--selector",
    "links=a",
  ], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const params = requests[0].params as { signals?: string[] } | undefined;
  assert.ok(params?.signals?.includes("selector"));
});

test("inspect rejects unknown signal", async () => {
  const result = await runCli(["inspect", "--signal", "links"]);
  assert.equal(result.status, 1);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, false);
  assert.match(output.error.message, /Unknown signal/);
});

test("inspect passes selector attr", async () => {
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
    "--selector",
    '{"name":"link","selector":"a[href]","attr":"href-url"}',
  ], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const params = requests[0].params as { selectorSpecs?: Array<Record<string, unknown>> } | undefined;
  assert.equal(params?.selectorSpecs?.[0]?.attr, "href-url");
});

test("inspect selector-attr applies to non-json selectors", async () => {
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
    "--selector",
    "teletekst=a[href*='teletekst']",
    "--selector-attr",
    "href-url",
  ], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const params = requests[0].params as { selectorSpecs?: Array<Record<string, unknown>> } | undefined;
  assert.equal(params?.selectorSpecs?.[0]?.attr, "href-url");
});

test("inspect selector-attr preserves explicit attr in JSON", async () => {
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
    "--selector",
    '{"name":"link","selector":"a[href]","attr":"href"}',
    "--selector-attr",
    "href-url",
  ], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const params = requests[0].params as { selectorSpecs?: Array<Record<string, unknown>> } | undefined;
  assert.equal(params?.selectorSpecs?.[0]?.attr, "href");
});

test("inspect passes wait-for options", async () => {
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
    "--wait-for",
    "dom",
    "--wait-timeout-ms",
    "9000",
  ], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const params = requests[0].params as { waitFor?: string; waitTimeoutMs?: number } | undefined;
  assert.equal(params?.waitFor, "dom");
  assert.equal(params?.waitTimeoutMs, 9000);
});

test("inspect passes wait-for settle option", async () => {
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
    "--wait-for",
    "settle",
    "--wait-timeout-ms",
    "5000",
  ], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const params = requests[0].params as { waitFor?: string; waitTimeoutMs?: number } | undefined;
  assert.equal(params?.waitFor, "settle");
  assert.equal(params?.waitTimeoutMs, 5000);
});

test("inspect rejects invalid selector-attr", async () => {
  const result = await runCli(["inspect", "--selector", "a[href]", "--selector-attr", "blob"]); 
  assert.equal(result.status, 1);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, false);
  assert.match(output.error.message, /Invalid --selector-attr/);
});

test("inspect rejects :contains selector", async () => {
  const result = await runCli(["inspect", "--selector", "a:contains(Hello)"]);
  assert.equal(result.status, 1);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, false);
  assert.match(output.error.message, /:contains\(\) is not supported/);
});

test("screenshot rejects --ungrouped with --group-id", async () => {
  const result = await runCli(["screenshot", "--ungrouped", "--group-id", "3"]);
  assert.equal(result.status, 1);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, false);
  assert.equal(output.error.message, "--ungrouped cannot be combined with --group-id");
});

test("screenshot passes capture options", async () => {
  const outDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-shots-"));
  const mockEntries = [{
    tabId: 42,
    tiles: [{
      index: 0,
      total: 1,
      dataUrl: "data:image/png;base64,SGVsbG8=",
    }],
  }];
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { entries: mockEntries },
  }));

  const result = await runCli([
    "screenshot",
    "--tab",
    "42",
    "--mode",
    "full",
    "--format",
    "jpeg",
    "--quality",
    "70",
    "--tile-max-dim",
    "1500",
    "--max-bytes",
    "2000000",
    "--wait-for",
    "load",
    "--wait-timeout-ms",
    "7000",
    "--out",
    outDir,
    "--progress",
  ], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "screenshot");
  const params = requests[0].params as {
    tabIds?: number[];
    mode?: string;
    format?: string;
    quality?: number;
    tileMaxDim?: number;
    maxBytes?: number;
    waitFor?: string;
    waitTimeoutMs?: number;
    outDir?: string;
    progress?: boolean;
  } | undefined;
  assert.deepEqual(params?.tabIds, [42]);
  assert.equal(params?.mode, "full");
  assert.equal(params?.format, "jpeg");
  assert.equal(params?.quality, 70);
  assert.equal(params?.tileMaxDim, 1500);
  assert.equal(params?.maxBytes, 2000000);
  assert.equal(params?.waitFor, "load");
  assert.equal(params?.waitTimeoutMs, 7000);
  assert.equal(params?.outDir, outDir);
  assert.equal(params?.progress, true);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
  assert.equal(output.data.files, 1);
  const tiles = output.data.entries?.[0]?.tiles as Array<Record<string, unknown>> | undefined;
  assert.ok(tiles);
  const tilePath = String(tiles?.[0]?.path || "");
  assert.equal(tilePath.includes(path.join(outDir, "42")), true);
});

test("screenshot defaults to .tabctl/screenshots directory", async () => {
  const mockEntries = [{
    tabId: 7,
    tiles: [{
      index: 0,
      total: 1,
      dataUrl: "data:image/png;base64,SGVsbG8=",
    }],
  }];
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { entries: mockEntries },
  }));

  const originalCwd = process.cwd();
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-cwd-"));
  process.chdir(tempDir);
  try {
    const result = await runCli([
      "screenshot",
      "--tab",
      "7",
    ], socketPath);

    assert.equal(result.status, 0);
    const output = JSON.parse(result.stdout.trim());
    assert.equal(output.ok, true);
    const writtenTo = String(output.data?.writtenTo || "");
    assert.ok(writtenTo.includes(path.join(tempDir, ".tabctl", "screenshots")));
    const tiles = output.data?.entries?.[0]?.tiles as Array<Record<string, unknown>> | undefined;
    assert.ok(tiles);
    const tilePath = String(tiles?.[0]?.path || "");
    assert.ok(tilePath.includes(path.join(tempDir, ".tabctl", "screenshots")));
  } finally {
    process.chdir(originalCwd);
    await stopMockSocket(server, socketPath, sockets);
  }
});

test("screenshot rejects invalid mode", async () => {
  const result = await runCli(["screenshot", "--mode", "giant"]);
  assert.equal(result.status, 1);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, false);
  assert.match(output.error.message, /Invalid --mode/);
});

test("screenshot rejects quality without jpeg", async () => {
  const result = await runCli(["screenshot", "--format", "png", "--quality", "50"]);
  assert.equal(result.status, 1);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, false);
  assert.match(output.error.message, /--quality requires --format jpeg/);
});

test("screenshot rejects tile-max-dim in viewport", async () => {
  const result = await runCli(["screenshot", "--mode", "viewport", "--tile-max-dim", "1000"]);
  assert.equal(result.status, 1);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, false);
  assert.match(output.error.message, /--tile-max-dim requires --mode full/);
});

test("screenshot rejects max-bytes in viewport", async () => {
  const result = await runCli(["screenshot", "--mode", "viewport", "--max-bytes", "1000"]);
  assert.equal(result.status, 1);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, false);
  assert.match(output.error.message, /--max-bytes requires --mode full/);
});

test("unknown --format hints to use --json", async () => {
  const result = await runCli(["list", "--format", "json"]);
  assert.equal(result.status, 1);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, false);
  assert.match(output.error.message, /Unknown option: --format/);
  assert.match(output.error.hint, /Use --json/);
});

test("report pagination includes next hint", async () => {
  const { socketPath, server, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: {
      generatedAt: 1700000000000,
      entries: [
        { tabId: 1, title: "One", url: "https://one" },
        { tabId: 2, title: "Two", url: "https://two" },
      ],
    },
  }));

  const result = await runCli(["report", "--limit", "1"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.data.page.hasMore, true);
  assert.match(output.data.page.hint, /tabctl report/);
});

test("inspect supports --ungrouped", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { entries: [] },
  }));

  const result = await runCli(["inspect", "--ungrouped"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "inspect");
  const params = requests[0].params as { groupId?: number } | undefined;
  assert.equal(params?.groupId, -1);
});

test("inspect rejects --ungrouped with --group-id", async () => {
  const result = await runCli(["inspect", "--ungrouped", "--group-id", "3"]);
  assert.equal(result.status, 1);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, false);
  assert.equal(output.error.message, "--ungrouped cannot be combined with --group-id");
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

test("refresh passes tab id", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { summary: { refreshedTabs: 1 } },
  }));

  const result = await runCli(["refresh", "--tab", "99"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "refresh");
  const params = requests[0].params as { tabId?: number } | undefined;
  assert.equal(params?.tabId, 99);
});

test("refresh requires --tab", async () => {
  const result = await runCli(["refresh"]);
  assert.equal(result.status, 1);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, false);
  assert.equal(output.error.message, "refresh requires --tab");
});

test("refresh rejects multiple --tab", async () => {
  const result = await runCli(["refresh", "--tab", "1", "--tab", "2"]);
  assert.equal(result.status, 1);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, false);
  assert.equal(output.error.message, "refresh requires a single --tab");
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
    "--color",
    "blue",
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
    color?: string;
    afterGroupTitle?: string;
    windowId?: number;
    windowGroupTitle?: string;
    windowTabId?: number;
    windowUrl?: string;
  } | undefined;
  assert.deepEqual(params?.urls, ["https://nos.nl", "https://nu.nl"]);
  assert.equal(params?.groupTitle, "News");
  assert.equal(params?.color, "blue");
  assert.equal(params?.afterGroupTitle, "Microsoft 365 migration");
  assert.equal(params?.windowId, 3);
  assert.equal(params?.windowGroupTitle, "Graceful loader");
  assert.equal(params?.windowTabId, 42);
  assert.equal(params?.windowUrl, "mail.google.com");
});

test("analyze does not force all when --window active", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { candidates: [] },
  }));

  const result = await runCli(["analyze", "--window", "active"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const params = requests[0].params as { all?: boolean; windowId?: string } | undefined;
  assert.equal(params?.windowId, "active");
  assert.ok(!params?.all);
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

test("open rejects before/after tab together", async () => {
  const result = await runCli([
    "open",
    "--url",
    "https://example.com",
    "--before-tab",
    "1",
    "--after-tab",
    "2",
  ]);
  assert.equal(result.status, 1);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, false);
  assert.match(output.error.message, /Only one target position is allowed/);
});

test("open supports after-tab", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { summary: { createdTabs: 1 } },
  }));

  const result = await runCli([
    "open",
    "--url",
    "https://example.com",
    "--after-tab",
    "55",
  ], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const params = requests[0].params as { afterTabId?: number } | undefined;
  assert.equal(params?.afterTabId, 55);
});

test("open rejects invalid color", async () => {
  const result = await runCli(["open", "--group", "Docs", "--color", "chartreuse", "--url", "https://example.com"]);
  assert.equal(result.status, 1);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, false);
  assert.equal(output.error?.message, "Invalid color: chartreuse. Use one of: grey, blue, red, yellow, green, pink, purple, cyan, orange");
});

test("open supports window new alias", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { summary: { createdTabs: 1 } },
  }));

  const result = await runCli([
    "open",
    "--window",
    "new",
    "--url",
    "https://example.com",
  ], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
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
  const params = requests[0].params as { windowId?: string } | undefined;
  assert.equal(params?.windowId, "3");
});

test("group-list supports --window active", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { groups: [] },
  }));

  const result = await runCli(["group-list", "--window", "active"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "group-list");
  const params = requests[0].params as { windowId?: string } | undefined;
  assert.equal(params?.windowId, "active");
});

test("group-list paginates and filters by tab", async () => {
  const snapshot = {
    windows: [
      {
        windowId: 1,
        focused: true,
        tabs: [
          { tabId: 1, windowId: 1, index: 0, groupId: 10, groupTitle: "Work", title: "A", url: "https://a" },
          { tabId: 2, windowId: 1, index: 1, groupId: -1, groupTitle: null, title: "B", url: "https://b" },
        ],
        groups: [
          { groupId: 10, title: "Work" },
        ],
      },
      {
        windowId: 2,
        focused: false,
        tabs: [
          { tabId: 3, windowId: 2, index: 0, groupId: 11, groupTitle: "Home", title: "C", url: "https://c" },
        ],
        groups: [
          { groupId: 11, title: "Home" },
        ],
      },
    ],
  };

  const { socketPath, server, requests, sockets } = await startMockSocket((req) => {
    if (req.action === "list") {
      return { ok: true, action: req.action, requestId: req.id, data: snapshot };
    }
    if (req.action === "group-list") {
      return {
        ok: true,
        action: req.action,
        requestId: req.id,
        data: {
          groups: [
            { windowId: 1, groupId: 10, title: "Work", tabCount: 1 },
            { windowId: 2, groupId: 11, title: "Home", tabCount: 1 },
          ],
        },
      };
    }
    return { ok: true, action: req.action, requestId: req.id, data: {} };
  });

  const result = await runCli(["group-list", "--tab", "1", "--limit", "1"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "group-list");
  assert.equal(requests.length, 2);
  assert.equal(requests[1].action, "list");
  const output = JSON.parse(result.stdout.trim());
  const data = output.data as { groups?: Array<Record<string, unknown>>; page?: Record<string, unknown> };
  assert.equal(data.groups?.length, 1);
  assert.equal(data.groups?.[0].groupId, 10);
  assert.equal(data.page?.total, 1);
  assert.equal(data.page?.hasMore, false);
});

test("group-list with --all skips tab scoping", async () => {
  const snapshot = {
    windows: [
      {
        windowId: 1,
        focused: true,
        tabs: [
          { tabId: 1, windowId: 1, index: 0, groupId: 10, groupTitle: "Work", title: "A", url: "https://a" },
        ],
        groups: [
          { groupId: 10, title: "Work" },
        ],
      },
    ],
  };

  const { socketPath, server, requests, sockets } = await startMockSocket((req) => {
    if (req.action === "list") {
      return { ok: true, action: req.action, requestId: req.id, data: snapshot };
    }
    if (req.action === "group-list") {
      return {
        ok: true,
        action: req.action,
        requestId: req.id,
        data: {
          groups: [
            { windowId: 1, groupId: 10, title: "Work", tabCount: 1 },
            { windowId: 1, groupId: 11, title: "Home", tabCount: 0 },
          ],
        },
      };
    }
    return { ok: true, action: req.action, requestId: req.id, data: {} };
  });

  const result = await runCli(["group-list", "--tab", "999", "--all", "--limit", "100"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "group-list");
  assert.equal(requests.length, 1);
  const output = JSON.parse(result.stdout.trim());
  const data = output.data as { groups?: Array<Record<string, unknown>> };
  assert.equal(data.groups?.length, 2);
});

test("group-list --all ignores window and group filters", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => {
    if (req.action === "group-list") {
      return {
        ok: true,
        action: req.action,
        requestId: req.id,
        data: {
          groups: [
            { windowId: 1, groupId: 10, title: "Work" },
            { windowId: 2, groupId: 20, title: "Home" },
          ],
        },
      };
    }
    return { ok: true, action: req.action, requestId: req.id, data: {} };
  });

  const result = await runCli(["group-list", "--all", "--window", "1", "--group", "Work"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "group-list");
  const params = requests[0].params as { windowId?: number } | undefined;
  assert.equal(params?.windowId, undefined);
  const output = JSON.parse(result.stdout.trim());
  const data = output.data as { groups?: Array<Record<string, unknown>> };
  assert.equal(data.groups?.length, 2);
});

test("group-list supports --ungrouped", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => {
    if (req.action === "group-list") {
      return {
        ok: true,
        action: req.action,
        requestId: req.id,
        data: {
          groups: [
            { windowId: 1, groupId: 10, title: "Work" },
            { windowId: 1, groupId: -1, title: "Ungrouped" },
          ],
        },
      };
    }
    return { ok: true, action: req.action, requestId: req.id, data: {} };
  });

  const result = await runCli(["group-list", "--ungrouped"], socketPath);
  await stopMockSocket(server, socketPath, sockets);
  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  assert.ok(output.ok);
  const groups = output.data?.groups as Array<{ groupId: number }> | undefined;
  assert.ok(groups);
  assert.equal(groups?.length, 1);
  assert.equal(groups?.[0]?.groupId, -1);
});

test("group-list falls back to snapshot when groups missing", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => {
    if (req.action === "group-list") {
      return { ok: true, action: req.action, requestId: req.id, data: { groups: null } };
    }
    if (req.action === "list") {
      return {
        ok: true,
        action: req.action,
        requestId: req.id,
        data: {
          windows: [
            {
              windowId: 7,
              tabs: [
                { tabId: 1, windowId: 7, index: 0, groupId: 33 },
              ],
              groups: [
                { groupId: 33, title: "Test", color: "blue", collapsed: false },
              ],
            },
          ],
        },
      };
    }
    return { ok: true, action: req.action, requestId: req.id, data: {} };
  });

  const result = await runCli(["group-list", "--window", "7"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "group-list");
  assert.equal(requests.length, 2);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
  const groups = output.data?.groups as Array<Record<string, unknown>>;
  assert.equal(groups.length, 1);
  assert.equal(groups[0].groupId, 33);
  assert.equal(groups[0].windowId, 7);
});

test("group-list fallback applies policy exclusions", async () => {
  const policyDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-policy-fallback-"));
  const policyPath = path.join(policyDir, "tabctl", "policy.json");
  fs.mkdirSync(path.dirname(policyPath), { recursive: true });
  fs.writeFileSync(
    policyPath,
    JSON.stringify({ protect: { groupTitles: ["Secret"] } }, null, 2),
    "utf8"
  );

  const { socketPath, server, sockets } = await startMockSocket((req) => {
    if (req.action === "group-list") {
      return { ok: true, action: req.action, requestId: req.id, data: { groups: null } };
    }
    if (req.action === "list") {
      return {
        ok: true,
        action: req.action,
        requestId: req.id,
        data: {
          windows: [
            {
              windowId: 7,
              tabs: [
                { tabId: 1, windowId: 7, index: 0, groupId: 33, groupTitle: "Secret" },
                { tabId: 2, windowId: 7, index: 1, groupId: 44, groupTitle: "Public" },
              ],
              groups: [
                { groupId: 33, title: "Secret", color: "blue", collapsed: false },
                { groupId: 44, title: "Public", color: "green", collapsed: false },
              ],
            },
          ],
        },
      };
    }
    return { ok: true, action: req.action, requestId: req.id, data: {} };
  });

  const result = await runCli(["group-list", "--window", "7"], socketPath, { XDG_CONFIG_HOME: policyDir });
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  const groups = output.data?.groups as Array<Record<string, unknown>>;
  assert.equal(groups.length, 1);
  assert.equal(groups[0].groupId, 44);
});

test("archive supports --ungrouped", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { summary: { movedTabs: 0 } },
  }));

  const result = await runCli(["archive", "--ungrouped"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "archive");
  const params = requests[0].params as { groupId?: number } | undefined;
  assert.equal(params?.groupId, -1);
});

test("archive rejects --ungrouped with --group-id", async () => {
  const result = await runCli(["archive", "--ungrouped", "--group-id", "3"]);
  assert.equal(result.status, 1);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, false);
  assert.equal(output.error.message, "--ungrouped cannot be combined with --group-id");
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

test("group-update supports --window active", async () => {
  const snapshot = {
    windows: [
      {
        windowId: 1,
        focused: true,
        tabs: [
          { tabId: 1, windowId: 1, index: 0, groupId: 10, groupTitle: "Work", title: "A", url: "https://a" },
        ],
        groups: [
          { groupId: 10, title: "Work" },
        ],
      },
    ],
  };

  const { socketPath, server, requests, sockets } = await startMockSocket((req) => {
    if (req.action === "list") {
      return { ok: true, action: req.action, requestId: req.id, data: snapshot };
    }
    return { ok: true, action: req.action, requestId: req.id, data: { groups: [] } };
  });

  const result = await runCli([
    "group-update",
    "--group",
    "Work",
    "--window",
    "active",
    "--title",
    "Work Items",
  ], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const params = requests.find((req) => req.action === "group-update")?.params as { windowId?: string } | undefined;
  assert.equal(params?.windowId, "active");
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

test("dedupe supports --ungrouped", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { candidates: [], totals: { tabs: 0, candidates: 0 } },
  }));

  const result = await runCli(["dedupe", "--ungrouped"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "analyze");
  const params = requests[0].params as { groupId?: number } | undefined;
  assert.equal(params?.groupId, -1);
});

test("dedupe rejects --ungrouped with --group-id", async () => {
  const result = await runCli(["dedupe", "--ungrouped", "--group-id", "3"]);
  assert.equal(result.status, 1);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, false);
  assert.equal(output.error.message, "--ungrouped cannot be combined with --group-id");
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
  ], undefined, { HOME: homeDir, XDG_STATE_HOME: path.join(homeDir, ".local", "state"), XDG_CONFIG_HOME: path.join(homeDir, ".config") });

  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim()) as { ok: boolean; action?: string; data: Record<string, unknown> };
  assert.equal(output.ok, true);
  assert.equal(output.action, "setup");
  assert.equal(output.data.profileName, "edge");

  const wrapperPath = path.join(homeDir, ".local", "state", "tabctl", "profiles", "edge", "tabctl-host.sh");
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

  // Manifest uses standard host name
  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8")) as { name?: string; path?: string; allowed_origins?: string[] };
  assert.equal(manifest.name, "com.erwinkroon.tabctl");
  assert.equal(manifest.path, wrapperPath);
  assert.deepEqual(manifest.allowed_origins, [`chrome-extension://${extensionId}/`]);

  const wrapper = fs.readFileSync(wrapperPath, "utf8");
  const hostPath = path.resolve(__dirname, "../../host/host.js");
  assert.ok(wrapper.includes(nodePath));
  assert.ok(wrapper.includes(hostPath));
  assert.ok(wrapper.includes('export TABCTL_PROFILE="edge"'));

  // Profile registered
  assert.equal(output.data.isDefault, true);
  const profilesPath = path.join(homeDir, ".config", "tabctl", "profiles.json");
  assert.ok(fs.existsSync(profilesPath));
  const profiles = JSON.parse(fs.readFileSync(profilesPath, "utf8"));
  assert.equal(profiles.default, "edge");
  assert.ok(profiles.profiles.edge);
});

test("setup writes native host manifest for chrome", async () => {
  if (process.platform !== "darwin") {
    return;
  }
  const homeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-setup-chrome-"));
  const extensionId = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
  const nodePath = process.execPath;
  const result = await runCli([
    "setup",
    "--browser",
    "chrome",
    "--extension-id",
    extensionId,
    "--node",
    nodePath,
  ], undefined, { HOME: homeDir, XDG_STATE_HOME: path.join(homeDir, ".local", "state"), XDG_CONFIG_HOME: path.join(homeDir, ".config") });

  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim()) as { ok: boolean; action?: string; data: Record<string, unknown> };
  assert.equal(output.ok, true);
  assert.equal(output.action, "setup");
  assert.equal(output.data.profileName, "chrome");

  const wrapperPath = path.join(homeDir, ".local", "state", "tabctl", "profiles", "chrome", "tabctl-host.sh");
  const manifestPath = path.join(
    homeDir,
    "Library",
    "Application Support",
    "Google",
    "Chrome",
    "NativeMessagingHosts",
    "com.erwinkroon.tabctl.json",
  );
  assert.equal(output.data.wrapperPath, wrapperPath);
  assert.equal(output.data.manifestPath, manifestPath);
  assert.ok(fs.existsSync(wrapperPath));
  assert.ok(fs.existsSync(manifestPath));

  // Manifest uses standard host name
  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8")) as { name?: string; path?: string; allowed_origins?: string[] };
  assert.equal(manifest.name, "com.erwinkroon.tabctl");
  assert.equal(manifest.path, wrapperPath);
  assert.deepEqual(manifest.allowed_origins, [`chrome-extension://${extensionId}/`]);

  const wrapper = fs.readFileSync(wrapperPath, "utf8");
  const hostPath = path.resolve(__dirname, "../../host/host.js");
  assert.ok(wrapper.includes(nodePath));
  assert.ok(wrapper.includes(hostPath));
  assert.ok(wrapper.includes('export TABCTL_PROFILE="chrome"'));

  // Profile registered with browser: "chrome"
  assert.equal(output.data.isDefault, true);
  const profilesPath = path.join(homeDir, ".config", "tabctl", "profiles.json");
  assert.ok(fs.existsSync(profilesPath));
  const profiles = JSON.parse(fs.readFileSync(profilesPath, "utf8"));
  assert.equal(profiles.default, "chrome");
  assert.ok(profiles.profiles.chrome);
  assert.equal(profiles.profiles.chrome.browser, "chrome");
});

test("setup --user-data-dir writes manifest to custom path", async () => {
  if (process.platform !== "darwin") {
    return;
  }
  const homeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-setup-udd-"));
  const userDataDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-udd-chrome-"));
  const extensionId = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
  const nodePath = process.execPath;
  const result = await runCli([
    "setup",
    "--browser",
    "chrome",
    "--extension-id",
    extensionId,
    "--node",
    nodePath,
    "--user-data-dir",
    userDataDir,
  ], undefined, { HOME: homeDir, XDG_STATE_HOME: path.join(homeDir, ".local", "state"), XDG_CONFIG_HOME: path.join(homeDir, ".config") });

  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim()) as { ok: boolean; action?: string; data: Record<string, unknown> };
  assert.equal(output.ok, true);
  assert.equal(output.action, "setup");
  assert.equal(output.data.profileName, "chrome");

  // Manifest written to userDataDir, NOT the system-wide path
  const manifestPath = path.join(userDataDir, "NativeMessagingHosts", "com.erwinkroon.tabctl.json");
  const systemManifestPath = path.join(
    homeDir,
    "Library",
    "Application Support",
    "Google",
    "Chrome",
    "NativeMessagingHosts",
    "com.erwinkroon.tabctl.json",
  );
  assert.equal(output.data.manifestPath, manifestPath);
  assert.ok(fs.existsSync(manifestPath), "manifest should exist in userDataDir");
  assert.ok(!fs.existsSync(systemManifestPath), "manifest should NOT exist in system-wide path");

  // Manifest has correct allowed_origins
  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8")) as { name?: string; path?: string; allowed_origins?: string[] };
  assert.equal(manifest.name, "com.erwinkroon.tabctl");
  assert.deepEqual(manifest.allowed_origins, [`chrome-extension://${extensionId}/`]);

  // Profile registered with browser "chrome"
  const profilesPath = path.join(homeDir, ".config", "tabctl", "profiles.json");
  assert.ok(fs.existsSync(profilesPath));
  const profiles = JSON.parse(fs.readFileSync(profilesPath, "utf8"));
  assert.ok(profiles.profiles.chrome);
  assert.equal(profiles.profiles.chrome.browser, "chrome");

  // Output JSON includes userDataDir
  assert.equal(output.data.userDataDir, userDataDir);
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
  assert.match(result.stdout, /Command Details/);
  assert.match(result.stdout, /Option Groups/);
});

test("help supports json output", async () => {
  const result = await runCli(["help", "--json"]);
  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
  assert.ok(output.data?.commands);
  assertVersion(output.data?.version as string | undefined);
  const optionGroups = output.data?.optionGroups as Array<{ name: string; options: string[] }> | undefined;
  assert.ok(optionGroups);
  const scopeGroup = optionGroups?.find((group) => group?.name === "Scope Options");
  const paginationGroup = optionGroups?.find((group) => group?.name === "Pagination Options");
  assert.ok(scopeGroup?.options?.includes("--tab <id> (repeatable)"));
  assert.ok(scopeGroup?.options?.includes("--group <name>"));
  assert.ok(scopeGroup?.options?.includes("--group-id <id>"));
  assert.ok(scopeGroup?.options?.includes("--ungrouped"));
  assert.ok(scopeGroup?.options?.includes("--window <id|active|last-focused>"));
  assert.ok(scopeGroup?.options?.includes("--all"));
  assert.ok(paginationGroup?.options?.includes("--limit <n>"));
  assert.ok(paginationGroup?.options?.includes("--offset <n>"));
  assert.ok(paginationGroup?.options?.includes("--no-page"));
  const analyze = output.data?.commands?.find((cmd: { name: string; options?: string[] }) => cmd?.name === "analyze");
  assert.ok(analyze?.options?.includes("--window-title"));
  const list = output.data?.commands?.find((cmd: { name: string; options?: string[] }) => cmd?.name === "list");
  assert.ok(list?.options?.includes("--groups"));
  const open = output.data?.commands?.find((cmd: { name: string; options?: string[] }) => cmd?.name === "open");
  assert.ok(open?.options?.includes("--url <url> (repeatable)"));
  assert.ok(open?.options?.includes("--before-tab <id>"));
  assert.ok(open?.options?.includes("--after-tab <id>"));
  assert.ok(open?.options?.includes("--new-window"));
  const report = output.data?.commands?.find((cmd: { name: string; options?: string[] }) => cmd?.name === "report");
  assert.ok(report?.options?.includes("--format json|md|csv"));
  const close = output.data?.commands?.find((cmd: { name: string; options?: string[] }) => cmd?.name === "close");
  assert.ok(close?.options?.includes("--dry-run"));
  const setup = output.data?.commands?.find((cmd: { name: string; options?: string[] }) => cmd?.name === "setup");
  assert.ok(setup?.options?.includes("--browser edge|chrome"));
});

test("command-specific help filters output", async () => {
  const result = await runCli(["help", "open", "--json"]);
  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
  assert.equal(output.data?.commands?.length, 1);
  assert.equal(output.data?.commands?.[0]?.name, "open");
});

test("version outputs cli version", async () => {
  const result = await runCli(["version"]);
  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
  assertVersion(output.data?.version as string | undefined);
  assert.equal(output.data?.component, "cli");
  assert.equal(output.data?.baseVersion, pkgVersion);
});

test("skill install creates project skill link", async () => {
  const testRoot = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-skill-"));
  const originalCwd = process.cwd();
  const installRoot = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-install-"));
  const repoRoot = path.resolve(__dirname, "../..");
  const fakeBin = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-fakebin-"));
  const npxFixture = path.join(__dirname, "fixtures", "npx");
  const fakeNpx = path.join(fakeBin, "npx");
  fs.copyFileSync(npxFixture, fakeNpx);
  fs.chmodSync(fakeNpx, 0o755);
  const npxCapture = path.join(testRoot, "npx-args.json");
  fs.cpSync(path.join(repoRoot, "cli"), path.join(installRoot, "cli"), { recursive: true });
  fs.cpSync(path.join(repoRoot, "shared"), path.join(installRoot, "shared"), { recursive: true });
  const cliTarget = path.join(installRoot, "cli", "tabctl.js");
  process.chdir(testRoot);
  try {
    const result = await runCli([
      "skill",
    ], undefined, {
      XDG_CONFIG_HOME: path.join(testRoot, ".config"),
      NPX_CAPTURE_PATH: npxCapture,
    }, cliTarget, fakeNpx);
    assert.equal(result.status, 0);
    const output = JSON.parse(result.stdout.trim());
    assert.equal(output.ok, true);
    const targetDir = output.data?.targetDir as string;
    assert.ok(targetDir);
    assert.equal(output.data?.scope, "project");
    assert.ok(targetDir.includes(path.join(testRoot, ".opencode", "skills", "tabctl")));
    const captured = JSON.parse(fs.readFileSync(npxCapture, "utf8"));
    assert.deepEqual(captured.args, [
      "skills",
      "add",
      "https://github.com/ekroon/tabctl",
      "--skill",
      "tabctl",
    ]);
  } finally {
    process.chdir(originalCwd);
  }
});

test("skill install supports global scope", async () => {
  const testRoot = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-skill-global-"));
  const configHome = path.join(testRoot, "config");
  const originalCwd = process.cwd();
  const installRoot = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-install-"));
  const repoRoot = path.resolve(__dirname, "../..");
  const fakeBin = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-fakebin-"));
  const npxFixture = path.join(__dirname, "fixtures", "npx");
  const fakeNpx = path.join(fakeBin, "npx");
  fs.copyFileSync(npxFixture, fakeNpx);
  fs.chmodSync(fakeNpx, 0o755);
  const npxCapture = path.join(testRoot, "npx-args.json");
  fs.cpSync(path.join(repoRoot, "cli"), path.join(installRoot, "cli"), { recursive: true });
  fs.cpSync(path.join(repoRoot, "shared"), path.join(installRoot, "shared"), { recursive: true });
  const cliTarget = path.join(installRoot, "cli", "tabctl.js");
  process.chdir(testRoot);

  try {
    const result = await runCli(["skill", "--global"], undefined, {
      XDG_CONFIG_HOME: configHome,
      NPX_CAPTURE_PATH: npxCapture,
    }, cliTarget, fakeNpx);
    assert.equal(result.status, 0);
    const output = JSON.parse(result.stdout.trim());
    const targetDir = output.data?.targetDir as string;
    assert.ok(targetDir);
    assert.equal(output.data?.scope, "global");
    assert.ok(targetDir.includes(path.join(configHome, "opencode", "skills", "tabctl")));
    const captured = JSON.parse(fs.readFileSync(npxCapture, "utf8"));
    assert.deepEqual(captured.args, [
      "skills",
      "add",
      "https://github.com/ekroon/tabctl",
      "--skill",
      "tabctl",
      "-g",
    ]);
  } finally {
    process.chdir(originalCwd);
  }
});


test("version includes dev sha when built", async () => {
  const result = await runCli(["version"]);
  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
  const version = output.data?.version as string | undefined;
  assert.ok(version);
  if (version && version.includes("-dev.")) {
    const re = new RegExp(`^${pkgVersion.replace(/\./g, "\\.")}-dev\\.[0-9a-f]{8}(\\.dirty)?$`);
    assert.match(version, re);
  } else {
    assert.equal(version, pkgVersion);
  }
  assert.equal(output.data?.baseVersion, pkgVersion);
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

test("undo supports --txid", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { summary: { restoredTabs: 1 } },
  }));

  const result = await runCli(["undo", "--txid", "tx-555"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const params = requests[0].params as { txid?: string } | undefined;
  assert.equal(params?.txid, "tx-555");
});

test("undo supports --latest", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    data: { summary: { restoredTabs: 1 } },
  }));

  const result = await runCli(["undo", "--latest"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const params = requests[0].params as { latest?: boolean } | undefined;
  assert.equal(params?.latest, true);
});

test("undo rejects --latest with txid", async () => {
  const result = await runCli(["undo", "--latest", "tx-123"]);
  assert.equal(result.status, 1);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, false);
  assert.match(output.error.message, /undo --latest/);
});

test("undo rejects --latest with --txid", async () => {
  const result = await runCli(["undo", "--latest", "--txid", "tx-123"]);
  assert.equal(result.status, 1);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, false);
  assert.match(output.error.message, /undo --latest/);
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

// --- Profile integration tests ---

test("profile-list shows empty when no profiles", async () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-profile-test-"));
  const result = await runCli(["profile-list", "--json"], undefined, {
    TABCTL_CONFIG_DIR: tmpDir,
  });
  fs.rmSync(tmpDir, { recursive: true, force: true });

  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
  assert.equal(output.action, "profile-list");
  assert.deepEqual(output.data.profiles, []);
});

test("profile-show shows legacy mode when no profiles", async () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-profile-test-"));
  const result = await runCli(["profile-show", "--json"], undefined, {
    TABCTL_CONFIG_DIR: tmpDir,
  });
  fs.rmSync(tmpDir, { recursive: true, force: true });

  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
  assert.equal(output.action, "profile-show");
  assert.equal(output.data.mode, "legacy");
});

test("profile-switch fails for unknown profile", async () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-profile-test-"));
  const result = await runCli(["profile-switch", "nonexistent"], undefined, {
    TABCTL_CONFIG_DIR: tmpDir,
  });
  fs.rmSync(tmpDir, { recursive: true, force: true });

  assert.notEqual(result.status, 0);
});

test("profile-show with configured profile shows profile name", async () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-profile-test-"));
  const profiles = {
    default: "test-profile",
    profiles: {
      "test-profile": {
        browser: "edge",
        extensionId: "test-ext-id",
        nodePath: process.execPath,
        hostPath: "/tmp/fake-host.js",
        dataDir: path.join(tmpDir, "data"),
      },
    },
  };
  fs.writeFileSync(path.join(tmpDir, "profiles.json"), JSON.stringify(profiles, null, 2));

  const result = await runCli(["profile-show", "--json"], undefined, {
    TABCTL_CONFIG_DIR: tmpDir,
  });
  fs.rmSync(tmpDir, { recursive: true, force: true });

  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
  assert.equal(output.action, "profile-show");
  assert.equal(output.data.name, "test-profile");
  assert.equal(output.data.browser, "edge");
});

// --- Additional profile integration tests ---

function makeTwoProfileConfig(tmpDir: string) {
  const profiles = {
    default: "edge",
    profiles: {
      edge: {
        browser: "edge",
        extensionId: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        nodePath: "/usr/bin/node",
        hostPath: "/tmp/fake-host.js",
        dataDir: path.join(tmpDir, "data-edge"),
      },
      chrome: {
        browser: "chrome",
        extensionId: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        nodePath: "/usr/bin/node",
        hostPath: "/tmp/fake-host.js",
        dataDir: path.join(tmpDir, "data-chrome"),
      },
    },
  };
  fs.writeFileSync(path.join(tmpDir, "profiles.json"), JSON.stringify(profiles, null, 2));
  return profiles;
}

test("profile-switch success updates default", async () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-profile-test-"));
  try {
    makeTwoProfileConfig(tmpDir);
    const result = await runCli(["profile-switch", "chrome"], undefined, {
      TABCTL_CONFIG_DIR: tmpDir,
    });

    assert.equal(result.status, 0);
    const output = JSON.parse(result.stdout.trim());
    assert.equal(output.ok, true);
    assert.equal(output.action, "profile-switch");
    assert.equal(output.data.name, "chrome");

    const updated = JSON.parse(fs.readFileSync(path.join(tmpDir, "profiles.json"), "utf8"));
    assert.equal(updated.default, "chrome");
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

test("profile-remove success removes profile", async () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-profile-test-"));
  try {
    makeTwoProfileConfig(tmpDir);
    const result = await runCli(["profile-remove", "chrome"], undefined, {
      TABCTL_CONFIG_DIR: tmpDir,
    });

    assert.equal(result.status, 0);
    const output = JSON.parse(result.stdout.trim());
    assert.equal(output.ok, true);
    assert.equal(output.action, "profile-remove");

    const updated = JSON.parse(fs.readFileSync(path.join(tmpDir, "profiles.json"), "utf8"));
    assert.equal(updated.default, "edge");
    assert.ok(updated.profiles.edge);
    assert.equal(updated.profiles.chrome, undefined);
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

test("--profile flag overrides active profile", async () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-profile-test-"));
  try {
    makeTwoProfileConfig(tmpDir);
    const result = await runCli(["profile-show", "--profile", "chrome", "--json"], undefined, {
      TABCTL_CONFIG_DIR: tmpDir,
    });

    assert.equal(result.status, 0);
    const output = JSON.parse(result.stdout.trim());
    assert.equal(output.data.name, "chrome");
    assert.equal(output.data.browser, "chrome");
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

test("TABCTL_PROFILE env overrides active profile", async () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-profile-test-"));
  try {
    makeTwoProfileConfig(tmpDir);
    const result = await runCli(["profile-show", "--json"], undefined, {
      TABCTL_CONFIG_DIR: tmpDir,
      TABCTL_PROFILE: "chrome",
    });

    assert.equal(result.status, 0);
    const output = JSON.parse(result.stdout.trim());
    assert.equal(output.data.name, "chrome");
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

test("setup --name creates custom-named profile", async () => {
  if (process.platform !== "darwin") {
    return;
  }
  const homeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-setup-name-"));
  try {
    const extensionId = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const result = await runCli([
      "setup",
      "--browser", "edge",
      "--extension-id", extensionId,
      "--name", "my-edge",
      "--node", process.execPath,
    ], undefined, {
      HOME: homeDir,
      XDG_STATE_HOME: path.join(homeDir, ".local", "state"),
      XDG_CONFIG_HOME: path.join(homeDir, ".config"),
    });

    assert.equal(result.status, 0);
    const output = JSON.parse(result.stdout.trim());
    assert.equal(output.ok, true);
    assert.equal(output.data.profileName, "my-edge");

    const profilesPath = path.join(homeDir, ".config", "tabctl", "profiles.json");
    const profiles = JSON.parse(fs.readFileSync(profilesPath, "utf8"));
    assert.ok(profiles.profiles["my-edge"]);
  } finally {
    fs.rmSync(homeDir, { recursive: true, force: true });
  }
});

test("output includes profile and browser fields", async () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-profile-test-"));
  try {
    const profiles = {
      default: "edge",
      profiles: {
        edge: {
          browser: "edge",
          extensionId: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
          nodePath: "/usr/bin/node",
          hostPath: "/tmp/fake-host.js",
          dataDir: path.join(tmpDir, "data"),
        },
      },
    };
    fs.writeFileSync(path.join(tmpDir, "profiles.json"), JSON.stringify(profiles, null, 2));

    const result = await runCli(["profile-list", "--json"], undefined, {
      TABCTL_CONFIG_DIR: tmpDir,
    });

    assert.equal(result.status, 0);
    const output = JSON.parse(result.stdout.trim());
    assert.equal(output.profile, "edge");
    assert.equal(output.browser, "edge");
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

test("profile-list with multiple profiles shows all", async () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-profile-test-"));
  try {
    const profiles = {
      default: "edge",
      profiles: {
        edge: {
          browser: "edge",
          extensionId: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
          nodePath: "/usr/bin/node",
          hostPath: "/tmp/fake-host.js",
          dataDir: path.join(tmpDir, "data-edge"),
        },
        chrome: {
          browser: "chrome",
          extensionId: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
          nodePath: "/usr/bin/node",
          hostPath: "/tmp/fake-host.js",
          dataDir: path.join(tmpDir, "data-chrome"),
        },
        "chrome-work": {
          browser: "chrome",
          extensionId: "cccccccccccccccccccccccccccccccc",
          nodePath: "/usr/bin/node",
          hostPath: "/tmp/fake-host.js",
          dataDir: path.join(tmpDir, "data-chrome-work"),
        },
      },
    };
    fs.writeFileSync(path.join(tmpDir, "profiles.json"), JSON.stringify(profiles, null, 2));

    const result = await runCli(["profile-list", "--json"], undefined, {
      TABCTL_CONFIG_DIR: tmpDir,
    });

    assert.equal(result.status, 0);
    const output = JSON.parse(result.stdout.trim());
    assert.equal(output.data.profiles.length, 3);

    const defaultProfile = output.data.profiles.find((p: { name: string }) => p.name === "edge");
    assert.ok(defaultProfile);
    assert.equal(defaultProfile.isDefault, true);

    const nonDefaults = output.data.profiles.filter((p: { isDefault: boolean }) => !p.isDefault);
    assert.equal(nonDefaults.length, 2);
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

test("profile-show isDefault is false when using --profile override", async () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-profile-test-"));
  try {
    makeTwoProfileConfig(tmpDir);
    const result = await runCli(["profile-show", "--profile", "chrome", "--json"], undefined, {
      TABCTL_CONFIG_DIR: tmpDir,
    });

    assert.equal(result.status, 0);
    const output = JSON.parse(result.stdout.trim());
    assert.equal(output.data.name, "chrome");
    assert.equal(output.data.isDefault, false);
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

// Non-interactive setup mode is covered by "setup writes native host manifest" and
// "setup writes native host manifest for chrome" tests above (both pass --extension-id).

test("setup interactive mode reads extension-id from stdin", async () => {
  if (process.platform !== "darwin") {
    return;
  }
  const homeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-setup-interactive-"));
  try {
    const extensionId = "cccccccccccccccccccccccccccccccc";
    const envOverrides = {
      HOME: homeDir,
      XDG_STATE_HOME: path.join(homeDir, ".local", "state"),
      XDG_CONFIG_HOME: path.join(homeDir, ".config"),
    };
    const result = await runCliWithStdin(
      ["setup", "--browser", "chrome", "--node", process.execPath],
      `${extensionId}\n`,
      envOverrides,
    );

    assert.equal(result.status, 0, `expected exit 0, stderr: ${result.stderr}`);
    const output = JSON.parse(result.stdout.trim()) as { ok: boolean; action?: string; data: Record<string, unknown> };
    assert.equal(output.ok, true);
    assert.equal(output.action, "setup");
    assert.equal(output.data.extensionId, extensionId);
    assert.equal(output.data.profileName, "chrome");

    // Manifest written with correct allowed_origins
    const manifestPath = output.data.manifestPath as string;
    assert.ok(fs.existsSync(manifestPath));
    const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8")) as { allowed_origins?: string[] };
    assert.deepEqual(manifest.allowed_origins, [`chrome-extension://${extensionId}/`]);

    // Profile registered
    const profilesPath = path.join(homeDir, ".config", "tabctl", "profiles.json");
    assert.ok(fs.existsSync(profilesPath));
    const profiles = JSON.parse(fs.readFileSync(profilesPath, "utf8"));
    assert.ok(profiles.profiles.chrome);
    assert.equal(profiles.profiles.chrome.extensionId, extensionId);
  } finally {
    fs.rmSync(homeDir, { recursive: true, force: true });
  }
});

test("setup interactive mode rejects invalid ids and accepts valid on retry", async () => {
  if (process.platform !== "darwin") {
    return;
  }
  const homeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-setup-retry-"));
  try {
    const validId = "dddddddddddddddddddddddddddddddd".slice(0, 32);
    const envOverrides = {
      HOME: homeDir,
      XDG_STATE_HOME: path.join(homeDir, ".local", "state"),
      XDG_CONFIG_HOME: path.join(homeDir, ".config"),
    };
    // First two inputs are invalid, third is valid
    const stdinData = "bad\nXYZ\n" + validId + "\n";
    const result = await runCliWithStdin(
      ["setup", "--browser", "chrome", "--node", process.execPath],
      stdinData,
      envOverrides,
    );

    assert.equal(result.status, 0, `expected exit 0, stderr: ${result.stderr}`);
    const output = JSON.parse(result.stdout.trim()) as { ok: boolean; data: Record<string, unknown> };
    assert.equal(output.ok, true);
    assert.equal(output.data.extensionId, validId);
    // Stderr should mention invalid attempts
    assert.ok(result.stderr.includes("Invalid extension ID"), "expected retry message on stderr");
  } finally {
    fs.rmSync(homeDir, { recursive: true, force: true });
  }
});

test("setup interactive mode fails after 3 invalid attempts", async () => {
  if (process.platform !== "darwin") {
    return;
  }
  const homeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-setup-fail-"));
  try {
    const envOverrides = {
      HOME: homeDir,
      XDG_STATE_HOME: path.join(homeDir, ".local", "state"),
      XDG_CONFIG_HOME: path.join(homeDir, ".config"),
    };
    // All three inputs are invalid
    const stdinData = "bad1\nbad2\nbad3\n";
    const result = await runCliWithStdin(
      ["setup", "--browser", "chrome", "--node", process.execPath],
      stdinData,
      envOverrides,
    );

    assert.notEqual(result.status, 0, "expected non-zero exit after 3 invalid attempts");
    assert.ok(
      result.stderr.includes("3 attempts") || result.stdout.includes("3 attempts"),
      "expected error message about 3 attempts",
    );
  } finally {
    fs.rmSync(homeDir, { recursive: true, force: true });
  }
});

test("setup second profile does not nest under first profile dataDir", async () => {
  if (process.platform !== "darwin") {
    return;
  }
  const homeDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-setup-nest-"));
  try {
    const envOverrides = {
      HOME: homeDir,
      XDG_STATE_HOME: path.join(homeDir, ".local", "state"),
      XDG_CONFIG_HOME: path.join(homeDir, ".config"),
    };
    const baseStateDir = path.join(homeDir, ".local", "state", "tabctl");

    // First setup creates "edge" as default
    await runCli([
      "setup", "--browser", "edge",
      "--extension-id", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "--node", process.execPath,
    ], undefined, envOverrides);

    // Second setup creates "chrome"
    const result = await runCli([
      "setup", "--browser", "chrome",
      "--extension-id", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
      "--node", process.execPath,
    ], undefined, envOverrides);

    assert.equal(result.status, 0);
    const output = JSON.parse(result.stdout.trim());
    const chromeDataDir = output.data.dataDir as string;

    // Must be baseStateDir/profiles/chrome, NOT nested under edge's dataDir
    assert.equal(chromeDataDir, path.join(baseStateDir, "profiles", "chrome"));
    assert.ok(fs.existsSync(chromeDataDir));
  } finally {
    fs.rmSync(homeDir, { recursive: true, force: true });
  }
});
