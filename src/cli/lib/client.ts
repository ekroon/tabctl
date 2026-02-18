import net from "node:net";
import { resolveConfig, parseSocketPath } from "./constants";
import type { ProgressCallback } from "./types";

export function createRequestId(): string {
  return `req-${Date.now()}-${Math.random().toString(16).slice(2, 10)}`;
}

export function sendRequest(
  payload: Record<string, unknown>,
  onProgress?: ProgressCallback
): Promise<Record<string, unknown>> {
  return new Promise<Record<string, unknown>>((resolve, reject) => {
    const { socketPath } = resolveConfig();
    const socketInfo = parseSocketPath(socketPath);

    let client: net.Socket;
    if (socketInfo.type === "tcp") {
      client = net.createConnection({ port: socketInfo.port!, host: socketInfo.host! });
    } else {
      client = net.createConnection(socketPath);
    }

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

/** Send a request without waiting for a response (fire-and-forget). */
export function sendFireAndForget(payload: Record<string, unknown>): void {
  try {
    const { socketPath } = resolveConfig();
    const socketInfo = parseSocketPath(socketPath);

    let client: net.Socket;
    if (socketInfo.type === "tcp") {
      client = net.createConnection({ port: socketInfo.port!, host: socketInfo.host! });
    } else {
      client = net.createConnection(socketPath);
    }

    client.on("connect", () => {
      client.write(`${JSON.stringify(payload)}\n`);
      // Unref after write so Node can exit without waiting for response
      client.unref();
      const timer = setTimeout(() => { client.end(); client.destroy(); }, 200);
      timer.unref();
    });
    client.on("error", () => {
      // Silently ignore — this is best-effort
    });
  } catch {
    // Silently ignore
  }
}
