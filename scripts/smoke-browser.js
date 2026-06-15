#!/usr/bin/env node
"use strict";

/**
 * Launches an isolated Edge (or Chrome) instance for smoke testing.
 *
 * The browser uses a temp user-data-dir and loads the local extension build.
 * tabctl setup runs first (writes native messaging manifest into the temp dir),
 * then the browser is started so it finds the manifest on first native-messaging
 * connection attempt.
 *
 * Emits one JSON line to stdout when the browser is connected and ready:
 *   {"ok":true,"profile":"smoke-<ts>","pid":<n>,"tmpDir":"<path>","extensionDir":"<path>"}
 *
 * Stays alive until killed. On SIGINT/SIGTERM: removes the tabctl profile,
 * kills the browser, and deletes the temp dir.
 *
 * Usage:
 *   node scripts/smoke-browser.js [extension-dir]
 *   TABCTL_BIN=./rust/target/debug/tabctl node scripts/smoke-browser.js dist/extension
 *
 * Environment variables:
 *   TABCTL_BIN               Path to tabctl binary (default: rust/target/debug/tabctl → tabctl)
 *   EDGE_PATH                Override browser binary path
 *   TABCTL_EXTENSION_DIR     Alternative to argv[2] for extension dir
 *   SMOKE_BROWSER_TIMEOUT_MS Ping timeout in ms (default: 30000)
 */

const fs = require("node:fs");
const path = require("node:path");
const os = require("node:os");
const { spawn, execFileSync } = require("node:child_process");

const defaultTmpRoot =
  process.platform === "win32" ? path.join(os.tmpdir(), "tctl-it") : path.join("/tmp", "tctl-it");
const smokeTmpRoot = process.env.TABCTL_TEST_TMP_ROOT || defaultTmpRoot;

function log(msg) {
  process.stderr.write(`[smoke-browser] ${msg}\n`);
}

