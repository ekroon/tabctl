/**
 * CDP utilities for integration tests — Chrome discovery, launch, and
 * DevTools Protocol messaging over stdio pipes.
 */

import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { spawn, type ChildProcess } from "node:child_process";
import type { Writable, Readable } from "node:stream";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

export function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

export function log(msg: string): void {
  console.log(`[integration] ${msg}`);
}

export function waitForFile(filePath: string, timeoutMs: number): Promise<void> {
  const start = Date.now();
  return new Promise((resolve, reject) => {
    const check = () => {
      if (fs.existsSync(filePath)) return resolve();
      if (Date.now() - start > timeoutMs) {
        return reject(new Error(`Timed out waiting for ${filePath}`));
      }
      setTimeout(check, 500);
    };
    check();
  });
}

/** Wait for a socket/named pipe to become available by attempting to connect. */
export function waitForSocket(socketPath: string, timeoutMs: number): Promise<void> {
  if (process.platform !== "win32") {
    return waitForFile(socketPath, timeoutMs);
  }
  // On Windows, named pipes can't be detected with fs.existsSync; try connecting
  const start = Date.now();
  return new Promise((resolve, reject) => {
    const check = () => {
      if (Date.now() - start > timeoutMs) {
        return reject(new Error(`Timed out waiting for ${socketPath}`));
      }
      const client = net.createConnection(socketPath);
      client.on("connect", () => { client.destroy(); resolve(); });
      client.on("error", () => { setTimeout(check, 500); });
    };
    check();
  });
}

// ---------------------------------------------------------------------------
// Chrome discovery
// ---------------------------------------------------------------------------

export function findChrome(): string {
  if (process.env.CHROME_PATH) return process.env.CHROME_PATH;
  const candidates: string[] = process.platform === "win32" ? [
    path.join(process.env["PROGRAMFILES"] || "C:\\Program Files", "Google", "Chrome", "Application", "chrome.exe"),
    path.join(process.env["PROGRAMFILES(X86)"] || "C:\\Program Files (x86)", "Google", "Chrome", "Application", "chrome.exe"),
    path.join(process.env.LOCALAPPDATA || "", "Google", "Chrome", "Application", "chrome.exe"),
  ] : process.platform === "linux" ? [
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
    "/usr/bin/chromium-browser",
    "/usr/bin/chromium",
  ] : [
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
  ];
  for (const p of candidates) {
    if (fs.existsSync(p)) return p;
  }
  throw new Error("Chrome not found. Set CHROME_PATH env var.");
}

// ---------------------------------------------------------------------------
// CDP messaging over pipes (fd 3 = write, fd 4 = read, null-byte delimited)
// ---------------------------------------------------------------------------

let cdpId = 0;
const pendingCdp = new Map<number, { resolve: Function; reject: Function }>();
let cdpWrite: Writable;
let cdpRead: Readable;
let cdpBuffer = "";

export function initCDP(chrome: ChildProcess): void {
  cdpWrite = chrome.stdio![3] as Writable;
  cdpRead = chrome.stdio![4] as Readable;

  cdpRead.on("data", (chunk: Buffer) => {
    cdpBuffer += chunk.toString("utf8");
    const parts = cdpBuffer.split("\0");
    cdpBuffer = parts.pop()!;
    for (const part of parts) {
      if (!part.trim()) continue;
      try {
        const msg = JSON.parse(part);
        if (msg.id && pendingCdp.has(msg.id)) {
          const p = pendingCdp.get(msg.id)!;
          pendingCdp.delete(msg.id);
          if (msg.error) p.reject(new Error(msg.error.message));
          else p.resolve(msg.result);
        }
      } catch {
        // ignore malformed fragments
      }
    }
  });
}

export function sendCDP(method: string, params: Record<string, unknown> = {}, sessionId?: string): Promise<any> {
  return new Promise((resolve, reject) => {
    const id = ++cdpId;
    pendingCdp.set(id, { resolve, reject });
    const payload: Record<string, unknown> = { id, method, params };
    if (sessionId) payload.sessionId = sessionId;
    const msg = JSON.stringify(payload) + "\0";
    cdpWrite.write(msg);
  });
}

// ---------------------------------------------------------------------------
// Chrome launch
// ---------------------------------------------------------------------------

export function launchChrome(chromePath: string, userDataDir: string): ChildProcess {
  const chrome = spawn(chromePath, [
    "--headless=new",
    "--remote-debugging-pipe",
    "--enable-unsafe-extension-debugging",
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-gpu",
    "--disable-background-timer-throttling",
    "--user-data-dir=" + userDataDir,
  ], {
    stdio: ["ignore", "pipe", "pipe", "pipe", "pipe"],
  });
  return chrome;
}

// ---------------------------------------------------------------------------
// Extension loading via CDP
// ---------------------------------------------------------------------------

export async function loadExtension(
  extensionDir: string,
): Promise<string> {
  log(`Loading extension from ${extensionDir}`);
  const loadResult = await sendCDP("Extensions.loadUnpacked", { path: extensionDir });
  const extensionId: string = loadResult.id;
  log(`Extension loaded: ${extensionId}`);
  return extensionId;
}

export async function attachServiceWorker(
  extensionId: string,
  chrome: ChildProcess,
  stderrBuf: { value: string },
): Promise<string> {
  let swTarget: { targetId: string; type: string; url: string } | undefined;
  for (let attempt = 0; attempt < 20; attempt++) {
    if (chrome.exitCode !== null) {
      log(`Chrome stderr: ${stderrBuf.value.slice(-500)}`);
      throw new Error(`Chrome exited during service worker discovery (code ${chrome.exitCode})`);
    }
    const targets = await sendCDP("Target.getTargets");
    swTarget = (targets.targetInfos as Array<{ targetId: string; type: string; url: string }>)
      .find((t) => t.type === "service_worker" && t.url.includes(extensionId));
    if (swTarget) break;
    log(`Service worker not found yet (attempt ${attempt + 1}/20)…`);
    await sleep(500);
  }
  if (!swTarget) {
    log(`Chrome stderr: ${stderrBuf.value.slice(-500)}`);
    throw new Error("Extension service worker not found");
  }

  const { sessionId } = await sendCDP("Target.attachToTarget", {
    targetId: swTarget.targetId,
    flatten: true,
  });

  return sessionId;
}
