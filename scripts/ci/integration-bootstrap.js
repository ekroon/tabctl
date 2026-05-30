#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const path = require("node:path");
const { spawn } = require("node:child_process");
const { execFileSync } = require("node:child_process");

const HOST_NAME = process.env.TABCTL_HOST_NAME || "com.erwinkroon.tabctl";

function log(message) {
  process.stderr.write(`[integration-bootstrap] ${message}\n`);
}

function isNoisyChromeLine(line) {
  return (
    line.includes("chrome/updater/") ||
    line.includes("EdgeUpdater") ||
    line.includes("crash_reporter") ||
    line.includes("crash_client") ||
    line.includes("Crashpad") ||
    line.includes("crashpad/") ||
    line.includes("component_update_utils") ||
    line.includes("registration_request.cc") ||
    line.includes("event_history.cc") ||
    line.includes("UPDATER_PROCESS") ||
    line.includes("UpdaterMain") ||
    line.includes("TensorFlow Lite XNNPACK delegate") ||
    line.includes("Trying to load the allocator multiple times") ||
    line.includes("Requested load of chrome://newtab/ for incorrect profile type") ||
    line.includes("task_policy_set TASK_CATEGORY_POLICY") ||
    line.includes("task_policy_set TASK_SUPPRESSION_POLICY") ||
    line.includes("Device is MDM enrolled") ||
    line.includes("No tenant ID in PSSO device cert") ||
    line.includes("Microsoft Corp tenant not confirmed") ||
    line.includes("IsInternalAadJoinedMac") ||
    line.includes("Failed to write history event: logging not initialized") ||
    line.includes("Shutdown: 0")
  );
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function testScratchRoot() {
  const root = process.env.TABCTL_BOOTSTRAP_TMP_ROOT || path.join("/tmp", "tctl-it");
  fs.mkdirSync(root, { recursive: true });
  return root;
}

function findChrome() {
  if (process.env.CHROME_PATH) return process.env.CHROME_PATH;
  const candidates = process.platform === "win32"
    ? [
        path.join(process.env.PROGRAMFILES || "C:\\Program Files", "Google", "Chrome", "Application", "chrome.exe"),
        path.join(process.env["PROGRAMFILES(X86)"] || "C:\\Program Files (x86)", "Google", "Chrome", "Application", "chrome.exe"),
        path.join(process.env.LOCALAPPDATA || "", "Google", "Chrome", "Application", "chrome.exe"),
      ]
    : process.platform === "linux"
      ? ["/usr/bin/google-chrome", "/usr/bin/google-chrome-stable", "/usr/bin/chromium-browser", "/usr/bin/chromium"]
      : [
          "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
          "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
        ];

  for (const candidate of candidates) {
    if (candidate && fs.existsSync(candidate)) {
      return candidate;
    }
  }
  throw new Error("Chrome not found. Set CHROME_PATH.");
}

function launchChrome(chromePath, userDataDir) {
  const args = [
    "--headless=new",
    "--remote-debugging-pipe",
    "--enable-unsafe-extension-debugging",
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-gpu",
    "--disable-background-timer-throttling",
    `--user-data-dir=${userDataDir}`,
  ];
  if (process.platform === "linux") {
    args.push("--no-sandbox");
  }
  return spawn(chromePath, args, {
    stdio: ["ignore", "pipe", "pipe", "pipe", "pipe"],
  });
}

let chrome = null;
let tmpDir = null;
let shuttingDown = false;
let manifestPath = null;
let registryKey = null;

let cdpWrite = null;
let cdpRead = null;
let cdpId = 0;
let cdpBuffer = "";
const pendingCdp = new Map();

function initCDP(chromeProcess) {
  cdpWrite = chromeProcess.stdio[3];
  cdpRead = chromeProcess.stdio[4];
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

async function attachServiceWorker(extensionId, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (chrome.exitCode !== null) {
      throw new Error(`Chrome exited during service-worker discovery (code ${chrome.exitCode})`);
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

async function findServiceWorkerTarget(extensionId) {
  const targets = await sendCDP("Target.getTargets");
  return (targets.targetInfos || []).find(
    (target) => target.type === "service_worker" && String(target.url || "").includes(extensionId)
  );
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
    const connected = evaluation?.result?.value?.connected === true;
    if (connected) {
      return;
    }
    await sleep(250);
  }
  throw new Error("native port did not connect before timeout");
}

async function shutdown(exitCode) {
  if (shuttingDown) return;
  shuttingDown = true;
  if (chrome && !chrome.killed) {
    chrome.kill();
    await sleep(300);
  }
  if (registryKey && process.platform === "win32") {
    try {
      execFileSync("reg", ["delete", registryKey, "/f"], { stdio: "ignore" });
    } catch {
      // Best effort cleanup.
    }
  }
  if (tmpDir) {
    try {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    } catch {
      // Best effort cleanup.
    }
  }
  process.exit(exitCode);
}

process.on("SIGINT", () => {
  shutdown(0).catch(() => process.exit(0));
});
process.on("SIGTERM", () => {
  shutdown(0).catch(() => process.exit(0));
});

async function main() {
  const extensionDirInput = process.env.TABCTL_EXTENSION_DIR;
  if (!extensionDirInput) {
    throw new Error("TABCTL_EXTENSION_DIR is required");
  }
  const extensionDir = fs.realpathSync(extensionDirInput);
  if (!fs.existsSync(path.join(extensionDir, "manifest.json"))) {
    throw new Error(`Extension manifest not found at ${path.join(extensionDir, "manifest.json")}`);
  }

  const timeoutMs = Number.parseInt(process.env.TABCTL_BOOTSTRAP_TIMEOUT_MS || "30000", 10);
  const hostWrapper = process.env.TABCTL_HOST_WRAPPER;
  if (!hostWrapper) {
    throw new Error("TABCTL_HOST_WRAPPER is required");
  }
  if (!fs.existsSync(hostWrapper)) {
    throw new Error(`TABCTL_HOST_WRAPPER does not exist at ${hostWrapper}`);
  }
  const chromePath = findChrome();
  log(`Chrome: ${chromePath}`);
  log(`Extension: ${extensionDir}`);

  tmpDir = fs.mkdtempSync(path.join(testScratchRoot(), "b-"));
  const userDataDir = path.join(tmpDir, "chrome-profile");
  fs.mkdirSync(userDataDir, { recursive: true });

  chrome = launchChrome(chromePath, userDataDir);
  chrome.stdout?.resume();
  let chromeStderrBuffer = "";
  chrome.stderr?.on("data", (chunk) => {
    chromeStderrBuffer += chunk.toString("utf8");
    const lines = chromeStderrBuffer.split(/\r?\n/);
    chromeStderrBuffer = lines.pop() || "";
    for (const line of lines) {
      const trimmed = line.trim();
      if (trimmed && !isNoisyChromeLine(trimmed)) {
        log(`chrome-stderr: ${trimmed}`);
      }
    }
  });
  chrome.on("exit", (code) => {
    if (!shuttingDown) {
      log(`Chrome exited unexpectedly (code ${code})`);
      shutdown(1).catch(() => process.exit(1));
    }
  });

  await sleep(1500);
  if (chrome.exitCode !== null) {
    throw new Error(`Chrome exited early with code ${chrome.exitCode}`);
  }

  initCDP(chrome);
  const loadResult = await sendCDP("Extensions.loadUnpacked", { path: extensionDir });
  const extensionId = loadResult && loadResult.id;
  if (!extensionId) {
    throw new Error("Failed to determine extension id from Extensions.loadUnpacked");
  }

  const manifest = {
    name: HOST_NAME,
    description: "tabctl integration bootstrap",
    path: hostWrapper,
    type: "stdio",
    allowed_origins: [`chrome-extension://${extensionId}/`],
  };
  if (process.platform === "win32") {
    manifestPath = path.join(tmpDir, `${HOST_NAME}.json`);
    registryKey = `HKCU\\Software\\Google\\Chrome\\NativeMessagingHosts\\${HOST_NAME}`;
    fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2), "utf8");
    execFileSync(
      "reg",
      ["add", registryKey, "/ve", "/t", "REG_SZ", "/d", manifestPath, "/f"],
      { stdio: "ignore" }
    );
  } else {
    const manifestDir = path.join(userDataDir, "NativeMessagingHosts");
    fs.mkdirSync(manifestDir, { recursive: true });
    manifestPath = path.join(manifestDir, `${HOST_NAME}.json`);
    fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2), "utf8");
  }

  const sessionId = await attachServiceWorker(extensionId, timeoutMs);
  await ensureNativePortConnected(sessionId, timeoutMs);

  process.stdout.write(`${JSON.stringify({ ok: true, event: "ready", extensionId })}\n`);
}

main().catch((error) => {
  log(`FATAL: ${error instanceof Error ? error.message : String(error)}`);
  shutdown(1).catch(() => process.exit(1));
});
