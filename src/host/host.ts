#!/usr/bin/env node
import fs from "node:fs";
import net from "node:net";
import crypto from "node:crypto";
import { resolveConfig, parseSocketPath, writeTcpPortForWSL, DEFAULT_WSL_TCP_PORT } from "../shared/config";
import {
  handleNativeMessage as _handleNativeMessage,
  handleCliRequest as _handleCliRequest,
  respond,
  PendingRequest,
  HandlerDeps,
} from "./lib/handlers";

let config;
try {
  config = resolveConfig();
} catch (err) {
  process.stderr.write(`[tabctl-host] Fatal: ${(err as Error).message}\n`);
  process.exit(1);
}
const SOCKET_DIR = config.dataDir;
const SOCKET_PATH = config.socketPath;

const pending = new Map<string, PendingRequest>();
const analyses = new Map<string, { createdAt: number; data: Record<string, unknown> }>();

function log(...args: string[]) {
  process.stderr.write(`[tabctl-host] ${args.join(" ")}\n`);
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

const deps: HandlerDeps = {
  pending,
  analyses,
  undoLog: config.undoLog,
  createId,
  sendNative,
  log,
};

function handleNativeMessage(payload: string) {
  _handleNativeMessage(deps, payload);
}

function handleCliRequest(socket: net.Socket, request: Record<string, unknown>) {
  _handleCliRequest(deps, socket, request);
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

function startSocketServer() {
  ensureDir();

  const socketInfo = parseSocketPath(SOCKET_PATH);

  // Named pipes on Windows don't use filesystem paths; skip cleanup
  if (socketInfo.type === "unix" && fs.existsSync(SOCKET_PATH)) {
    fs.unlinkSync(SOCKET_PATH);
  }

  const servers: net.Server[] = [];

  function createHandler() {
    return (socket: net.Socket) => {
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
    };
  }

  // On Windows, listen on both named pipe (for Windows CLI) and TCP (for WSL CLI)
  if (process.platform === "win32") {
    // Primary: named pipe
    const pipeServer = net.createServer(createHandler());
    servers.push(pipeServer);

    pipeServer.on("error", (err: NodeJS.ErrnoException) => {
      if (err.code === "EADDRINUSE") {
        log("Named pipe in use, retrying...");
        setTimeout(() => pipeServer.listen(SOCKET_PATH), 500);
      } else {
        throw err;
      }
    });

    pipeServer.listen(SOCKET_PATH, () => {
      log(`Listening on ${SOCKET_PATH}`);
    });

    // Secondary: TCP socket for WSL access
    // Start at default port 24050, then increment if taken (up to +10 attempts)
    const defaultPort = DEFAULT_WSL_TCP_PORT;
    
    function tryTcpPort(port: number, attemptsLeft: number) {
      if (attemptsLeft <= 0) {
        log(`Failed to bind TCP socket after trying ports ${defaultPort}-${port - 1}`);
        return;
      }
      
      const tcpServer = net.createServer(createHandler());
      servers.push(tcpServer);

      tcpServer.on("error", (err: NodeJS.ErrnoException) => {
        if (err.code === "EADDRINUSE") {
          log(`Port ${port} already in use, trying ${port + 1}...`);
          // Remove failed server from list
          const idx = servers.indexOf(tcpServer);
          if (idx >= 0) servers.splice(idx, 1);
          // Try next port
          tryTcpPort(port + 1, attemptsLeft - 1);
        } else {
          log(`TCP socket failed (port ${port}): ${err.message}`);
        }
      });

      tcpServer.listen(port, "127.0.0.1", () => {
        log(`Listening on tcp://127.0.0.1:${port} (for WSL)`);
        if (port !== defaultPort) {
          log(`Note: Using port ${port} instead of default ${defaultPort} (port conflict)`);
        }
        // Write actual port file for WSL to discover
        writeTcpPortForWSL(config.dataDir, port);
      });
    }
    
    // Start with default port, allow up to 10 retries
    tryTcpPort(defaultPort, 10);

    return servers;
  }

  // Non-Windows: single server (TCP if configured, otherwise Unix socket)
  const server = net.createServer(createHandler());

  let retries = 0;
  const maxRetries = 0;

  server.on("error", (err: NodeJS.ErrnoException) => {
    if (err.code === "EADDRINUSE" && retries < maxRetries) {
      retries++;
      log(`Socket in use, retrying (${retries}/${maxRetries})…`);
      setTimeout(() => {
        if (socketInfo.type === "tcp") {
          server.listen(socketInfo.port!, socketInfo.host!);
        } else {
          server.listen(SOCKET_PATH);
        }
      }, 500);
    } else {
      throw err;
    }
  });

  if (socketInfo.type === "tcp") {
    server.listen(socketInfo.port!, socketInfo.host!, () => {
      log(`Listening on tcp://${socketInfo.host}:${socketInfo.port}`);
    });
  } else {
    server.listen(SOCKET_PATH, () => {
      if (socketInfo.type === "unix") {
        try { fs.chmodSync(SOCKET_PATH, 0o600); } catch { /* ignore on platforms without chmod */ }
      }
      log(`Listening on ${SOCKET_PATH}`);
    });
  }

  return [server];
}

function cleanupAndExit(code: number) {
  try {
    const socketInfo = parseSocketPath(SOCKET_PATH);
    // Only Unix sockets need filesystem cleanup
    if (socketInfo.type === "unix" && fs.existsSync(SOCKET_PATH)) {
      fs.unlinkSync(SOCKET_PATH);
    }
  } catch {
    // ignore
  }
  process.exit(code);
}

const servers = startSocketServer();

process.on("SIGINT", () => cleanupAndExit(0));
process.on("SIGTERM", () => cleanupAndExit(0));

for (const server of servers) {
  server.on("close", () => cleanupAndExit(0));
}
