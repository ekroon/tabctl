#!/usr/bin/env node
import fs from "fs";
import net from "net";
import os from "os";
import path from "path";
import crypto from "crypto";
import {
  appendUndoRecord,
  readUndoRecords,
  filterByRetention,
  findUndoRecord,
} from "./lib/undo";

const SOCKET_DIR = path.join(os.homedir(), ".tabarchive");
const SOCKET_PATH = path.join(SOCKET_DIR, "tabarchive.sock");
const UNDO_LOG = path.join(SOCKET_DIR, "undo.jsonl");
const REQUEST_TIMEOUT_MS = 30000;
const HISTORY_LIMIT_DEFAULT = 20;
const RETENTION_DAYS = 30;

type PendingRequest = {
  socket: net.Socket;
  action: string;
  txid: string | null;
  timeout: NodeJS.Timeout;
};

const pending = new Map<string, PendingRequest>();
const analyses = new Map<string, { createdAt: number; data: Record<string, unknown> }>();

function log(...args: string[]) {
  process.stderr.write(`[tabarchive-host] ${args.join(" ")}\n`);
}

function ensureDir() {
  fs.mkdirSync(SOCKET_DIR, { recursive: true, mode: 0o700 });
}

function createId(prefix: string) {
  return `${prefix}-${Date.now()}-${crypto.randomBytes(4).toString("hex")}`;
}

function sendNative(message: Record<string, unknown>) {
  const json = JSON.stringify(message);
  const length = Buffer.byteLength(json);
  const buffer = Buffer.alloc(4 + length);
  buffer.writeUInt32LE(length, 0);
  buffer.write(json, 4);
  process.stdout.write(buffer);
}

let nativeBuffer = Buffer.alloc(0);
process.stdin.on("data", (chunk) => {
  nativeBuffer = Buffer.concat([nativeBuffer, chunk]);
  while (nativeBuffer.length >= 4) {
    const length = nativeBuffer.readUInt32LE(0);
    if (nativeBuffer.length < 4 + length) {
      return;
    }
    const payload = nativeBuffer.slice(4, 4 + length).toString("utf8");
    nativeBuffer = nativeBuffer.slice(4 + length);
    handleNativeMessage(payload);
  }
});

process.stdin.on("end", () => {
  log("Extension disconnected, exiting");
  cleanupAndExit(0);
});

function handleNativeMessage(payload: string) {
  let message: Record<string, unknown>;
  try {
    message = JSON.parse(payload);
  } catch (error) {
    const err = error as Error;
    log("Failed to parse native message", err.message);
    return;
  }

  const messageId = message.id as string | undefined;
  if (!messageId) {
    return;
  }

  const pendingRequest = pending.get(messageId);
  if (!pendingRequest) {
    return;
  }

  if (message.progress) {
    refreshTimeout(pendingRequest, messageId);
    respond(pendingRequest.socket, {
      ok: true,
      action: pendingRequest.action,
      requestId: messageId,
      progress: true,
      data: message.data || {},
    });
    return;
  }

  clearTimeout(pendingRequest.timeout);
  pending.delete(messageId);

  if (!message.ok) {
    respond(pendingRequest.socket, {
      ok: false,
      action: pendingRequest.action,
      requestId: messageId,
      error: message.error || { message: "Unknown error" },
    });
    return;
  }

  if (pendingRequest.action === "analyze") {
    const analysisId = createId("analysis");
    analyses.set(analysisId, {
      createdAt: Date.now(),
      data: message.data as Record<string, unknown>,
    });
    respond(pendingRequest.socket, {
      ok: true,
      action: "analyze",
      requestId: messageId,
      data: {
        ...(message.data as Record<string, unknown>),
        analysisId,
      },
    });
    return;
  }

  if (pendingRequest.action === "archive" || pendingRequest.action === "close") {
    const record = {
      txid: pendingRequest.txid,
      createdAt: Date.now(),
      action: pendingRequest.action,
      summary: (message.data as Record<string, unknown>)?.summary || {},
      undo: (message.data as Record<string, unknown>)?.undo || null,
    };

    if (record.undo) {
      appendUndoRecord(UNDO_LOG, record);
    }

    respond(pendingRequest.socket, {
      ok: true,
      action: pendingRequest.action,
      requestId: messageId,
      data: {
        ...(message.data as Record<string, unknown>),
        txid: pendingRequest.txid,
      },
    });
    return;
  }

  respond(pendingRequest.socket, {
    ok: true,
    action: pendingRequest.action,
    requestId: messageId,
    data: message.data,
  });
}

function respond(socket: net.Socket, payload: Record<string, unknown>) {
  socket.write(`${JSON.stringify(payload)}\n`);
}

function refreshTimeout(pendingRequest: PendingRequest, requestId: string) {
  clearTimeout(pendingRequest.timeout);
  pendingRequest.timeout = setTimeout(() => {
    pending.delete(requestId);
    respond(pendingRequest.socket, {
      ok: false,
      action: pendingRequest.action,
      requestId,
      error: { message: "Request timed out" },
    });
  }, REQUEST_TIMEOUT_MS);
}

