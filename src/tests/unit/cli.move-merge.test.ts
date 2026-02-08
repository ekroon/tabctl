import assert from "node:assert/strict";
import test from "node:test";
import { startMockSocket, stopMockSocket } from "./socket";
import { runCli, parseOutput, mockResponse } from "./cli-helpers";

test("move-tab supports new window flag", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => (mockResponse(req, { summary: { movedTabs: 1 } })));

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
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => (mockResponse(req, { summary: { movedTabs: 2 } })));

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
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => (mockResponse(req, { summary: { movedTabs: 3 } })));

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
      return mockResponse(req, {
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
        });
    }
    return mockResponse(req, { summary: { closedTabs: 1, skippedTabs: 0 } });
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
  const output = parseOutput(result);
  assert.equal(output.action, "dedupe");
  assert.equal(output.data?.summary?.closed, 1);
});

test("dedupe outputs nextCommand when not confirmed", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => {
    if (req.action === "analyze") {
      return mockResponse(req, {
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
        });
    }
    return mockResponse(req, { summary: { closedTabs: 1, skippedTabs: 0 } });
  });

  const result = await runCli(["dedupe"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests.length, 1);
  assert.equal(requests[0].action, "analyze");
  const output = parseOutput(result);
  assert.equal(output.action, "dedupe");
  assert.equal(output.data?.nextCommand, "tabctl close --apply analysis-1 --confirm");
});

test("dedupe supports --ungrouped", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => (mockResponse(req, { candidates: [], totals: { tabs: 0, candidates: 0 } })));

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
  const output = parseOutput(result);
  assert.equal(output.ok, false);
  assert.equal(output.error.message, "--ungrouped cannot be combined with --group-id");
});

test("move-tab passes target options", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => (mockResponse(req, { summary: { movedTabs: 1 } })));

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
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => (mockResponse(req, { summary: { movedTabs: 2 } })));

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
  const output = parseOutput(result);
  assert.equal(output.data?.txid, "tx-move-tab");
  assert.equal(output.version, "0.1.0");
  assert.equal(output.component, "host");
  assert.equal(output.data?.extensionVersion, "0.1.0");
  assert.equal(output.data?.hostBaseVersion, "0.1.0");
});
