import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";
import test from "node:test";
import { readUndoRecords } from "../../host/lib/undo";
import { resolveSocketPath } from "../../shared/config";

const pkgVersion = JSON.parse(fs.readFileSync(path.resolve(__dirname, "../../../package.json"), "utf8")).version;

type NativeMessage = Record<string, unknown>;

function encodeNativeMessage(message: NativeMessage) {
  const json = JSON.stringify(message);
  const length = Buffer.byteLength(json);
  const buffer = Buffer.alloc(4 + length);
  buffer.writeUInt32LE(length, 0);
  buffer.write(json, 4);
  return buffer;
}

function sendNativeMessage(stream: NodeJS.WritableStream, message: NativeMessage) {
  stream.write(encodeNativeMessage(message));
}

function readNativeMessage(stream: NodeJS.ReadableStream, timeoutMs = 2000): Promise<NativeMessage> {
  return new Promise((resolve, reject) => {
    let buffer = Buffer.alloc(0);
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error("Timed out waiting for native message"));
    }, timeoutMs);

    const onData = (chunk: Buffer) => {
      buffer = Buffer.concat([buffer, chunk]);
      while (buffer.length >= 4) {
        const length = buffer.readUInt32LE(0);
        if (buffer.length < 4 + length) {
          return;
        }
        const payload = buffer.slice(4, 4 + length).toString("utf8");
        buffer = buffer.slice(4 + length);
        cleanup();
        try {
          resolve(JSON.parse(payload) as NativeMessage);
        } catch (error) {
          reject(error);
        }
        return;
      }
    };

    const onError = (error: Error) => {
      cleanup();
      reject(error);
    };

    const cleanup = () => {
      clearTimeout(timeout);
      stream.off("data", onData);
      stream.off("error", onError);
    };

    stream.on("data", onData);
    stream.on("error", onError);
  });
}

