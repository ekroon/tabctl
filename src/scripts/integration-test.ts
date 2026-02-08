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
import { spawnSync, type ChildProcess } from "node:child_process";

import {
  sleep,
  log,
  waitForFile,
  findChrome,
  initCDP,
  sendCDP,
  launchChrome,
  loadExtension,
  attachServiceWorker,
} from "./lib/integration-cdp";

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
    const extensionDir = path.resolve(__dirname, "..", "extension");
    chrome = launchChrome(chromePath, userDataDir);

    chrome.on("exit", (code) => {
      log(`Chrome exited (code ${code})`);
    });

    // Drain stdout and capture stderr for diagnostics
    chrome.stdout?.resume();
    const stderrBuf = { value: "" };
    chrome.stderr?.on("data", (chunk: Buffer) => {
      stderrBuf.value += chunk.toString();
    });

    // Wait for Chrome to initialize
    await sleep(3000);

    // Verify Chrome is still running
    if (chrome.exitCode !== null) {
      throw new Error(`Chrome exited prematurely with code ${chrome.exitCode}`);
    }

    initCDP(chrome);

    // 3. Write native messaging manifest with a placeholder. We need the
    //    extension ID for allowed_origins, but we also need the manifest ready
    //    quickly so connectNative() can succeed.
    const manifestDir = path.join(userDataDir, "NativeMessagingHosts");
    fs.mkdirSync(manifestDir, { recursive: true });
    const hostPath = path.resolve(__dirname, "..", "host", "host.js");
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

    // 4. Load extension to discover its ID, write manifest, then find service worker.
    const extensionId = await loadExtension(extensionDir);

    // Write the manifest with correct allowed_origins immediately after getting
    // the extension ID — connectNative() needs it ready before the service
    // worker is discovered and attached.
    const manifest = {
      name: DEFAULT_HOST_NAME,
      description: "tabctl integration test",
      path: wrapperPath,
      type: "stdio",
      allowed_origins: [`chrome-extension://${extensionId}/`],
    };
    fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2));
    log(`Manifest written: ${manifestPath}`);

    // Now find and attach to the service worker.
    const sessionId = await attachServiceWorker(extensionId, chrome, stderrBuf);

    // 5. Reset port and reconnect native host.
    await sendCDP("Runtime.evaluate", {
      expression: "self.__tabctl.state.port = null; self.__tabctl.connectNative();",
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
    const cliPath = path.resolve(__dirname, "..", "cli", "tabctl.js");
    let passed = 0;
    let failed = 0;

    const cliEnv = {
      ...process.env,
      TABCTL_SOCKET: socketPath,
      XDG_CONFIG_HOME: configHome,
      XDG_STATE_HOME: stateHome,
    };

    async function runTest(name: string, args: string[]): Promise<boolean> {
      const result = spawnSync(process.execPath, [cliPath, ...args], {
        env: cliEnv,
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

    // -- Reload test ---------------------------------------------------------
    // Validates the upgrade chain: extension reload → new SW → reconnect → ping
    log("");
    log("--- Reload cycle test ---");

    // 1. Send "reload" action via CLI → host → extension
    const reloadResult = spawnSync(process.execPath, [cliPath, "reload", "--json"], {
      env: cliEnv,
      encoding: "utf-8",
      timeout: 10_000,
    });
    const reloadRaw = (reloadResult.stdout || "").trim();
    let reloadOk = false;
    try {
      const reloadOutput = JSON.parse(reloadRaw);
      reloadOk = reloadOutput.ok && reloadOutput.data?.reloading;
    } catch {}

    if (reloadOk) {
      log("  PASS: reload action accepted");
      passed++;
    } else {
      log(`  FAIL: reload action: ${reloadRaw.slice(0, 200)}`);
      failed++;
    }

    // 2. Wait for extension to reload and host to restart
    //    The old host exits (extension killed the native port).
    //    The extension reloads → onInstalled → connectNative() → new host starts.
    await sleep(3000);

    // 3. Re-attach to the new service worker (old session is dead)
    log("  Looking for new service worker after reload...");
    try {
      const newSessionId = await attachServiceWorker(extensionId, chrome, stderrBuf);
      log(`  New SW session: ${newSessionId}`);

      // Force reconnect in case onInstalled race
      await sendCDP("Runtime.evaluate", {
        expression: "self.__tabctl.state.port = null; self.__tabctl.connectNative();",
        returnByValue: true,
      }, newSessionId);

      // 4. Wait for new socket
      log("  Waiting for socket after reload...");
      await waitForFile(socketPath, 15_000);
      await sleep(1000);

      // 5. Verify ping works through the reloaded extension
      if (await runTest("ping after reload", ["ping", "--json"])) passed++;
      else failed++;

      // 6. Verify version is consistent
      if (await runTest("version after reload", ["version", "--json"])) passed++;
      else failed++;
    } catch (e) {
      log(`  FAIL: reload cycle: ${(e as Error).message}`);
      failed += 2; // count ping + version as failed
    }

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
