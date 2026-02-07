/**
 * CDP integration test — launches headless Chrome, loads the tabctl extension
 * via DevTools Protocol over pipes, and runs CLI commands against a real browser.
 *
 * Zero npm dependencies: raw CDP over child_process.spawn stdio pipes.
 *
 * Usage:  node build/scripts/integration-test.js
 * Env:    CHROME_PATH – override Chrome binary location
 */

import fs from "fs";
import os from "os";
import path from "path";
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import type { Writable, Readable } from "node:stream";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

function log(msg: string): void {
  console.log(`[integration] ${msg}`);
}

function waitForFile(filePath: string, timeoutMs: number): Promise<void> {
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

// ---------------------------------------------------------------------------
// Chrome discovery
// ---------------------------------------------------------------------------

function findChrome(): string {
  if (process.env.CHROME_PATH) return process.env.CHROME_PATH;
  const candidates = [
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

function initCDP(chrome: ChildProcess): void {
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

function sendCDP(method: string, params: Record<string, unknown> = {}, sessionId?: string): Promise<any> {
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
// Main
// ---------------------------------------------------------------------------

const GLOBAL_TIMEOUT_MS = 90_000;
const DEFAULT_HOST_NAME = "com.erwinkroon.tabctl";

async function main(): Promise<void> {
  if (process.platform !== "darwin") {
    log("Integration tests are macOS-only (native messaging paths).");
    process.exit(0);
  }

  // Global timeout guard
  const killTimer = setTimeout(() => {
    log("FATAL: global timeout reached – aborting");
    process.exit(2);
  }, GLOBAL_TIMEOUT_MS);

  // Temp dirs
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-integration-"));
  const userDataDir = path.join(tmpDir, "chrome-profile");
  const configHome = path.join(tmpDir, "xdg-config");
  const stateHome = path.join(tmpDir, "xdg-state");
  const configDir = path.join(configHome, "tabctl");
  const dataDir = path.join(stateHome, "tabctl");
  fs.mkdirSync(userDataDir, { recursive: true });
  fs.mkdirSync(configDir, { recursive: true });
  fs.mkdirSync(dataDir, { recursive: true });

  let chrome: ChildProcess | null = null;
  let manifestPath: string | null = null;

  try {
    // 1. Find Chrome
    const chromePath = findChrome();
    log(`Chrome: ${chromePath}`);

    // 2. Launch headless Chrome with CDP pipe.
    //    Load the extension via --load-extension so the service worker registers
    //    at startup (Extensions.loadUnpacked alone can cause Chrome to exit
    //    before the worker is ready in headless mode).
    const extensionDir = path.resolve(__dirname, "..", "..", "extension");
    chrome = spawn(chromePath, [
      "--headless=new",
      "--remote-debugging-pipe",
      "--enable-unsafe-extension-debugging",
      "--no-first-run",
      "--no-default-browser-check",
      "--disable-gpu",
      "--disable-background-timer-throttling",
      "--no-sandbox",
      `--disable-extensions-except=${extensionDir}`,
      `--load-extension=${extensionDir}`,
      "--user-data-dir=" + userDataDir,
    ], {
      stdio: ["ignore", "pipe", "pipe", "pipe", "pipe"],
    });

    chrome.on("exit", (code) => {
      log(`Chrome exited (code ${code})`);
    });

    // Drain stdout and capture stderr for diagnostics
    chrome.stdout?.resume();
    let stderrBuf = "";
    chrome.stderr?.on("data", (chunk: Buffer) => {
      stderrBuf += chunk.toString();
    });

    // Wait for Chrome to initialize
    await sleep(3000);

    // Verify Chrome is still running
    if (chrome.exitCode !== null) {
      throw new Error(`Chrome exited prematurely with code ${chrome.exitCode}`);
    }

    initCDP(chrome);

    // 3. Discover the extension ID from the service worker target.
    //    The extension was loaded at startup via --load-extension.
    //    The first connectNative() will fail (no manifest yet) — that's expected.
    log(`Loading extension from ${extensionDir}`);
    let swTarget: { targetId: string; type: string; url: string } | undefined;
    let extensionId = "";
    for (let attempt = 0; attempt < 15; attempt++) {
      await sleep(1000);
      if (chrome.exitCode !== null) {
        log(`Chrome stderr: ${stderrBuf.slice(-500)}`);
        throw new Error(`Chrome exited during service worker discovery (code ${chrome.exitCode})`);
      }
      const targets = await sendCDP("Target.getTargets");
      swTarget = (targets.targetInfos as Array<{ targetId: string; type: string; url: string }>)
        .find((t) => t.type === "service_worker" && t.url.startsWith("chrome-extension://"));
      if (swTarget) {
        const m = swTarget.url.match(/chrome-extension:\/\/([^/]+)/);
        extensionId = m ? m[1] : "";
        break;
      }
      log(`Service worker not found yet (attempt ${attempt + 1}/15)…`);
    }
    if (!swTarget || !extensionId) {
      log(`Chrome stderr: ${stderrBuf.slice(-500)}`);
      throw new Error("Extension service worker not found");
    }
    log(`Extension loaded: ${extensionId}`);

    // 4. Write native messaging manifest inside the user-data-dir.
    //    Chrome with --user-data-dir looks for manifests in
    //    <user-data-dir>/NativeMessagingHosts/, NOT the system-level path.
    const manifestDir = path.join(userDataDir, "NativeMessagingHosts");
    fs.mkdirSync(manifestDir, { recursive: true });

    const hostPath = path.resolve(__dirname, "..", "..", "host", "host.js");
    const wrapperPath = path.join(dataDir, "tabctl-host.sh");

    const wrapper = [
      "#!/usr/bin/env bash",
      "set -euo pipefail",
      `export XDG_CONFIG_HOME="${configHome}"`,
      `export XDG_STATE_HOME="${stateHome}"`,
      `exec "${process.execPath}" "${hostPath}"`,
      "",
    ].join("\n");
    fs.writeFileSync(wrapperPath, wrapper, { mode: 0o700 });

    manifestPath = path.join(manifestDir, `${DEFAULT_HOST_NAME}.json`);
    const manifest = {
      name: DEFAULT_HOST_NAME,
      description: "tabctl integration test",
      path: wrapperPath,
      type: "stdio",
      allowed_origins: [`chrome-extension://${extensionId}/`],
    };
    fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2));
    log(`Manifest written: ${manifestPath}`);

    // 5. Trigger reconnect via CDP — attach to the extension's service worker
    //    and call connectNative() directly. The first attempt failed (no manifest),
    //    but now the manifest is in place.
    const { sessionId } = await sendCDP("Target.attachToTarget", {
      targetId: swTarget.targetId,
      flatten: true,
    });
    // Reset port and reconnect
    await sendCDP("Runtime.evaluate", {
      expression: "state.port = null; connectNative();",
      returnByValue: true,
    }, sessionId);
    log("Triggered native host reconnect via CDP");

    // 6. Wait for the host socket (host creates it when started by Chrome)
    const socketPath = path.join(dataDir, "tabctl.sock");
    log("Waiting for host socket…");
    await waitForFile(socketPath, 15_000);
    log(`Socket ready: ${socketPath}`);

    // Brief pause for host to fully initialize
    await sleep(1000);

    // 7. Run CLI test scenarios
    const cliPath = path.resolve(__dirname, "..", "..", "cli", "tabctl.js");
    let passed = 0;
    let failed = 0;

    async function runTest(name: string, args: string[]): Promise<boolean> {
      const result = spawnSync(process.execPath, [cliPath, ...args], {
        env: {
          ...process.env,
          TABCTL_SOCKET: socketPath,
          XDG_CONFIG_HOME: configHome,
          XDG_STATE_HOME: stateHome,
        },
        encoding: "utf-8",
        timeout: 10_000,
      });

      const raw = (result.stdout || "").trim();
      try {
        const output = JSON.parse(raw);
        if (output.ok) {
          log(`  PASS: ${name}`);
          return true;
        }
        log(`  FAIL: ${name}: ${JSON.stringify(output.error ?? output)}`);
        return false;
      } catch {
        log(`  FAIL: ${name}: non-JSON output: ${raw.slice(0, 200)}`);
        if (result.stderr) log(`  stderr: ${result.stderr.slice(0, 200)}`);
        return false;
      }
    }

    // -- Tests ---------------------------------------------------------------

    if (await runTest("ping", ["ping", "--json"])) passed++;
    else failed++;

    if (await runTest("version", ["version", "--json"])) passed++;
    else failed++;

    if (await runTest("list --all", ["list", "--all", "--json"])) passed++;
    else failed++;

    if (
      await runTest("open", [
        "open",
        "--new-window",
        "--url", "https://example.com",
        "--url", "https://example.org",
        "--group", "TEST-Integration",
        "--json",
      ])
    ) passed++;
    else failed++;

    await sleep(2000);

    if (await runTest("list --window last-focused", ["list", "--window", "last-focused", "--json"]))
      passed++;
    else failed++;

    if (await runTest("group-list", ["group-list", "--window", "last-focused", "--json"]))
      passed++;
    else failed++;

    if (
      await runTest("close test group", [
        "close",
        "--group", "TEST-Integration",
        "--window", "last-focused",
        "--confirm",
        "--json",
      ])
    ) passed++;
    else failed++;

    if (await runTest("undo --latest", ["undo", "--latest", "--json"])) passed++;
    else failed++;

    // -- Summary -------------------------------------------------------------
    log("");
    log(`Results: ${passed} passed, ${failed} failed`);

    if (failed > 0) process.exitCode = 1;
  } finally {
    // Teardown
    if (chrome && !chrome.killed) {
      chrome.kill();
      await sleep(500);
    }
    if (manifestPath) {
      try { fs.unlinkSync(manifestPath); } catch {}
    }
    try { fs.rmSync(tmpDir, { recursive: true, force: true }); } catch {}

    clearTimeout(killTimer);
  }
}

main().catch((err) => {
  log(`FATAL: ${err.message}`);
  process.exit(2);
});
