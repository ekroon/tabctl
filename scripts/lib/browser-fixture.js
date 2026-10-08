#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const path = require("node:path");
const os = require("node:os");
const { spawn, execFileSync } = require("node:child_process");
const { startBrowserControl } = require("./browser-control");

const defaultTmpRoot = "/tmp/tctl-it";
const smokeTmpRoot = process.env.TABCTL_TEST_TMP_ROOT || defaultTmpRoot;
let fixtureOptions = {};

function log(msg) {
  process.stderr.write(`[browser-fixture] ${msg}\n`);
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
  const candidates = {
    edge: [
      process.env.EDGE_PATH,
      "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    ],
    chrome: [
      process.env.CHROME_PATH,
      "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    ],
  };
  const requested = process.env.TABCTL_TEST_BROWSER || fixtureOptions.browser;
  if (requested && !Object.hasOwn(candidates, requested)) {
    throw new Error(`Unsupported test browser: ${requested}; use chrome or edge`);
  }
  for (const name of requested ? [requested] : ["edge", "chrome"]) {
    const bin = candidates[name].find((candidate) => candidate && fs.existsSync(candidate));
    if (bin) return { bin, name };
  }
  throw new Error(`${requested || "Chrome/Edge"} not found; set CHROME_PATH or EDGE_PATH`);
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
let normalManifests = [];
let browserControl = null;

function captureNormalManifests() {
  return ["Google/Chrome", "Microsoft Edge"].map((browser) => {
    const file = path.join(os.homedir(), "Library/Application Support", browser, "NativeMessagingHosts/com.erwinkroon.tabctl.json");
    return { file, content: fs.existsSync(file) ? fs.readFileSync(file) : null };
  });
}

function verifyNormalManifests() {
  for (const { file, content } of normalManifests) {
    const current = fs.existsSync(file) ? fs.readFileSync(file) : null;
    if ((content === null) !== (current === null) || (content && !content.equals(current))) {
      throw new Error(`Browser fixture changed a normal native-host manifest: ${file}`);
    }
  }
}

function buildSmokeEnv() {
  if (!tmpDir || !configDir || !dataDir) {
    throw new Error("smoke directories are not initialized");
  }
  const env = {
    ...Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith("TABCTL_"))),
    TABCTL_CONFIG_DIR: configDir,
    TABCTL_DATA_DIR: dataDir,
    TABCTL_STATE_DIR: dataDir,
    XDG_CONFIG_HOME: path.join(tmpDir, "c"),
    XDG_STATE_HOME: path.join(tmpDir, "s"),
    TABCTL_AUTO_SYNC_MODE: "off",
    TABCTL_SETUP_FETCH_EXTENSION: "0",
    HOME: tmpDir,
  };
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
      } catch (error) {
        log(`Malformed CDP response: ${error.message}`);
      }
    }
  });
}

function sendCDP(method, params = {}, sessionId) {
  return new Promise((resolve, reject) => {
    const id = ++cdpId;
    const timeout = setTimeout(() => {
      pendingCdp.delete(id);
      reject(new Error(`CDP ${method} timed out`));
    }, 30_000);
    pendingCdp.set(id, {
      resolve: (value) => { clearTimeout(timeout); resolve(value); },
      reject: (error) => { clearTimeout(timeout); reject(error); },
    });
    const payload = { id, method, params };
    if (sessionId) payload.sessionId = sessionId;
    cdpWrite.write(`${JSON.stringify(payload)}\0`);
  });
}

async function shutdown(exitCode) {
  if (shuttingDown) return;
  shuttingDown = true;
  if (browserControl) await browserControl.close();
  for (const request of pendingCdp.values()) {
    request.reject(new Error("Browser fixture is shutting down"));
  }
  pendingCdp.clear();

  if (browserProc && browserProc.exitCode === null) {
    log("Stopping browser...");
    browserProc.kill("SIGTERM");
    await sleep(800);
    if (browserProc.exitCode === null) {
      browserProc.kill("SIGKILL");
      await Promise.race([
        new Promise((resolve) => browserProc.once("exit", resolve)),
        sleep(5000).then(() => {
          if (browserProc.exitCode === null && browserProc.signalCode === null) {
            throw new Error("Browser did not exit after SIGKILL");
          }
        }),
      ]);
    }
  }

  if (profileName && tabctlBin && smokeCliEnv) {
    try {
      execFileSync(tabctlBin, ["profile-remove", profileName], {
        stdio: "ignore",
        env: smokeCliEnv,
        timeout: 5000,
      });
      log(`Removed profile ${profileName}`);
    } catch (error) {
      log(`Profile cleanup failed: ${error.message}`);
      exitCode = 1;
    }
  }

  if (tmpDir) {
    if (process.env.SMOKE_KEEP_ARTIFACTS === "1") {
      log(`Preserved ${tmpDir} because SMOKE_KEEP_ARTIFACTS=1`);
    } else {
      try {
        fs.rmSync(tmpDir, { recursive: true, force: true });
        log(`Removed ${tmpDir}`);
      } catch (error) {
        log(`Fixture cleanup failed: ${error.message}`);
        exitCode = 1;
      }
    }
  }

  try {
    verifyNormalManifests();
  } catch (error) {
    log(error.message);
    exitCode = 1;
  }
  process.exit(exitCode);
}