function forwardToExtension(
  socket: net.Socket,
  request: Record<string, unknown>,
  overrides: { txid?: string } = {},
) {
  const requestId = (request.id as string) || createId("req");
  const txid = overrides.txid || null;
  const params = { ...((request.params as Record<string, unknown>) || {}) };
  if (txid) {
    params.txid = txid;
  }

  pending.set(requestId, {
    socket,
    action: request.action as string,
    txid,
    timeout: setTimeout(() => {
      pending.delete(requestId);
      respond(socket, {
        ok: false,
        action: request.action,
        requestId,
        error: { message: "Request timed out" },
      });
    }, REQUEST_TIMEOUT_MS),
  });

  sendNative({ id: requestId, action: request.action, params });
}

function handleCliRequest(socket: net.Socket, request: Record<string, unknown>) {
  if (!request || typeof request !== "object") {
    respond(socket, { ok: false, error: { message: "Invalid request" } });
    return;
  }

  const action = request.action as string | undefined;
  if (!action) {
    respond(socket, { ok: false, error: { message: "Missing action" } });
    return;
  }

  if (action === "history") {
    const limit = Number.isFinite((request.params as Record<string, unknown>)?.limit)
      ? Number((request.params as Record<string, unknown>)?.limit)
      : HISTORY_LIMIT_DEFAULT;
    const records = readUndoRecords(UNDO_LOG);
    const filtered = filterByRetention(records, RETENTION_DAYS);
    respond(socket, {
      ok: true,
      action,
      requestId: request.id || null,
      data: filtered.slice(-limit),
    });
    return;
  }

  if (action === "undo") {
    const txid = (request.params as Record<string, unknown>)?.txid as string | undefined;
    if (!txid) {
      respond(socket, { ok: false, action, error: { message: "Missing txid" } });
      return;
    }
    const record = findUndoRecord(UNDO_LOG, txid, RETENTION_DAYS);
    if (!record) {
      respond(socket, { ok: false, action, error: { message: "Undo record not found" } });
      return;
    }
    forwardToExtension(socket, {
      id: request.id,
      action: "undo",
      params: { record },
    });
    return;
  }

  if (action === "close" && (request.params as Record<string, unknown>)?.mode === "apply") {
    const analysisId = (request.params as Record<string, unknown>).analysisId as string | undefined;
    const analysis = analysisId ? analyses.get(analysisId) : undefined;
    if (!analysis) {
      respond(socket, { ok: false, action, error: { message: "Unknown analysisId" } });
      return;
    }

    const candidates = (analysis.data.candidates as Array<Record<string, unknown>>) || [];
    const tabIds = candidates.map((candidate) => candidate.tabId).filter(Boolean) as number[];
    const expectedUrls: Record<string, string> = {};
    for (const candidate of candidates) {
      if (candidate.tabId) {
        expectedUrls[String(candidate.tabId)] = candidate.url as string;
      }
    }

    if (!tabIds.length) {
      respond(socket, {
        ok: true,
        action,
        requestId: request.id || null,
        data: {
          txid: null,
          summary: { closedTabs: 0, skippedTabs: 0 },
          skipped: [],
        },
      });
      return;
    }

    const txid = createId("tx");
    forwardToExtension(socket, {
      id: request.id,
      action: "close",
      params: {
        mode: "apply",
        tabIds,
        expectedUrls,
      },
    }, { txid });
    return;
  }

  if (action === "archive" || action === "close") {
    const txid = createId("tx");
    forwardToExtension(socket, request, { txid });
    return;
  }

  forwardToExtension(socket, request);
}

function startSocketServer() {
  ensureDir();
  if (fs.existsSync(SOCKET_PATH)) {
    fs.unlinkSync(SOCKET_PATH);
  }

  const server = net.createServer((socket) => {
    socket.setEncoding("utf8");
    let buffer = "";

    socket.on("data", (data) => {
      buffer += data;
      let index;
      while ((index = buffer.indexOf("\n")) >= 0) {
        const line = buffer.slice(0, index).trim();
        buffer = buffer.slice(index + 1);
        if (!line) {
          continue;
        }
        let request: Record<string, unknown>;
        try {
          request = JSON.parse(line);
        } catch {
          respond(socket, { ok: false, error: { message: "Invalid JSON" } });
          continue;
        }
        handleCliRequest(socket, request);
      }
    });

    socket.on("error", (error) => {
      log("CLI socket error", error.message);
    });
  });

  server.listen(SOCKET_PATH, () => {
    fs.chmodSync(SOCKET_PATH, 0o600);
    log(`Listening on ${SOCKET_PATH}`);
  });

  return server;
}

function cleanupAndExit(code: number) {
  try {
    if (fs.existsSync(SOCKET_PATH)) {
      fs.unlinkSync(SOCKET_PATH);
    }
  } catch {
    // ignore
  }
  process.exit(code);
}

const server = startSocketServer();

process.on("SIGINT", () => cleanupAndExit(0));
process.on("SIGTERM", () => cleanupAndExit(0));

server.on("close", () => cleanupAndExit(0));
