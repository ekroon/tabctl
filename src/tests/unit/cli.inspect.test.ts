import assert from "node:assert/strict";
import test from "node:test";
import { startMockSocket, stopMockSocket } from "./socket";
import { runCli } from "./cli-helpers";

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