async function main() {
  if (process.platform !== "darwin") {
    throw new Error("tabctl browser fixtures require macOS");
  }
  normalManifests = captureNormalManifests();
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
  profileName = fixtureOptions.profile || `smoke-${ts}`;

  fs.mkdirSync(smokeTmpRoot, { recursive: true });
  const candidateRoot = fixtureOptions.root || fs.mkdtempSync(path.join(smokeTmpRoot, "smoke-"));
  if (
    path.dirname(path.resolve(candidateRoot)) !== path.resolve(smokeTmpRoot) ||
    fs.lstatSync(candidateRoot).isSymbolicLink() ||
    fs.readdirSync(candidateRoot).length !== 0
  ) {
    throw new Error("Browser fixture requires an empty direct child of TABCTL_TEST_TMP_ROOT");
  }
  tmpDir = candidateRoot;
  configDir = path.join(tmpDir, "c", "tabctl");
  dataDir = path.join(tmpDir, "s", "tabctl");
  browserProfileDir = path.join(tmpDir, "browser-profile");
  for (const dir of [
    configDir,
    dataDir,
    browserProfileDir,
    path.join(tmpDir, "c"),
    path.join(tmpDir, "s"),
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
      { encoding: "utf8", env: smokeEnv, timeout: 30000 }
    );
    verifyNormalManifests();
  } catch (err) {
    throw new Error(
      `tabctl setup failed: ${err.stderr ? err.stderr.toString() : err.message}`
    );
  }

  // Extract the active extension dir path from setup JSON output.
  // tabctl setup syncs the local extension to the active dir and returns its path.
  // We launch the browser with --load-extension pointing at the same path setup used
  // to derive the extension ID, ensuring they match.
  const setupJson = JSON.parse(setupOutput);
  const activeExtDir = setupJson?.data?.extensionSync?.activePath;
  const expectedExtensionId = setupJson?.data?.extensionId;

  if (!activeExtDir) {
    throw new Error(
      "tabctl setup did not return an active extension path (data.extensionSync.activePath)"
    );
  }
  if (!/^[a-p]{32}$/.test(expectedExtensionId || "")) {
    throw new Error("tabctl setup did not return a valid extension ID");
  }
  const manifestPath = path.join(browserProfileDir, "NativeMessagingHosts/com.erwinkroon.tabctl.json");
  const nativeManifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
  for (const managedPath of [activeExtDir, nativeManifest.path]) {
    const relative = path.relative(fs.realpathSync(tmpDir), fs.realpathSync(managedPath));
    if (relative.startsWith("..") || path.isAbsolute(relative)) {
      throw new Error(`Setup escaped the browser fixture: ${managedPath}`);
    }
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
  // A disposable browser profile must never ask to reset the user's macOS keychain.
  browserArgs.push("--use-mock-keychain", "--password-store=basic");

  log(`Launching ${visible ? "visible" : "headless"} browser...`);
  browserProc = spawn(browserBin, browserArgs, {
    stdio: visible ? ["ignore", "ignore", "pipe"] : ["ignore", "pipe", "pipe", "pipe", "pipe"],
    detached: false,
    env: { ...smokeEnv, HOME: os.homedir() },
  });
  browserProc.stdout?.resume();
  browserProc.on("error", (error) => {
    log(`Browser process failed: ${error.message}`);
    shutdown(1).catch((shutdownError) => {
      log(shutdownError.message);
      process.exit(1);
    });
  });

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
  }
  log(`Waiting for ping (${timeoutMs}ms timeout)...`);

  let lastPingError;
  while (Date.now() < deadline) {
    if (browserProc.exitCode !== null) {
      throw new Error(`Browser exited early (code ${browserProc.exitCode})`);
    }
    try {
      const output = execFileSync(tabctlBin, ["ping", "--profile", profileName, "--json", "--no-pretty"], {
        encoding: "utf8",
        timeout: 3000,
        env: smokeCliEnv,
      });
      const response = JSON.parse(output);
      const ping = response.data || response;
      if (ping.runtimeId !== expectedExtensionId) {
        throw new Error(`Ping returned unexpected runtimeId: ${ping.runtimeId}`);
      }
      break;
    } catch (error) {
      lastPingError = error;
      await sleep(1000);
    }
  }

  if (Date.now() >= deadline && browserProc.exitCode === null) {
    throw new Error(`Browser did not connect within ${timeoutMs}ms: ${lastPingError?.message || "no response"}`);
  }
  if (!visible) {
    browserControl = await startBrowserControl(tmpDir, expectedExtensionId, sendCDP);
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
    extensionId: expectedExtensionId,
    browser: browserName,
  };
  process.stdout.write(`${JSON.stringify(ready)}\n`);
  log("Ready. Waiting for shutdown signal...");

  await new Promise(() => {});
}

function run(options = {}) {
  fixtureOptions = options;
  process.on("SIGINT", () => shutdown(0).catch(() => process.exit(1)));
  process.on("SIGTERM", () => shutdown(0).catch(() => process.exit(1)));
  main().catch((err) => {
    log(`FATAL: ${err instanceof Error ? err.message : String(err)}`);
    shutdown(1).catch(() => process.exit(1));
  });
}

module.exports = { run };