function isNoisyBrowserLine(line) {
  return (
    line.includes("chrome/updater/") ||
    line.includes("EdgeUpdater") ||
    line.includes("crash_reporter") ||
    line.includes("crash_client") ||
    line.includes("Crashpad") ||
    line.includes("crashpad/") ||
    line.includes("component_update_utils") ||
    line.includes("registration_request.cc") ||
    line.includes("IsInternalAadJoinedMac") ||
    line.includes("UPDATER_PROCESS") ||
    line.includes("TensorFlow Lite XNNPACK delegate") ||
    line.includes("Trying to load the allocator multiple times") ||
    line.includes("Requested load of chrome://newtab/ for incorrect profile type") ||
    line.includes("task_policy_set TASK_CATEGORY_POLICY") ||
    line.includes("task_policy_set TASK_SUPPRESSION_POLICY") ||
    line.includes("Device is MDM enrolled") ||
    line.includes("No tenant ID in PSSO device cert") ||
    line.includes("Microsoft Corp tenant not confirmed") ||
    line.includes("returned 0")
  );
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function findBrowser() {
  if (process.env.EDGE_PATH) {
    return { bin: process.env.EDGE_PATH, name: "edge" };
  }
  const edgeCandidates =
    process.platform === "darwin"
      ? [
          "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
          "/Applications/Microsoft Edge Beta.app/Contents/MacOS/Microsoft Edge Beta",
          "/Applications/Microsoft Edge Canary.app/Contents/MacOS/Microsoft Edge Canary",
        ]
      : process.platform === "win32"
        ? [
            path.join(
              process.env.PROGRAMFILES || "C:\\Program Files",
              "Microsoft",
              "Edge",
              "Application",
              "msedge.exe"
            ),
            path.join(
              process.env["PROGRAMFILES(X86)"] || "C:\\Program Files (x86)",
              "Microsoft",
              "Edge",
              "Application",
              "msedge.exe"
            ),
          ]
        : ["/usr/bin/microsoft-edge", "/usr/bin/microsoft-edge-stable"];

  for (const c of edgeCandidates) {
    if (c && fs.existsSync(c)) return { bin: c, name: "edge" };
  }

  const chromeCandidates =
    process.platform === "darwin"
      ? ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"]
      : process.platform === "win32"
        ? [
            path.join(
              process.env.PROGRAMFILES || "C:\\Program Files",
              "Google",
              "Chrome",
              "Application",
              "chrome.exe"
            ),
          ]
        : ["/usr/bin/google-chrome", "/usr/bin/google-chrome-stable"];

  for (const c of chromeCandidates) {
    if (c && fs.existsSync(c)) return { bin: c, name: "chrome" };
  }

  throw new Error("Edge not found. Set EDGE_PATH to the browser binary.");
}

function findTabctl() {
  if (process.env.TABCTL_BIN) return process.env.TABCTL_BIN;
  const debugBin = path.join(process.cwd(), "rust", "target", "debug", "tabctl");
  if (fs.existsSync(debugBin)) return debugBin;
  return "tabctl";
}

let browserProc = null;
let tmpDir = null;
let configDir = null;
let dataDir = null;
let browserProfileDir = null;
let profileName = null;
let tabctlBin = null;
let smokeEnv = null;
let smokeCliEnv = null;
let shuttingDown = false;
let cdpWrite = null;
let cdpRead = null;
let cdpId = 0;
let cdpBuffer = "";
const pendingCdp = new Map();

function buildSmokeEnv() {
  if (!tmpDir || !configDir || !dataDir) {
    throw new Error("smoke directories are not initialized");
  }
  const env = {
    ...process.env,
    TABCTL_CONFIG_DIR: configDir,
    TABCTL_DATA_DIR: dataDir,
    TABCTL_STATE_DIR: dataDir,
    XDG_CONFIG_HOME: path.join(tmpDir, "xdg-config"),
    XDG_STATE_HOME: path.join(tmpDir, "xdg-state"),
  };
  delete env.TABCTL_PROFILE;
  delete env.TABCTL_TRANSPORT;
  delete env.TABCTL_TCP_PORT;
  delete env.TABCTL_AUTH_TOKEN;
  return env;
}

function buildSmokeCliEnv() {
  const env = buildSmokeEnv();
  delete env.TABCTL_DATA_DIR;
  return env;
}

function visibleBrowserRequested() {
  return process.env.SMOKE_BROWSER_VISIBLE === "1";
}

function initCDP(browserProcess) {
  cdpWrite = browserProcess.stdio[3];
  cdpRead = browserProcess.stdio[4];
  cdpRead.on("data", (chunk) => {
    cdpBuffer += chunk.toString("utf8");
    const parts = cdpBuffer.split("\0");
    cdpBuffer = parts.pop() || "";
    for (const part of parts) {
      if (!part.trim()) continue;
      try {
        const msg = JSON.parse(part);
        if (msg.id && pendingCdp.has(msg.id)) {
          const deferred = pendingCdp.get(msg.id);
          pendingCdp.delete(msg.id);
          if (msg.error) deferred.reject(new Error(msg.error.message || "Unknown CDP error"));
          else deferred.resolve(msg.result);
        }
      } catch {
        // Ignore malformed fragments while buffering.
      }
    }
  });
}

function sendCDP(method, params = {}, sessionId) {
  return new Promise((resolve, reject) => {
    const id = ++cdpId;
    pendingCdp.set(id, { resolve, reject });
    const payload = { id, method, params };
    if (sessionId) payload.sessionId = sessionId;
    cdpWrite.write(`${JSON.stringify(payload)}\0`);
  });
}

async function findServiceWorkerTarget(extensionId) {
  const targets = await sendCDP("Target.getTargets");
  return (targets.targetInfos || []).find(
    (target) => target.type === "service_worker" && String(target.url || "").includes(extensionId)
  );
}

async function attachServiceWorker(extensionId, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (browserProc.exitCode !== null) {
      throw new Error(`Browser exited during service-worker discovery (code ${browserProc.exitCode})`);
    }
    const swTarget = await findServiceWorkerTarget(extensionId);
    if (swTarget) {
      const attached = await sendCDP("Target.attachToTarget", {
        targetId: swTarget.targetId,
        flatten: true,
      });
      return attached.sessionId;
    }
    await sleep(250);
  }
  throw new Error("Extension service worker not found before timeout");
}

async function ensureNativePortConnected(sessionId, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const evaluation = await sendCDP(
      "Runtime.evaluate",
      {
        expression: `
          (() => {
            if (!self.__tabctl?.state?.port) {
              self.__tabctl?.connectNative?.();
              return { connected: false };
            }
            return { connected: true };
          })();
        `,
        returnByValue: true,
      },
      sessionId
    );
    if (evaluation?.exceptionDetails) {
      const detail =
        evaluation.exceptionDetails.exception?.description ||
        evaluation.exceptionDetails.text ||
        "unknown runtime exception";
      throw new Error(`native port probe failed: ${detail}`);
    }
    if (evaluation?.result?.value?.connected === true) {
      return;
    }
    await sleep(250);
  }
  throw new Error("native port did not connect before timeout");
}

