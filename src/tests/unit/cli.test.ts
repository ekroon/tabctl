import assert from "node:assert/strict";
import fs from "fs";
import os from "os";
import path from "path";
import { spawn } from "node:child_process";
import test from "node:test";
import { startMockSocket, stopMockSocket } from "./socket";

const cliPath = path.resolve(__dirname, "../../cli/tabctl.js");

async function runCli(args: string[], socketPath?: string, extraEnv?: Record<string, string>) {
  const env = { ...process.env };
  if (socketPath) {
    env.TABARCHIVE_SOCKET = socketPath;
  }
  if (extraEnv) {
    Object.assign(env, extraEnv);
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

test("policy init creates default file", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "tabarchive-policy-init-"));
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

test("help supports json output", async () => {
  const result = await runCli(["help", "--json"]);
  assert.equal(result.status, 0);
  const output = JSON.parse(result.stdout.trim());
  assert.equal(output.ok, true);
  assert.ok(output.data?.commands);
});
