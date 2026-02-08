import net from "net";
import { VERSION, BASE_VERSION, GIT_SHA, DIRTY } from "../../shared/version";
import {
  appendUndoRecord,
  readUndoRecords,
  filterByRetention,
  findUndoRecord,
  findLatestUndoRecord,
} from "./undo";

const REQUEST_TIMEOUT_MS = 30000;
const MAX_RESPONSE_BYTES = 20 * 1024 * 1024;
const HISTORY_LIMIT_DEFAULT = 20;
const RETENTION_DAYS = 30;

const UNDO_ACTIONS = new Set([
  "archive",
  "close",
  "group-update",
  "group-ungroup",
  "group-assign",
  "move-tab",
  "move-group",
  "merge-window",
]);
const LOCAL_ACTIONS = new Set(["history", "undo", "version"]);

export type PendingRequest = {
  socket: net.Socket;
  action: string;
  txid: string | null;
  timeout: NodeJS.Timeout;
};

export interface HandlerDeps {
  pending: Map<string, PendingRequest>;
  analyses: Map<string, { createdAt: number; data: Record<string, unknown> }>;
  undoLog: string;
  createId: (prefix: string) => string;
  sendNative: (message: Record<string, unknown>) => void;
  log: (...args: string[]) => void;
}

export function respond(socket: net.Socket, payload: Record<string, unknown>) {
  const serialized = JSON.stringify(payload);
  if (Buffer.byteLength(serialized, "utf8") > MAX_RESPONSE_BYTES) {
    socket.write(`${JSON.stringify({
      ok: false,
      action: payload.action,
      requestId: payload.requestId,
      component: "host",
      version: VERSION,
      error: { message: "Response too large", hint: "Reduce scope or use --out to write files." },
    })}\n`);
    return;
  }
  socket.write(`${serialized}\n`);
}

export function refreshTimeout(
  deps: HandlerDeps,
  pendingRequest: PendingRequest,
  requestId: string,
) {
  clearTimeout(pendingRequest.timeout);
  pendingRequest.timeout = setTimeout(() => {
    deps.pending.delete(requestId);
    respond(pendingRequest.socket, {
      ok: false,
      action: pendingRequest.action,
      requestId,
      component: "host",
      version: VERSION,
      error: { message: "Request timed out" },
    });
  }, REQUEST_TIMEOUT_MS);
}

export function forwardToExtension(
  deps: HandlerDeps,
  socket: net.Socket,
  request: Record<string, unknown>,
  overrides: { txid?: string } = {},
) {
  const requestId = (request.id as string) || deps.createId("req");
  const txid = overrides.txid || null;
  const params = { ...((request.params as Record<string, unknown>) || {}) };
  if (txid) {
    params.txid = txid;
  }

  if (!LOCAL_ACTIONS.has(request.action as string)) {
    params.client = {
      component: "host",
      version: VERSION,
    };
  }

  deps.pending.set(requestId, {
    socket,
    action: request.action as string,
    txid,
    timeout: setTimeout(() => {
      deps.pending.delete(requestId);
      respond(socket, {
        ok: false,
        action: request.action,
        requestId,
        component: "host",
        version: VERSION,
        error: { message: "Request timed out" },
      });
    }, REQUEST_TIMEOUT_MS),
  });

  deps.sendNative({ id: requestId, action: request.action, params });
}