async function shutdown(exitCode) {
  if (shuttingDown) return;
  shuttingDown = true;

  if (browserProc && browserProc.exitCode === null) {
    log("Stopping browser...");
    try {
      browserProc.kill("SIGTERM");
    } catch {
      // already gone
    }
    await sleep(800);
    if (browserProc.exitCode === null) {
      try {
        browserProc.kill("SIGKILL");
      } catch {
        // already gone
      }
    }
  }

  if (profileName && tabctlBin && smokeCliEnv) {
    try {
      execFileSync(tabctlBin, ["profile-remove", profileName], {
        stdio: "ignore",
        env: smokeCliEnv,
      });
      log(`Removed profile ${profileName}`);
    } catch {
      // best effort
    }
  }

  if (tmpDir) {
    if (process.env.SMOKE_KEEP_ARTIFACTS === "1") {
      log(`Preserved ${tmpDir} because SMOKE_KEEP_ARTIFACTS=1`);
    } else {
      try {
        fs.rmSync(tmpDir, { recursive: true, force: true });
        log(`Removed ${tmpDir}`);
      } catch {
        // best effort
      }
    }
  }

  process.exit(exitCode);
}

process.on("SIGINT", () => shutdown(0).catch(() => process.exit(0)));
process.on("SIGTERM", () => shutdown(0).catch(() => process.exit(0)));

