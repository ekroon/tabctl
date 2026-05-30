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

function log(msg) {
  process.stderr.write(`[smoke-browser] ${msg}\n`);
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
let profileName = null;
let tabctlBin = null;
let shuttingDown = false;

async function shutdown(exitCode) {
  if (shuttingDown) return;
  shuttingDown = true;

  if (browserProc && !browserProc.killed) {
    log("Stopping browser...");
    browserProc.kill("SIGTERM");
    await sleep(800);
    if (!browserProc.killed) {
      try {
        browserProc.kill("SIGKILL");
      } catch {
        // already gone
      }
    }
  }

  if (profileName && tabctlBin) {
    try {
      execFileSync(tabctlBin, ["profile-remove", profileName], { stdio: "ignore" });
      log(`Removed profile ${profileName}`);
    } catch {
      // best effort
    }
  }

  if (tmpDir) {
    try {
      fs.rmSync(tmpDir, { recursive: true, force: true });
      log(`Removed ${tmpDir}`);
    } catch {
      // best effort
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

  log(`tabctl:    ${tabctlBin}`);
  log(`browser:   ${browserBin} (${browserName})`);
  log(`extension: ${extensionDirInput}`);
  log(`profile:   ${profileName}`);

  // Create temp dir for isolated browser profile.
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-smoke-"));
  const browserProfileDir = path.join(tmpDir, "browser-profile");
  fs.mkdirSync(browserProfileDir, { recursive: true });

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
      { encoding: "utf8" }
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
  try {
    const setupJson = JSON.parse(setupOutput);
    activeExtDir = setupJson?.data?.extensionSync?.activePath;
  } catch {
    // ignore parse errors; will fail below
  }

  if (!activeExtDir) {
    throw new Error(
      "tabctl setup did not return an active extension path (data.extensionSync.activePath)"
    );
  }
  log(`Active extension: ${activeExtDir}`);

  // Step 2: Launch the browser with the isolated profile and local extension.
  const browserArgs = [
    `--load-extension=${activeExtDir}`,
    `--user-data-dir=${browserProfileDir}`,
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-default-apps",
    "--new-window",
    "about:blank",
  ];

  log(`Launching browser...`);
  browserProc = spawn(browserBin, browserArgs, {
    stdio: ["ignore", "ignore", "pipe"],
    detached: false,
  });

  browserProc.stderr.on("data", (chunk) => {
    const text = chunk.toString("utf8").trim();
    if (text) log(`browser: ${text.slice(-200)}`);
  });

  browserProc.on("exit", (code) => {
    if (!shuttingDown) {
      log(`Browser exited unexpectedly (code ${code})`);
      shutdown(1).catch(() => process.exit(1));
    }
  });

  // Step 3: Poll tabctl ping until the extension connects.
  const timeoutMs = parseInt(process.env.SMOKE_BROWSER_TIMEOUT_MS || "30000", 10);
  const deadline = Date.now() + timeoutMs;
  log(`Waiting for ping (${timeoutMs}ms timeout)...`);

  while (Date.now() < deadline) {
    if (browserProc.exitCode !== null) {
      throw new Error(`Browser exited early (code ${browserProc.exitCode})`);
    }
    try {
      execFileSync(tabctlBin, ["ping", "--profile", profileName], {
        stdio: "ignore",
        timeout: 3000,
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
