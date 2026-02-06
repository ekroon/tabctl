import fs from "fs";
import net from "net";
import path from "path";
import { loadConfig, resolveBrowser, resolveSocketPath } from "../../shared/config";
import { STATE_HOME, LEGACY_SOCKET_PATH } from "./constants";
import type { ProgressCallback } from "./types";

export function createRequestId(): string {
  return `req-${Date.now()}-${Math.random().toString(16).slice(2, 10)}`;
}

function resolveClientSocketPath(): string {
  if (process.env.TABCTL_SOCKET) {
    return process.env.TABCTL_SOCKET;
  }
  const config = loadConfig();
  const browser = resolveBrowser(config);
  const socketPath = resolveSocketPath(STATE_HOME, browser, config);
  const hasCustomSocket = Boolean(config?.socketName);
  if (browser === "edge" && !hasCustomSocket && !fs.existsSync(socketPath)) {
    const legacyEdgeSocket = path.join(STATE_HOME, "tabctl", "tabctl.sock");
    if (fs.existsSync(legacyEdgeSocket)) {
      return legacyEdgeSocket;
    }
    // Backward compatibility for older Edge installs that still use the tabarchive socket.
    return LEGACY_SOCKET_PATH;
  }
  // Chrome uses its own socket path without legacy fallback.
  return socketPath;
}

export function sendRequest(
  payload: Record<string, unknown>,
  onProgress?: ProgressCallback
): Promise<Record<string, unknown>> {
  return new Promise<Record<string, unknown>>((resolve, reject) => {
    const socketPath = resolveClientSocketPath();
    const client = net.createConnection(socketPath);
    let buffer = "";

    client.on("connect", () => {
      client.write(`${JSON.stringify(payload)}\n`);
    });

    client.on("data", (data) => {
      buffer += data;
      let index;
      while ((index = buffer.indexOf("\n")) >= 0) {
        const line = buffer.slice(0, index).trim();
        buffer = buffer.slice(index + 1);
        if (!line) {
          continue;
        }
        let response: Record<string, unknown>;
        try {
          response = JSON.parse(line);
        } catch (error) {
          client.end();
          client.destroy();
          reject(error);
          return;
        }

        if (response.progress && onProgress) {
          onProgress(response);
          continue;
        }

        client.end();
        client.destroy();
        resolve(response);
        return;
      }
    });

    client.on("error", (error) => {
      reject(error);
    });
  });
}

export async function fetchSnapshot(): Promise<Record<string, unknown> | null> {
  const response = await sendRequest({ id: createRequestId(), action: "list", params: {} });
  if (!response.ok) {
    return null;
  }
  return response.data as Record<string, unknown> | null;
}
