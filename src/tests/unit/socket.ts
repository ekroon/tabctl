import fs from "fs";
import net from "net";
import os from "os";
import path from "path";
import crypto from "crypto";

export function createSocketPath(): string {
  const name = `ta-${process.pid}-${Date.now()}-${crypto.randomBytes(3).toString("hex")}.sock`;
  return path.join(os.tmpdir(), name);
}

export async function startMockSocket(handler: (request: Record<string, unknown>) => Record<string, unknown> | void) {
  const socketPath = createSocketPath();
  if (fs.existsSync(socketPath)) {
    fs.unlinkSync(socketPath);
  }

  const requests: Array<Record<string, unknown>> = [];
  const sockets = new Set<net.Socket>();
  const server = net.createServer((socket) => {
    sockets.add(socket);
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
          socket.write(JSON.stringify({ ok: false, error: { message: "Invalid JSON" } }) + "\n");
          continue;
        }

        requests.push(request);
        const response = handler(request) || {
          ok: true,
          action: request.action,
          requestId: request.id,
          data: {},
        };
        socket.write(JSON.stringify(response) + "\n");
      }
    });

    socket.on("close", () => {
      sockets.delete(socket);
    });
  });

  await new Promise<void>((resolve) => {
    server.listen(socketPath, () => resolve());
  });

  return { socketPath, server, requests, sockets };
}

export async function stopMockSocket(server: net.Server, socketPath: string, sockets?: Set<net.Socket>) {
  if (sockets) {
    for (const socket of sockets) {
      socket.destroy();
    }
  }
  await new Promise<void>((resolve) => {
    server.close(() => resolve());
  });

  if (fs.existsSync(socketPath)) {
    fs.unlinkSync(socketPath);
  }
}
