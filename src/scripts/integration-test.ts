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
import net from "net";
import os from "os";
import path from "path";
import { execSync, spawnSync, type ChildProcess } from "node:child_process";
import { resolveSocketPath } from "../shared/config";

import {
  sleep,
  log,
  waitForSocket,
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

    // 1b. Build the host wrapper.
    const hostPath = path.resolve(__dirname, "..", "host", "host.js");
    let wrapperPath: string;

    if (process.platform === "win32") {
      // Use the Go launcher binary that proxies binary stdin/stdout between
      // Chrome and Node. Try the prebuilt binary from the platform package first,
      // fall back to compiling from source (requires Go).
      const exePath = path.join(dataDir, "tabctl-host.exe");
      const cfgPath = path.join(dataDir, "host-launcher.cfg");

      // Write config file: node path, host path, env vars
      fs.writeFileSync(cfgPath, [
        process.execPath,
        hostPath,
        `XDG_CONFIG_HOME=${configHome}`,
        `XDG_STATE_HOME=${stateHome}`,
        "",
      ].join("\r\n"));

      let prebuilt: string | undefined;
      try {
        prebuilt = require.resolve("tabctl-win32-x64/tabctl-host.exe");
      } catch { /* not installed */ }

      if (prebuilt) {
        fs.copyFileSync(prebuilt, exePath);
        log("Using prebuilt native host launcher");
      } else {
        const launcherDir = path.resolve(__dirname, "..", "host", "launcher");
        log("Compiling native host launcher...");
        const compileResult = spawnSync("go", ["build", "-o", exePath, "."], {
          cwd: launcherDir,
          encoding: "utf-8",
          timeout: 60_000,
          env: { ...process.env, CGO_ENABLED: "0" },
        });
        if (compileResult.status !== 0) {
          throw new Error(`Failed to compile host launcher:\n${compileResult.stdout}\n${compileResult.stderr}`);
        }
        log("Native host launcher compiled");
      }
      wrapperPath = exePath;
    } else {
      wrapperPath = path.join(dataDir, "tabctl-host.sh");
      const wrapper = [
        "#!/usr/bin/env bash",
        "set -euo pipefail",
        `export XDG_CONFIG_HOME="${configHome}"`,
        `export XDG_STATE_HOME="${stateHome}"`,
        `exec "${process.execPath}" "${hostPath}"`,
        "",
      ].join("\n");
      fs.writeFileSync(wrapperPath, wrapper, { mode: 0o700 });
    }

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

    // On Windows, Chrome uses the registry; on macOS/Linux, a directory-based manifest.
    const manifestDir = process.platform === "win32"
      ? dataDir  // Just store the manifest file somewhere; registry points to it
      : path.join(userDataDir, "NativeMessagingHosts");
    fs.mkdirSync(manifestDir, { recursive: true });
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

    // On Windows, register the manifest in the registry
    if (process.platform === "win32") {
      const regKey = `HKCU\\Software\\Google\\Chrome\\NativeMessagingHosts\\${DEFAULT_HOST_NAME}`;
      execSync(`reg add "${regKey}" /ve /t REG_SZ /d "${manifestPath}" /f`, { stdio: "pipe" });
      log(`Registry key written: ${regKey}`);
    }

    // Now find and attach to the service worker.
    const sessionId = await attachServiceWorker(extensionId, chrome, stderrBuf);

    // 5. Disconnect existing port (if any) and reconnect native host.
    // On Windows, named pipes can't be re-bound while the previous host holds them,
    // so we must disconnect first to let the previous host exit.
    await sendCDP("Runtime.evaluate", {
      expression: `
        if (self.__tabctl.state.port) {
          try { self.__tabctl.state.port.disconnect(); } catch(e) {}
        }
        self.__tabctl.state.port = null;
      `,
      returnByValue: true,
    }, sessionId);
    // Give the previous host time to exit and release the named pipe
    await sleep(process.platform === "win32" ? 1500 : 200);
    await sendCDP("Runtime.evaluate", {
      expression: "self.__tabctl.connectNative();",
      returnByValue: true,
    }, sessionId);
    log("Triggered native host reconnect via CDP");

    // 6. Wait for the host socket (host creates it when started by Chrome)
    const socketPath = resolveSocketPath(dataDir);
    log("Waiting for host socket…");
    await waitForSocket(socketPath, 15_000);
    log(`Socket ready: ${socketPath}`);

    // Brief pause for host to fully initialize
    await sleep(1000);

    // Check Chrome is still alive
    log(`Chrome process alive: ${chrome.exitCode === null}, pid=${chrome.pid}`);

    // Diagnostic: check extension port state via CDP
    const portCheck = await sendCDP("Runtime.evaluate", {
      expression: "JSON.stringify({ portExists: !!self.__tabctl.state.port, portName: self.__tabctl.state.port?.name ?? null })",
      returnByValue: true,
    }, sessionId);
    log(`Extension port state: ${JSON.stringify(portCheck?.result?.value ?? portCheck)}`);

    // If port is null, try to get last error info
    if (portCheck?.result?.value) {
      try {
        const parsed = JSON.parse(portCheck.result.value);
        if (!parsed.portExists) {
          log("Extension port is NULL — native messaging disconnected!");
          // Try reconnecting and check lastError
          const reconnectCheck = await sendCDP("Runtime.evaluate", {
            expression: `
              (async () => {
                try {
                  const port = chrome.runtime.connectNative("${DEFAULT_HOST_NAME}");
                  const err = chrome.runtime.lastError;
                  return JSON.stringify({ connected: !!port, lastError: err?.message ?? null });
                } catch(e) {
                  return JSON.stringify({ connected: false, error: e.message });
                }
              })()
            `,
            awaitPromise: true,
            returnByValue: true,
          }, sessionId);
          log(`Reconnect attempt: ${JSON.stringify(reconnectCheck?.result?.value ?? reconnectCheck)}`);
          await sleep(2000);
          // Check port state again
          const portCheck2 = await sendCDP("Runtime.evaluate", {
            expression: "JSON.stringify({ portExists: !!self.__tabctl.state.port, portName: self.__tabctl.state.port?.name ?? null })",
            returnByValue: true,
          }, sessionId);
          log(`Extension port state after reconnect: ${JSON.stringify(portCheck2?.result?.value ?? portCheck2)}`);
          // Wait for new socket
          await waitForSocket(socketPath, 15_000);
          log(`Socket ready after reconnect: ${socketPath}`);
          await sleep(1000);
        }
      } catch {}
    }

    // Diagnostic: raw socket ping to verify host is reachable
    const diagResult = await new Promise<string>((resolve) => {
      const timer = setTimeout(() => resolve("TIMEOUT"), 5000);
      const sock = net.createConnection(socketPath);
      sock.on("connect", () => {
        sock.write(JSON.stringify({ id: "diag-1", action: "version" }) + "\n");
      });
      sock.on("data", (d) => {
        clearTimeout(timer);
        sock.end();
        resolve(d.toString().trim());
      });
      sock.on("error", (e) => {
        clearTimeout(timer);
        resolve(`ERROR: ${e.message}`);
      });
    });
    log(`Diagnostic socket version: ${diagResult.slice(0, 200)}`);

    // Diagnostic: raw socket ping (requires extension roundtrip)
    const diagPing = await new Promise<string>((resolve) => {
      const timer = setTimeout(() => resolve("TIMEOUT"), 8000);
      const sock = net.createConnection(socketPath);
      sock.on("connect", () => {
        sock.write(JSON.stringify({ id: "diag-2", action: "ping" }) + "\n");
      });
      sock.on("data", (d) => {
        clearTimeout(timer);
        sock.end();
        resolve(d.toString().trim());
      });
      sock.on("error", (e) => {
        clearTimeout(timer);
        resolve(`ERROR: ${e.message}`);
      });
    });
    log(`Diagnostic socket ping: ${diagPing.slice(0, 200)}`);

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
    log(`CLI env: TABCTL_SOCKET=${socketPath}`);

    function runCli(args: string[]): { ok: boolean; data?: Record<string, unknown>; raw: string } {
      const result = spawnSync(process.execPath, [cliPath, ...args], {
        env: cliEnv,
        encoding: "utf-8",
        timeout: 10_000,
      });
      const raw = (result.stdout || "").trim();
      try {
        const output = JSON.parse(raw);
        return { ok: !!output.ok, data: output.data ?? output, raw };
      } catch {
        return { ok: false, raw };
      }
    }

    async function runTest(name: string, args: string[]): Promise<boolean> {
      const { ok, data, raw } = runCli(args);
      if (ok) {
        log(`  PASS: ${name}`);
        return true;
      }
      log(`  FAIL: ${name}: ${raw.slice(0, 200)}`);
      return false;
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

    // -- Group reuse and duplicate skipping tests ----------------------------
    // The open test above created a TEST-Integration group with example.com
    // and example.org. Now test that re-opening the same group reuses it
    // and skips duplicates.

    {
      const name = "open --group reuse skips duplicates";
      const result = runCli([
        "open",
        "--url", "https://example.com",       // duplicate
        "--url", "https://example.net",        // new
        "--group", "TEST-Integration",
        "--window", "last-focused",
        "--json",
      ]);
      if (result.ok) {
        const data = result.data as Record<string, unknown>;
        const summary = data?.summary as Record<string, unknown> | undefined;
        const skipped = data?.skipped as Array<Record<string, unknown>> | undefined;
        if (
          summary &&
          summary.createdTabs === 1 &&
          summary.skippedUrls === 1 &&
          summary.grouped === true &&
          skipped?.length === 1 &&
          skipped[0].reason === "duplicate"
        ) {
          log(`  PASS: ${name}`);
          passed++;
        } else {
          log(`  FAIL: ${name}: unexpected summary ${JSON.stringify(summary)} skipped ${JSON.stringify(skipped)}`);
          failed++;
        }
      } else {
        log(`  FAIL: ${name}: ${result.raw.slice(0, 200)}`);
        failed++;
      }
    }

    await sleep(1000);

    {
      const name = "open --group --allow-duplicates opens duplicates";
      const result = runCli([
        "open",
        "--url", "https://example.com",
        "--group", "TEST-Integration",
        "--window", "last-focused",
        "--allow-duplicates",
        "--json",
      ]);
      if (result.ok) {
        const data = result.data as Record<string, unknown>;
        const summary = data?.summary as Record<string, unknown> | undefined;
        if (
          summary &&
          summary.createdTabs === 1 &&
          summary.skippedUrls === 0
        ) {
          log(`  PASS: ${name}`);
          passed++;
        } else {
          log(`  FAIL: ${name}: unexpected summary ${JSON.stringify(summary)}`);
          failed++;
        }
      } else {
        log(`  FAIL: ${name}: ${result.raw.slice(0, 200)}`);
        failed++;
      }
    }

    await sleep(1000);

    {
      const name = "open --group --new-group creates separate group";
      const result = runCli([
        "open",
        "--url", "https://example.com",
        "--group", "TEST-Integration",
        "--window", "last-focused",
        "--new-group",
        "--json",
      ]);
      if (result.ok) {
        const data = result.data as Record<string, unknown>;
        const summary = data?.summary as Record<string, unknown> | undefined;
        if (
          summary &&
          summary.createdTabs === 1 &&
          summary.grouped === true
        ) {
          log(`  PASS: ${name}`);
          passed++;
        } else {
          log(`  FAIL: ${name}: unexpected summary ${JSON.stringify(summary)}`);
          failed++;
        }
      } else {
        log(`  FAIL: ${name}: ${result.raw.slice(0, 200)}`);
        failed++;
      }
    }

    await sleep(1000);

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
      try {
        await Promise.race([
          (async () => {
            await sendCDP("Runtime.evaluate", {
              expression: `
                if (self.__tabctl.state.port) {
                  try { self.__tabctl.state.port.disconnect(); } catch(e) {}
                }
                self.__tabctl.state.port = null;
              `,
              returnByValue: true,
            }, newSessionId);
            // Give the previous host time to exit and release the named pipe
            await sleep(process.platform === "win32" ? 1500 : 200);
            await sendCDP("Runtime.evaluate", {
              expression: "self.__tabctl.connectNative();",
              returnByValue: true,
            }, newSessionId);
          })(),
          sleep(8000).then(() => { throw new Error("CDP reconnect timed out"); }),
        ]);
      } catch (e) {
        log(`  Warning: reconnect CDP: ${(e as Error).message}`);
      }

      // 4. Wait for new socket
      log("  Waiting for socket after reload...");
      await waitForSocket(socketPath, 15_000);
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
    // Clean up registry key on Windows
    if (process.platform === "win32") {
      try {
        execSync(`reg delete "HKCU\\Software\\Google\\Chrome\\NativeMessagingHosts\\${DEFAULT_HOST_NAME}" /f`, { stdio: "pipe" });
      } catch {}
    }
    try { fs.rmSync(tmpDir, { recursive: true, force: true }); } catch {}

    clearTimeout(killTimer);
  }
}

main().catch((err) => {
  log(`FATAL: ${err.message}`);
  process.exit(2);
});
