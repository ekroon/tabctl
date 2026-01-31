import assert from "node:assert/strict";
import path from "path";
import { spawn } from "node:child_process";
import test from "node:test";
import { startMockSocket, stopMockSocket } from "./socket";

const cliPath = path.resolve(__dirname, "../../cli/tabctl.js");

async function runCli(args: string[], socketPath?: string) {
  const env = { ...process.env };
  if (socketPath) {
    env.TABARCHIVE_SOCKET = socketPath;
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