async function waitForSocket(socketPath: string, timeoutMs = 2000) {
  const start = Date.now();
  if (process.platform === "win32") {
    // Named pipes can't be detected with fs.existsSync; try connecting
    while (true) {
      if (Date.now() - start > timeoutMs) {
        throw new Error(`Timed out waiting for socket at ${socketPath}`);
      }
      try {
        await new Promise<void>((resolve, reject) => {
          const client = net.createConnection(socketPath);
          client.on("connect", () => { client.destroy(); resolve(); });
          client.on("error", reject);
        });
        return;
      } catch {
        await new Promise((resolve) => setTimeout(resolve, 50));
      }
    }
  }
  while (!fs.existsSync(socketPath)) {
    if (Date.now() - start > timeoutMs) {
      throw new Error(`Timed out waiting for socket at ${socketPath}`);
    }
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
}

function sendSocketRequest(socketPath: string, request: NativeMessage): Promise<NativeMessage> {
  return new Promise((resolve, reject) => {
    const client = net.createConnection(socketPath);
    let buffer = "";

    client.on("connect", () => {
      client.write(`${JSON.stringify(request)}\n`);
    });

    client.on("data", (data) => {
      buffer += data.toString();
      let index;
      while ((index = buffer.indexOf("\n")) >= 0) {
        const line = buffer.slice(0, index).trim();
        buffer = buffer.slice(index + 1);
        if (!line) {
          continue;
        }
        client.end();
        try {
          resolve(JSON.parse(line) as NativeMessage);
        } catch (error) {
          reject(error);
        }
        return;
      }
    });

    client.on("error", (error) => {
      reject(error);
    });
  });
}

async function startHost(stateHome: string, extraEnv: Record<string, string> = {}) {
  // Isolate from real profiles.json so socket uses stateHome-based path
  const cleanConfigHome = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-hostcfg-"));
  const hostPath = path.resolve(__dirname, "../../host/host.js");
  const child = spawn(process.execPath, [hostPath], {
    env: { ...process.env, ...extraEnv, XDG_STATE_HOME: stateHome, XDG_CONFIG_HOME: cleanConfigHome },
    stdio: ["pipe", "pipe", "pipe"],
  });

  if (!child.stdout || !child.stdin) {
    throw new Error("Failed to start host process");
  }

  const dataDir = path.join(stateHome, "tabctl");
  const socketPath = resolveSocketPath(dataDir);
  await waitForSocket(socketPath);
  const undoPath = path.join(dataDir, "undo.jsonl");
  return { child, socketPath, undoPath };
}

async function stopHost(child: ReturnType<typeof spawn>) {
  if (child.killed) {
    return;
  }
  child.kill("SIGTERM");
  await new Promise((resolve) => {
    const timeout = setTimeout(resolve, 1000);
    child.on("exit", () => {
      clearTimeout(timeout);
      resolve(null);
    });
  });
}

test("host records undo for move-tab", async () => {
  const stateHome = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-host-"));
  const { child, socketPath, undoPath } = await startHost(stateHome);

  try {
    const responsePromise = sendSocketRequest(socketPath, {
      id: "req-1",
      action: "move-tab",
      params: { tabId: 12 },
    });

    const forwarded = await readNativeMessage(child.stdout as NodeJS.ReadableStream);
    assert.equal(forwarded.action, "move-tab");
    const params = forwarded.params as Record<string, unknown> | undefined;
    const txid = params?.txid as string | undefined;
    assert.ok(txid);

    sendNativeMessage(child.stdin as NodeJS.WritableStream, {
      id: forwarded.id,
      ok: true,
      action: "move-tab",
      data: {
        summary: { movedTabs: 1 },
        undo: { action: "move-tab", tabs: [{ tabId: 12 }] },
      },
    });

    const response = await responsePromise;
    const responseData = response.data as Record<string, unknown> | undefined;
    assert.equal(responseData?.txid, txid);

    const records = readUndoRecords(undoPath);
    assert.equal(records.length, 1);
    assert.equal(records[0].txid, txid);
    assert.equal(records[0].action, "move-tab");
  } finally {
    await stopHost(child);
  }
});

test("host responds to version", async () => {
  const stateHome = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-host-"));
  const { child, socketPath } = await startHost(stateHome);

  try {
    const response = await sendSocketRequest(socketPath, {
      id: "req-version",
      action: "version",
      params: {},
    });

    assert.equal(response.ok, true);
    assert.equal(response.action, "version");
    const data = response.data as Record<string, unknown> | undefined;
    const version = data?.version as string | undefined;
    assert.ok(version);
    if (version && version.includes("-dev.")) {
      const re = new RegExp(`^${pkgVersion.replace(/\./g, "\\.")}-dev\\.[0-9a-f]{8}(\\.dirty)?$`);
      assert.match(version, re);
    } else {
      assert.equal(version, pkgVersion);
    }
    assert.equal(data?.component, "host");
    if (typeof response.version === "string" && response.version.includes("-dev.")) {
      const re = new RegExp(`^${pkgVersion.replace(/\./g, "\\.")}-dev\\.[0-9a-f]{8}(\\.dirty)?$`);
      assert.match(response.version, re);
    } else {
      assert.equal(response.version, pkgVersion);
    }
    assert.equal(data?.baseVersion, pkgVersion);
  } finally {
    await stopHost(child);
  }
});

test("host responds to version (dev when built)", async () => {
  const stateHome = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-host-"));
  const { child, socketPath } = await startHost(stateHome);

  try {
    const response = await sendSocketRequest(socketPath, {
      id: "req-version-dev",
      action: "version",
      params: {},
    });

    assert.equal(response.ok, true);
    const data = response.data as Record<string, unknown> | undefined;
    const version = data?.version as string | undefined;
    assert.ok(version);
    if (version && version.includes("-dev.")) {
      const re = new RegExp(`^${pkgVersion.replace(/\./g, "\\.")}-dev\\.[0-9a-f]{8}(\\.dirty)?$`);
      assert.match(version, re);
    } else {
      assert.equal(version, pkgVersion);
    }
    assert.equal(data?.baseVersion, pkgVersion);
  } finally {
    await stopHost(child);
  }
});

test("host skips undo when payload missing", async () => {
  const stateHome = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-host-"));
  const { child, socketPath, undoPath } = await startHost(stateHome);

  try {
    const responsePromise = sendSocketRequest(socketPath, {
      id: "req-2",
      action: "move-group",
      params: { groupId: 7 },
    });

    const forwarded = await readNativeMessage(child.stdout as NodeJS.ReadableStream);
    assert.equal(forwarded.action, "move-group");

    sendNativeMessage(child.stdin as NodeJS.WritableStream, {
      id: forwarded.id,
      ok: true,
      action: "move-group",
      data: {
        summary: { movedTabs: 2 },
      },
    });

    await responsePromise;
    const records = fs.existsSync(undoPath) ? readUndoRecords(undoPath) : [];
    assert.equal(records.length, 0);
  } finally {
    await stopHost(child);
  }
});
