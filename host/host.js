#!/usr/bin/env node
"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
const fs_1 = __importDefault(require("fs"));
const net_1 = __importDefault(require("net"));
const os_1 = __importDefault(require("os"));
const path_1 = __importDefault(require("path"));
const crypto_1 = __importDefault(require("crypto"));
const undo_1 = require("./lib/undo");
const SOCKET_DIR = path_1.default.join(os_1.default.homedir(), ".tabarchive");
const SOCKET_PATH = path_1.default.join(SOCKET_DIR, "tabarchive.sock");
const UNDO_LOG = path_1.default.join(SOCKET_DIR, "undo.jsonl");
const REQUEST_TIMEOUT_MS = 30000;
const HISTORY_LIMIT_DEFAULT = 20;
const RETENTION_DAYS = 30;
const pending = new Map();
const analyses = new Map();
function log(...args) {
    process.stderr.write(`[tabarchive-host] ${args.join(" ")}\n`);
}
function ensureDir() {
    fs_1.default.mkdirSync(SOCKET_DIR, { recursive: true, mode: 0o700 });
}
function createId(prefix) {
    return `${prefix}-${Date.now()}-${crypto_1.default.randomBytes(4).toString("hex")}`;
}
function sendNative(message) {
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
function handleNativeMessage(payload) {
    let message;
    try {
        message = JSON.parse(payload);
    }
    catch (error) {
        const err = error;
        log("Failed to parse native message", err.message);
        return;
    }
    const messageId = message.id;
    if (!messageId) {
        return;
    }
    const pendingRequest = pending.get(messageId);
    if (!pendingRequest) {
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
            data: message.data,
        });
        respond(pendingRequest.socket, {
            ok: true,
            action: "analyze",
            requestId: messageId,
            data: {
                ...message.data,
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
            summary: message.data?.summary || {},
            undo: message.data?.undo || null,
        };
        if (record.undo) {
            (0, undo_1.appendUndoRecord)(UNDO_LOG, record);
        }
        respond(pendingRequest.socket, {
            ok: true,
            action: pendingRequest.action,
            requestId: messageId,
            data: {
                ...message.data,
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
function respond(socket, payload) {
    socket.write(`${JSON.stringify(payload)}\n`);
}
function forwardToExtension(socket, request, overrides = {}) {
    const requestId = request.id || createId("req");
    const txid = overrides.txid || null;
    const params = { ...(request.params || {}) };
    if (txid) {
        params.txid = txid;
    }
    pending.set(requestId, {
        socket,
        action: request.action,
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
function handleCliRequest(socket, request) {
    if (!request || typeof request !== "object") {
        respond(socket, { ok: false, error: { message: "Invalid request" } });
        return;
    }
    const action = request.action;
    if (!action) {
        respond(socket, { ok: false, error: { message: "Missing action" } });
        return;
    }
    if (action === "history") {
        const limit = Number.isFinite(request.params?.limit)
            ? Number(request.params?.limit)
            : HISTORY_LIMIT_DEFAULT;
        const records = (0, undo_1.readUndoRecords)(UNDO_LOG);
        const filtered = (0, undo_1.filterByRetention)(records, RETENTION_DAYS);
        respond(socket, {
            ok: true,
            action,
            requestId: request.id || null,
            data: filtered.slice(-limit),
        });
        return;
    }
    if (action === "undo") {
        const txid = request.params?.txid;
        if (!txid) {
            respond(socket, { ok: false, action, error: { message: "Missing txid" } });
            return;
        }
        const record = (0, undo_1.findUndoRecord)(UNDO_LOG, txid, RETENTION_DAYS);
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
    if (action === "close" && request.params?.mode === "apply") {
        const analysisId = request.params.analysisId;
        const analysis = analysisId ? analyses.get(analysisId) : undefined;
        if (!analysis) {
            respond(socket, { ok: false, action, error: { message: "Unknown analysisId" } });
            return;
        }
        const candidates = analysis.data.candidates || [];
        const tabIds = candidates.map((candidate) => candidate.tabId).filter(Boolean);
        const expectedUrls = {};
        for (const candidate of candidates) {
            if (candidate.tabId) {
                expectedUrls[String(candidate.tabId)] = candidate.url;
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
    if (fs_1.default.existsSync(SOCKET_PATH)) {
        fs_1.default.unlinkSync(SOCKET_PATH);
    }
    const server = net_1.default.createServer((socket) => {
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
                let request;
                try {
                    request = JSON.parse(line);
                }
                catch {
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
        fs_1.default.chmodSync(SOCKET_PATH, 0o600);
        log(`Listening on ${SOCKET_PATH}`);
    });
    return server;
}
function cleanupAndExit(code) {
    try {
        if (fs_1.default.existsSync(SOCKET_PATH)) {
            fs_1.default.unlinkSync(SOCKET_PATH);
        }
    }
    catch {
        // ignore
    }
    process.exit(code);
}
const server = startSocketServer();
process.on("SIGINT", () => cleanupAndExit(0));
process.on("SIGTERM", () => cleanupAndExit(0));
server.on("close", () => cleanupAndExit(0));
