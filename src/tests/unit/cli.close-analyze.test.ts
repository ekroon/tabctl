import assert from "node:assert/strict";
import fs from "fs";
import os from "os";
import path from "path";
import test from "node:test";
import { startMockSocket, stopMockSocket } from "./socket";
import { runCli } from "./cli-helpers";

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
