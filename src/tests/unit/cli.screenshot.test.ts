import assert from "node:assert/strict";
import fs from "fs";
import os from "os";
import path from "path";
import test from "node:test";
import { startMockSocket, stopMockSocket } from "./socket";
import { runCli } from "./cli-helpers";

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
