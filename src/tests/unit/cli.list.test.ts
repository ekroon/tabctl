import assert from "node:assert/strict";
import test from "node:test";
import { startMockSocket, stopMockSocket } from "./socket";
import { runCli, parseOutput, mockResponse } from "./cli-helpers";

test("list sends list action", async () => {
  const { socketPath, server, sockets, requests } = await startMockSocket((req) => (mockResponse(req, { value: "ok" })));

  const result = await runCli(["list"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const output = parseOutput(result);
  assert.equal(output.ok, true);
  assert.equal(requests.length, 1);
  assert.equal(requests[0].action, "list");
});

test("ping sends ping action", async () => {
  const { socketPath, server, sockets, requests } = await startMockSocket((req) => (mockResponse(req, { value: "pong" })));

  const result = await runCli(["ping"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const output = parseOutput(result);
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
      return mockResponse(req, snapshot);
    }
    return mockResponse(req);
  });

  const result = await runCli(["list", "--group", "Work", "--limit", "1"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "list");
  const output = parseOutput(result);
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
      return mockResponse(req, snapshot);
    }
    return mockResponse(req);
  });

  const result = await runCli(["list", "--group", "Work", "--all", "--limit", "100"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const output = parseOutput(result);
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
      return mockResponse(req, snapshot);
    }
    return mockResponse(req);
  });

  const result = await runCli(["list", "--ungrouped", "--limit", "10"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const output = parseOutput(result);
  const data = output.data as { windows?: Array<Record<string, unknown>> };
  const tabs = ((data.windows?.[0].tabs as Array<Record<string, unknown>>) || []);
  assert.equal(tabs.length, 1);
  assert.equal(tabs[0].tabId, 1);
});

test("list rejects invalid window value", async () => {
  const result = await runCli(["list", "--window", "nope"]);
  assert.equal(result.status, 1);
  const output = parseOutput(result);
  assert.equal(output.ok, false);
  assert.match(output.error.message, /Invalid --window value/);
});

test("unknown --format hints to use --json", async () => {
  const result = await runCli(["list", "--format", "json"]);
  assert.equal(result.status, 1);
  const output = parseOutput(result);
  assert.equal(output.ok, false);
  assert.match(output.error.message, /Unknown option: --format/);
  assert.match(output.error.hint, /Use --json/);
});

test("focus passes tab id", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => (mockResponse(req, { tabId: 99, windowId: 1 })));

  const result = await runCli(["focus", "--tab", "99"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "focus");
  const params = requests[0].params as { tabId?: number } | undefined;
  assert.equal(params?.tabId, 99);
});

test("refresh passes tab id", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => (mockResponse(req, { summary: { refreshedTabs: 1 } })));

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
  const output = parseOutput(result);
  assert.equal(output.ok, false);
  assert.equal(output.error.message, "refresh requires --tab");
});

test("refresh rejects multiple --tab", async () => {
  const result = await runCli(["refresh", "--tab", "1", "--tab", "2"]);
  assert.equal(result.status, 1);
  const output = parseOutput(result);
  assert.equal(output.ok, false);
  assert.equal(output.error.message, "refresh requires a single --tab");
});

test("open passes urls and window selectors", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => (mockResponse(req, { summary: { createdTabs: 2 } })));

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

test("open supports new window flag", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => (mockResponse(req, { summary: { createdTabs: 1 } })));

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
  const output = parseOutput(result);
  assert.equal(output.ok, false);
  assert.match(output.error.message, /Only one target position is allowed/);
});

test("open supports after-tab", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => (mockResponse(req, { summary: { createdTabs: 1 } })));

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
  const output = parseOutput(result);
  assert.equal(output.ok, false);
  assert.equal(output.error?.message, "Invalid color: chartreuse. Use one of: grey, blue, red, yellow, green, pink, purple, cyan, orange");
});

test("open supports window new alias", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => (mockResponse(req, { summary: { createdTabs: 1 } })));

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

test("undo sends undo action with txid", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => (mockResponse(req, { summary: { restoredTabs: 1 } })));

  const result = await runCli(["undo", "tx-123"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests.length, 1);
  assert.equal(requests[0].action, "undo");
  const params = requests[0].params as { txid?: string } | undefined;
  assert.equal(params?.txid, "tx-123");
});

test("undo supports --txid", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => (mockResponse(req, { summary: { restoredTabs: 1 } })));

  const result = await runCli(["undo", "--txid", "tx-555"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const params = requests[0].params as { txid?: string } | undefined;
  assert.equal(params?.txid, "tx-555");
});

test("undo supports --latest", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => (mockResponse(req, { summary: { restoredTabs: 1 } })));

  const result = await runCli(["undo", "--latest"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const params = requests[0].params as { latest?: boolean } | undefined;
  assert.equal(params?.latest, true);
});

test("undo rejects --latest with txid", async () => {
  const result = await runCli(["undo", "--latest", "tx-123"]);
  assert.equal(result.status, 1);
  const output = parseOutput(result);
  assert.equal(output.ok, false);
  assert.match(output.error.message, /undo --latest/);
});

test("undo rejects --latest with --txid", async () => {
  const result = await runCli(["undo", "--latest", "--txid", "tx-123"]);
  assert.equal(result.status, 1);
  const output = parseOutput(result);
  assert.equal(output.ok, false);
  assert.match(output.error.message, /undo --latest/);
});

test("ENOENT error includes native host hint", async () => {
  const result = await runCli(["ping"], "/tmp/tabctl-nonexistent-socket-path.sock");
  assert.equal(result.status, 1);
  const output = parseOutput(result);
  assert.equal(output.ok, false);
  assert.ok(output.error.message.includes("ENOENT"), "error message should contain ENOENT");
  assert.equal(output.error.hint, "Native host not running. Ensure the browser extension is loaded and active. If you recently upgraded, run: tabctl setup");
});

test("reload sends reload action", async () => {
  const { socketPath, server, sockets, requests } = await startMockSocket((req) => (mockResponse(req, { reloading: true })));

  const result = await runCli(["reload", "--json"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const output = parseOutput(result);
  assert.equal(output.ok, true);
  assert.equal(requests.length, 1);
  assert.equal(requests[0].action, "reload");
  assert.equal(output.data.reloading, true);
});

test("version mismatch triggers auto-upgrade message on stderr", async () => {
  // Mock host responds with a different version than the CLI
  const { socketPath, server, sockets } = await startMockSocket((req) => ({
    ok: true,
    action: req.action,
    requestId: req.id,
    version: "0.0.1",
    data: { extensionVersion: "0.0.1", extensionComponent: "extension" },
  }));

  const result = await runCli(["ping", "--json"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  // Should contain upgrade or stale message on stderr
  assert.ok(
    result.stderr.includes("upgraded:") || result.stderr.includes("stale"),
    `expected upgrade or stale message on stderr, got: ${result.stderr}`,
  );
  // Should also trigger reload
  assert.ok(
    result.stderr.includes("Reloading") || result.stderr.includes("reloading"),
    `expected reloading message on stderr, got: ${result.stderr}`,
  );
});
