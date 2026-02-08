import assert from "node:assert/strict";
import fs from "fs";
import os from "os";
import path from "path";
import test from "node:test";
import { startMockSocket, stopMockSocket } from "./socket";
import { runCli, parseOutput, mockResponse } from "./cli-helpers";

test("group-list passes window option", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => (mockResponse(req, { groups: [] })));

  const result = await runCli(["group-list", "--window", "3"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "group-list");
  const params = requests[0].params as { windowId?: string } | undefined;
  assert.equal(params?.windowId, "3");
});

test("group-list supports --window active", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => (mockResponse(req, { groups: [] })));

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
      return mockResponse(req, snapshot);
    }
    if (req.action === "group-list") {
      return mockResponse(req, {
          groups: [
            { windowId: 1, groupId: 10, title: "Work", tabCount: 1 },
            { windowId: 2, groupId: 11, title: "Home", tabCount: 1 },
          ],
        });
    }
    return mockResponse(req);
  });

  const result = await runCli(["group-list", "--tab", "1", "--limit", "1"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "group-list");
  assert.equal(requests.length, 2);
  assert.equal(requests[1].action, "list");
  const output = parseOutput(result);
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
      return mockResponse(req, snapshot);
    }
    if (req.action === "group-list") {
      return mockResponse(req, {
          groups: [
            { windowId: 1, groupId: 10, title: "Work", tabCount: 1 },
            { windowId: 1, groupId: 11, title: "Home", tabCount: 0 },
          ],
        });
    }
    return mockResponse(req);
  });

  const result = await runCli(["group-list", "--tab", "999", "--all", "--limit", "100"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "group-list");
  assert.equal(requests.length, 1);
  const output = parseOutput(result);
  const data = output.data as { groups?: Array<Record<string, unknown>> };
  assert.equal(data.groups?.length, 2);
});

test("group-list --all ignores window and group filters", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => {
    if (req.action === "group-list") {
      return mockResponse(req, {
          groups: [
            { windowId: 1, groupId: 10, title: "Work" },
            { windowId: 2, groupId: 20, title: "Home" },
          ],
        });
    }
    return mockResponse(req);
  });

  const result = await runCli(["group-list", "--all", "--window", "1", "--group", "Work"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "group-list");
  const params = requests[0].params as { windowId?: number } | undefined;
  assert.equal(params?.windowId, undefined);
  const output = parseOutput(result);
  const data = output.data as { groups?: Array<Record<string, unknown>> };
  assert.equal(data.groups?.length, 2);
});

test("group-list supports --ungrouped", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => {
    if (req.action === "group-list") {
      return mockResponse(req, {
          groups: [
            { windowId: 1, groupId: 10, title: "Work" },
            { windowId: 1, groupId: -1, title: "Ungrouped" },
          ],
        });
    }
    return mockResponse(req);
  });

  const result = await runCli(["group-list", "--ungrouped"], socketPath);
  await stopMockSocket(server, socketPath, sockets);
  assert.equal(result.status, 0);
  const output = parseOutput(result);
  assert.ok(output.ok);
  const groups = output.data?.groups as Array<{ groupId: number }> | undefined;
  assert.ok(groups);
  assert.equal(groups?.length, 1);
  assert.equal(groups?.[0]?.groupId, -1);
});

test("group-list falls back to snapshot when groups missing", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => {
    if (req.action === "group-list") {
      return mockResponse(req, { groups: null });
    }
    if (req.action === "list") {
      return mockResponse(req, {
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
        });
    }
    return mockResponse(req);
  });

  const result = await runCli(["group-list", "--window", "7"], socketPath);
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  assert.equal(requests[0].action, "group-list");
  assert.equal(requests.length, 2);
  const output = parseOutput(result);
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
      return mockResponse(req, { groups: null });
    }
    if (req.action === "list") {
      return mockResponse(req, {
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
        });
    }
    return mockResponse(req);
  });

  const result = await runCli(["group-list", "--window", "7"], socketPath, { XDG_CONFIG_HOME: policyDir });
  await stopMockSocket(server, socketPath, sockets);

  assert.equal(result.status, 0);
  const output = parseOutput(result);
  const groups = output.data?.groups as Array<Record<string, unknown>>;
  assert.equal(groups.length, 1);
  assert.equal(groups[0].groupId, 44);
});

test("group-update passes update options", async () => {
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => (mockResponse(req, { groupId: 1 })));

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
      return mockResponse(req, snapshot);
    }
    return mockResponse(req, { groups: [] });
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
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => (mockResponse(req, { summary: { ungroupedTabs: 2 } })));

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
  const { socketPath, server, requests, sockets } = await startMockSocket((req) => (mockResponse(req, { summary: { groupedTabs: 2 } })));

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
  const output = parseOutput(result);
  assert.equal(output.data?.txid, "tx-group-assign");
  assert.equal(output.version, "0.1.0");
  assert.equal(output.component, "host");
  assert.equal(output.data?.extensionVersion, "0.1.0");
  assert.equal(output.data?.hostBaseVersion, "0.1.0");
});