export function handleNativeMessage(deps: HandlerDeps, payload: string) {
  let message: Record<string, unknown>;
  try {
    message = JSON.parse(payload);
  } catch (error) {
    const err = error as Error;
    deps.log("Failed to parse native message", err.message);
    return;
  }

  const messageId = message.id as string | undefined;
  if (!messageId) {
    return;
  }

  const pendingRequest = deps.pending.get(messageId);
  if (!pendingRequest) {
    return;
  }

  if (message.progress) {
    refreshTimeout(deps, pendingRequest, messageId);
    respond(pendingRequest.socket, {
      ok: true,
      action: pendingRequest.action,
      requestId: messageId,
      progress: true,
      component: "host",
      version: VERSION,
      data: message.data || {},
    });
    return;
  }

  const messageData = (message.data as Record<string, unknown>) || {};
  const extensionVersion = typeof messageData.version === "string" ? messageData.version : null;
  const extensionComponent = typeof messageData.component === "string" ? messageData.component : null;
  if (extensionVersion && extensionVersion !== VERSION) {
    deps.log(`Version mismatch: host ${VERSION}, extension ${extensionVersion}`);
  }

  clearTimeout(pendingRequest.timeout);
  deps.pending.delete(messageId);

  if (!message.ok) {
    respond(pendingRequest.socket, {
      ok: false,
      action: pendingRequest.action,
      requestId: messageId,
      component: "host",
      version: VERSION,
      error: message.error || { message: "Unknown error" },
    });
    return;
  }

  if (pendingRequest.action === "analyze") {
    const analysisId = deps.createId("analysis");
    deps.analyses.set(analysisId, {
      createdAt: Date.now(),
      data: messageData,
    });
    respond(pendingRequest.socket, {
      ok: true,
      action: "analyze",
      requestId: messageId,
      component: "host",
      version: VERSION,
      data: {
        ...messageData,
        extensionVersion,
        extensionComponent,
        hostBaseVersion: BASE_VERSION,
        hostGitSha: GIT_SHA,
        hostDirty: DIRTY,
        analysisId,
      },
    });
    return;
  }

  if (UNDO_ACTIONS.has(pendingRequest.action)) {
    const record = {
      txid: pendingRequest.txid,
      createdAt: Date.now(),
      action: pendingRequest.action,
      summary: messageData.summary || {},
      undo: messageData.undo || null,
    };

    if (record.undo) {
      appendUndoRecord(deps.undoLog, record);
    }

    respond(pendingRequest.socket, {
      ok: true,
      action: pendingRequest.action,
      requestId: messageId,
      component: "host",
      version: VERSION,
      data: {
        ...messageData,
        extensionVersion,
        extensionComponent,
        hostBaseVersion: BASE_VERSION,
        hostGitSha: GIT_SHA,
        hostDirty: DIRTY,
        txid: pendingRequest.txid,
      },
    });
    return;
  }

  respond(pendingRequest.socket, {
    ok: true,
    action: pendingRequest.action,
    requestId: messageId,
    component: "host",
    version: VERSION,
    data: {
      ...messageData,
      extensionVersion,
      extensionComponent,
      hostBaseVersion: BASE_VERSION,
      hostGitSha: GIT_SHA,
      hostDirty: DIRTY,
    },
  });
}

export function handleCliRequest(deps: HandlerDeps, socket: net.Socket, request: Record<string, unknown>) {
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
    const records = readUndoRecords(deps.undoLog);
    const filtered = filterByRetention(records, RETENTION_DAYS);
    respond(socket, {
      ok: true,
      action,
      requestId: request.id || null,
      component: "host",
      version: VERSION,
      data: filtered.slice(-limit),
    });
    return;
  }

  if (action === "version") {
    respond(socket, {
      ok: true,
      action,
      requestId: request.id || null,
      component: "host",
      version: VERSION,
      data: {
        version: VERSION,
        baseVersion: BASE_VERSION,
        gitSha: GIT_SHA,
        dirty: DIRTY,
        component: "host",
      },
    });
    return;
  }

  if (action === "undo") {
    const txid = (request.params as Record<string, unknown>)?.txid as string | undefined;
    const latest = (request.params as Record<string, unknown>)?.latest === true;
    if (!txid && !latest) {
      respond(socket, {
        ok: false,
        action,
        component: "host",
        version: VERSION,
        error: {
          message: "Missing txid",
          hint: "Use tabctl history --json to find a txid, or run tabctl undo --latest",
        },
      });
      return;
    }
    const record = txid
      ? findUndoRecord(deps.undoLog, txid, RETENTION_DAYS)
      : findLatestUndoRecord(deps.undoLog, RETENTION_DAYS);
    if (!record) {
      respond(socket, { ok: false, action, component: "host", version: VERSION, error: { message: "Undo record not found" } });
      return;
    }
    forwardToExtension(deps, socket, {
      id: request.id,
      action: "undo",
      params: { record },
    });
    return;
  }

  if (action === "close" && (request.params as Record<string, unknown>)?.mode === "apply") {
    const analysisId = (request.params as Record<string, unknown>).analysisId as string | undefined;
    const analysis = analysisId ? deps.analyses.get(analysisId) : undefined;
    if (!analysis) {
      respond(socket, { ok: false, action, component: "host", version: VERSION, error: { message: "Unknown analysisId" } });
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
        component: "host",
        version: VERSION,
        data: {
          txid: null,
          summary: { closedTabs: 0, skippedTabs: 0 },
          skipped: [],
        },
      });
      return;
    }

    const txid = deps.createId("tx");
    forwardToExtension(deps, socket, {
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

  if (UNDO_ACTIONS.has(action)) {
    const txid = deps.createId("tx");
    forwardToExtension(deps, socket, request, { txid });
    return;
  }

  forwardToExtension(deps, socket, request);
}