async function main() {
  const extensionDirInput =
    process.argv[2] || process.env.TABCTL_EXTENSION_DIR || "dist/extension";

  if (!fs.existsSync(path.join(extensionDirInput, "manifest.json"))) {
    throw new Error(
      `Extension not found at ${extensionDirInput}. Run 'npm run build' first.`
    );
  }

  tabctlBin = findTabctl();
  const { bin: browserBin, name: browserName } = findBrowser();
  const ts = Date.now();
  profileName = `smoke-${ts}`;

  fs.mkdirSync(smokeTmpRoot, { recursive: true });
  tmpDir = fs.mkdtempSync(path.join(smokeTmpRoot, "smoke-"));
  configDir = path.join(tmpDir, "tabctl-config");
  dataDir = path.join(tmpDir, "tabctl-data");
  browserProfileDir = path.join(tmpDir, "browser-profile");
  for (const dir of [
    configDir,
    dataDir,
    browserProfileDir,
    path.join(tmpDir, "xdg-config"),
    path.join(tmpDir, "xdg-state"),
  ]) {
    fs.mkdirSync(dir, { recursive: true });
  }
  smokeEnv = buildSmokeEnv();
  smokeCliEnv = buildSmokeCliEnv();

  log(`tabctl:    ${tabctlBin}`);
  log(`browser:   ${browserBin} (${browserName})`);
  log(`extension: ${extensionDirInput}`);
  log(`profile:   ${profileName}`);
  log(`tmp root:  ${tmpDir}`);
  log(`config:    ${configDir}`);
  log(`data:      ${dataDir}`);
  log(`user data: ${browserProfileDir}`);

  // Step 1: Run tabctl setup BEFORE launching the browser.
  // This syncs the extension to the active dir, derives the extension ID from
  // that active path, and writes the native messaging manifest into browserProfileDir.
  // The browser will find the manifest there because it starts with --user-data-dir pointing at the same dir.
  log("Running tabctl setup...");
  let setupOutput;
  try {
    setupOutput = execFileSync(
      tabctlBin,
      [
        "setup",
        "--browser",
        browserName,
        "--extension-dir",
        extensionDirInput,
        "--user-data-dir",
        browserProfileDir,
        "--name",
        profileName,
        "--force",
        "--json",
        "--no-pretty",
      ],
      { encoding: "utf8", env: smokeEnv }
    );
  } catch (err) {
    throw new Error(
      `tabctl setup failed: ${err.stderr ? err.stderr.toString() : err.message}`
    );
  }

  // Extract the active extension dir path from setup JSON output.
  // tabctl setup syncs the local extension to the active dir and returns its path.
  // We launch the browser with --load-extension pointing at the same path setup used
  // to derive the extension ID, ensuring they match.
  let activeExtDir;
  let expectedExtensionId;
  try {
    const setupJson = JSON.parse(setupOutput);
    activeExtDir = setupJson?.data?.extensionSync?.activePath;
    expectedExtensionId = setupJson?.data?.extensionId;
  } catch {
    // ignore parse errors; will fail below
  }

  if (!activeExtDir) {
    throw new Error(
      "tabctl setup did not return an active extension path (data.extensionSync.activePath)"
    );
  }
  log(`Active extension: ${activeExtDir}`);

  // Step 2: Launch the browser with the isolated profile. Headless mode keeps
  // smoke windows out of the user's window manager; visible mode is debugging-only.
  const visible = visibleBrowserRequested();
  const browserArgs = visible
    ? [
        `--load-extension=${activeExtDir}`,
        `--user-data-dir=${browserProfileDir}`,
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-default-apps",
        "--new-window",
        "about:blank",
      ]
    : [
        "--headless=new",
        "--remote-debugging-pipe",
        "--enable-unsafe-extension-debugging",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-gpu",
        "--disable-background-timer-throttling",
        `--user-data-dir=${browserProfileDir}`,
      ];

  log(`Launching ${visible ? "visible" : "headless"} browser...`);
  browserProc = spawn(browserBin, browserArgs, {
    stdio: visible ? ["ignore", "ignore", "pipe"] : ["ignore", "pipe", "pipe", "pipe", "pipe"],
    detached: false,
    env: smokeEnv,
  });
  browserProc.stdout?.resume();

  let browserStderrBuffer = "";
  browserProc.stderr.on("data", (chunk) => {
    browserStderrBuffer += chunk.toString("utf8");
    const lines = browserStderrBuffer.split(/\r?\n/);
    browserStderrBuffer = lines.pop() || "";
    for (const line of lines) {
      const trimmed = line.trim();
      if (trimmed && !isNoisyBrowserLine(trimmed)) {
        log(`browser: ${trimmed}`);
      }
    }
  });

  browserProc.on("exit", (code) => {
    if (!shuttingDown) {
      log(`Browser exited unexpectedly (code ${code})`);
      shutdown(1).catch(() => process.exit(1));
    }
  });

  // Step 3: Load the extension in headless mode, then poll tabctl ping until it connects.
  const timeoutMs = parseInt(process.env.SMOKE_BROWSER_TIMEOUT_MS || "30000", 10);
  const deadline = Date.now() + timeoutMs;
  if (!visible) {
    await sleep(1500);
    if (browserProc.exitCode !== null) {
      throw new Error(`Browser exited early (code ${browserProc.exitCode})`);
    }
    initCDP(browserProc);
    const loadResult = await sendCDP("Extensions.loadUnpacked", { path: activeExtDir });
    const loadedExtensionId = loadResult && loadResult.id;
    if (!loadedExtensionId) {
      throw new Error("Failed to determine extension id from Extensions.loadUnpacked");
    }
    if (expectedExtensionId && loadedExtensionId !== expectedExtensionId) {
      throw new Error(
        `Loaded extension id ${loadedExtensionId} did not match setup extension id ${expectedExtensionId}`
      );
    }
    log(`Loaded headless extension: ${loadedExtensionId}`);
    const sessionId = await attachServiceWorker(loadedExtensionId, timeoutMs);
    await ensureNativePortConnected(sessionId, timeoutMs);
  }
  log(`Waiting for ping (${timeoutMs}ms timeout)...`);

  while (Date.now() < deadline) {
    if (browserProc.exitCode !== null) {
      throw new Error(`Browser exited early (code ${browserProc.exitCode})`);
    }
    try {
      execFileSync(tabctlBin, ["ping", "--profile", profileName], {
        stdio: "ignore",
        timeout: 3000,
        env: smokeCliEnv,
      });
      break;
    } catch {
      await sleep(1000);
    }
  }

  if (Date.now() >= deadline && browserProc.exitCode === null) {
    // One last attempt before giving up.
    try {
      execFileSync(tabctlBin, ["ping", "--profile", profileName], {
        stdio: "ignore",
        timeout: 3000,
        env: smokeCliEnv,
      });
    } catch {
      throw new Error(`Browser did not connect within ${timeoutMs}ms`);
    }
  }

  // Step 4: Emit ready signal and keep alive.
  const ready = {
    ok: true,
    profile: profileName,
    pid: browserProc.pid,
    tmpDir,
    configDir,
    dataDir,
    browserProfileDir,
    extensionDir: activeExtDir,
  };
  process.stdout.write(`${JSON.stringify(ready)}\n`);
  log("Ready. Waiting for shutdown signal...");

  await new Promise(() => {});
}

main().catch((err) => {
  log(`FATAL: ${err instanceof Error ? err.message : String(err)}`);
  shutdown(1).catch(() => process.exit(1));
});
