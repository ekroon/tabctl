#!/usr/bin/env node
"use strict";

const { spawn } = require("node:child_process");
const path = require("node:path");
const os = require("node:os");

const tabctl = process.env.TABCTL_BIN || "./rust/target/debug/tabctl";
const defaultTmpRoot =
  process.platform === "win32" ? path.join(os.tmpdir(), "tctl-it") : path.join("/tmp", "tctl-it");
const shortTmpRoot = process.env.TABCTL_TEST_TMP_ROOT || defaultTmpRoot;
let smokeBrowser = null;
let smokeProfile = null;
let smokeReady = null;
let smokeTabctlEnv = null;
let readTab = null;
let readWindow = null;
let testWindow = null;
let testGroup = null;
const createdWindowIds = new Set();
let finalizing = null;

function log(message) {
  const ts = new Date().toISOString();
  process.stdout.write(`[smoke-test ${ts}] ${message}\n`);
}

function run(command, args, options = {}) {
  const capture = options.capture === true;
  const label = [command, ...args].join(" ");
  log(`$ ${label}`);
  return new Promise((resolve, reject) => {
    const stdoutChunks = [];
    let stderr = "";
    const started = Date.now();
    const heartbeat = setInterval(() => {
      const seconds = Math.round((Date.now() - started) / 1000);
      log(`Still running after ${seconds}s: ${label}`);
    }, 15_000);
    const env = {
      ...process.env,
      TABCTL_TEST_TMP_ROOT: shortTmpRoot,
      TABCTL_BOOTSTRAP_TMP_ROOT: process.env.TABCTL_BOOTSTRAP_TMP_ROOT || shortTmpRoot,
    };
    if (options.scrubTabctlEnv === true) {
      for (const key of [
        "TABCTL_PROFILE",
        "TABCTL_CONFIG_DIR",
        "TABCTL_DATA_DIR",
        "TABCTL_STATE_DIR",
        "TABCTL_TRANSPORT",
        "TABCTL_TCP_PORT",
        "TABCTL_AUTH_TOKEN",
      ]) {
        delete env[key];
      }
    }
    Object.assign(env, options.env || {});

    const child = spawn(command, args, {
      stdio: ["ignore", "pipe", "pipe"],
      env,
    });
    let settled = false;

    function finish(code, signal) {
      if (settled) return;
      settled = true;
      clearInterval(heartbeat);
      if (code === 0) {
        const seconds = Math.round((Date.now() - started) / 1000);
        log(`Completed in ${seconds}s: ${label}`);
        resolve(stdoutChunks.join(""));
      } else {
        reject(
          new Error(
            `${command} exited with ${signal || code}${stderr ? `\n${stderr}` : ""}`
          )
        );
      }
    }

    child.stdout.on("data", (chunk) => {
      const text = chunk.toString("utf8");
      if (capture) stdoutChunks.push(text);
      if (!capture) process.stdout.write(text);
    });
    child.stderr.on("data", (chunk) => {
      const text = chunk.toString("utf8");
      stderr += text;
      process.stderr.write(text);
    });

    child.on("error", (err) => {
      clearInterval(heartbeat);
      reject(err);
    });
    child.on("exit", finish);
    child.on("close", finish);
  });
}

async function runJson(command, args, options = {}) {
  const output = await run(command, args, { ...options, capture: true });
  const parsed = JSON.parse(output);
  if (parsed.errors && parsed.errors.length > 0) {
    throw new Error(JSON.stringify(parsed.errors, null, 2));
  }
  return parsed;
}

async function query(source) {
  return runJson(tabctl, ["query", "--profile", smokeProfile, source], {
    env: smokeTabctlEnv,
    scrubTabctlEnv: true,
  });
}

function buildTabctlSmokeEnv(ready) {
  return {
    TABCTL_CONFIG_DIR: ready.configDir,
    TABCTL_STATE_DIR: ready.dataDir,
    XDG_CONFIG_HOME: path.join(ready.tmpDir, "xdg-config"),
    XDG_STATE_HOME: path.join(ready.tmpDir, "xdg-state"),
  };
}

function pathIsInside(parent, candidate) {
  const relative = path.relative(parent, candidate);
  return relative === "" || (!!relative && !relative.startsWith("..") && !path.isAbsolute(relative));
}

function assertSmokeReady() {
  expect(smokeReady, "smoke browser did not provide ready metadata");
  expect(smokeProfile && smokeProfile.startsWith("smoke-"), `unsafe smoke profile name: ${smokeProfile}`);
  for (const [label, value] of [
    ["configDir", smokeReady.configDir],
    ["dataDir", smokeReady.dataDir],
    ["browserProfileDir", smokeReady.browserProfileDir],
  ]) {
    expect(value, `smoke browser did not report ${label}`);
    expect(
      pathIsInside(smokeReady.tmpDir, value),
      `${label} is outside smoke temp root: ${value}`
    );
  }
}

async function verifySmokeProfile() {
  assertSmokeReady();
  const show = await runJson(
    tabctl,
    ["--json", "--no-pretty", "--profile", smokeProfile, "profile-show"],
    { env: smokeTabctlEnv, scrubTabctlEnv: true }
  );
  const data = show.data || show;
  expect(data.name === smokeProfile, `profile-show resolved ${data.name}, expected ${smokeProfile}`);
  expect(
    pathIsInside(smokeReady.configDir, data.profilesPath),
    `profile-show profilesPath is outside smoke config dir: ${data.profilesPath}`
  );
  expect(
    pathIsInside(smokeReady.dataDir, data.dataDir),
    `profile-show dataDir is outside smoke data dir: ${data.dataDir}`
  );
}

async function startSmokeBrowser() {
  log("Starting isolated smoke browser");
  smokeBrowser = spawn(process.execPath, ["scripts/smoke-browser.js"], {
    stdio: ["ignore", "pipe", "inherit"],
    env: {
      ...process.env,
      TABCTL_TEST_TMP_ROOT: shortTmpRoot,
      TABCTL_BOOTSTRAP_TMP_ROOT: process.env.TABCTL_BOOTSTRAP_TMP_ROOT || shortTmpRoot,
    },
  });

  let stdout = "";
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      reject(new Error("smoke browser did not report ready within 45s"));
    }, 45_000);

    smokeBrowser.stdout.on("data", (chunk) => {
      stdout += chunk.toString("utf8");
      const lines = stdout.split(/\r?\n/);
      stdout = lines.pop() || "";
      for (const line of lines) {
        if (!line.trim()) continue;
        process.stdout.write(`${line}\n`);
        try {
          const ready = JSON.parse(line);
          if (
            ready.ok &&
            ready.profile &&
            ready.tmpDir &&
            ready.configDir &&
            ready.dataDir &&
            ready.browserProfileDir
          ) {
            clearTimeout(timeout);
            smokeReady = ready;
            smokeProfile = ready.profile;
            smokeTabctlEnv = buildTabctlSmokeEnv(ready);
            log(`Smoke browser ready: profile=${ready.profile} pid=${ready.pid}`);
            resolve(ready);
          }
        } catch {
          // Non-JSON stdout is still useful progress output.
        }
      }
    });

    smokeBrowser.on("error", (err) => {
      clearTimeout(timeout);
      reject(err);
    });
    smokeBrowser.on("exit", (code, signal) => {
      if (!smokeProfile) {
        clearTimeout(timeout);
        reject(new Error(`smoke browser exited before ready (${signal || code})`));
      }
    });
  });
}

function expect(condition, message) {
  if (!condition) throw new Error(message);
}

async function cleanupSmokeTabs() {
  if (!smokeProfile) return;

  const cleanupErrors = [];
  const ids = new Set();

  try {
    const tabs = await query("query { tabs(limit: 500) { items { tabId windowId groupTitle } } }");
    for (const tab of tabs.data.tabs.items) {
      if (
        tab.tabId === readTab ||
        createdWindowIds.has(tab.windowId) ||
        (testGroup && tab.groupTitle === testGroup)
      ) {
        ids.add(tab.tabId);
      }
    }
  } catch (err) {
    cleanupErrors.push(`query smoke tabs: ${err.message}`);
  }

  if (ids.size > 0) {
    const tabIds = [...ids];
    log(`Cleaning up smoke tabs: ${tabIds.join(",")}`);
    try {
      await query(`mutation { closeTabs(tabIds: [${tabIds.join(",")}], confirm: true) { txid closedTabs } }`);
    } catch (err) {
      cleanupErrors.push(`close smoke tabs: ${err.message}`);
    }
  }

  try {
    const after = await query("query { tabs(limit: 500) { items { tabId windowId groupTitle } } }");
    const remaining = after.data.tabs.items.filter(
      (tab) =>
        tab.tabId === readTab ||
        createdWindowIds.has(tab.windowId) ||
        (testGroup && tab.groupTitle === testGroup)
    );
    if (remaining.length > 0) {
      cleanupErrors.push(
        `remaining smoke tabs after cleanup: ${remaining.map((tab) => tab.tabId).join(",")}`
      );
    }
  } catch (err) {
    cleanupErrors.push(`verify smoke cleanup: ${err.message}`);
  }

  if (cleanupErrors.length > 0) {
    throw new Error(cleanupErrors.join("; "));
  }
}

async function stopSmokeBrowser() {
  if (!smokeBrowser || smokeBrowser.exitCode !== null) return;
  log("Stopping isolated smoke browser");
  try {
    smokeBrowser.kill("SIGTERM");
  } catch {
    // already gone
  }
  await new Promise((resolve) => {
    const timeout = setTimeout(() => {
      if (smokeBrowser.exitCode === null) {
        try {
          smokeBrowser.kill("SIGKILL");
        } catch {
          // already gone
        }
      }
      resolve();
    }, 3_000);
    smokeBrowser.on("exit", () => {
      clearTimeout(timeout);
      resolve();
    });
  });
}

async function finalizeSmokeRun() {
  if (finalizing) return finalizing;
  finalizing = (async () => {
    const errors = [];
    try {
      await cleanupSmokeTabs();
    } catch (err) {
      errors.push(`cleanup tabs: ${err.message}`);
    }
    try {
      await stopSmokeBrowser();
    } catch (err) {
      errors.push(`stop smoke browser: ${err.message}`);
    }
    if (errors.length > 0) {
      throw new Error(errors.join("; "));
    }
  })();
  return finalizing;
}

function installSignalHandlers() {
  for (const signal of ["SIGINT", "SIGTERM"]) {
    process.once(signal, () => {
      log(`Received ${signal}; cleaning up smoke browser`);
      finalizeSmokeRun()
        .catch((err) => {
          log(`Cleanup after ${signal} failed: ${err.message}`);
        })
        .finally(() => {
          process.exit(signal === "SIGINT" ? 130 : 143);
        });
    });
  }
}

async function main() {
  installSignalHandlers();

  log("Step 1/8: build");
  await run("npm", ["run", "build"]);

  log("Step 2/8: Rust verify");
  await run("npm", ["run", "rust:verify"]);

  log("Step 3/8: integration tests");
  await run("npm", ["run", "test:integration"]);

  log("Step 4/8: start isolated browser");
  await startSmokeBrowser();

  log("Step 5/8: connectivity and read-only GraphQL checks");
  await verifySmokeProfile();
  await run(tabctl, ["ping", "--profile", smokeProfile], {
    env: smokeTabctlEnv,
    scrubTabctlEnv: true,
  });
  const ping = await query("query { ping { ok latencyMs } }");
  expect(ping.data.ping.ok === true, "GraphQL ping did not return ok=true");
  const tabs = await query("query { tabs(limit: 20) { total items { tabId windowId url title groupId groupTitle active } } }");
  expect(tabs.data.tabs.total >= 0, "tabs query returned an invalid total");
  const analyze = await query("query { analyze(staleDays: 30) { totalTabs staleTabs duplicateTabs } }");
  expect(analyze.data.analyze.totalTabs >= 0, "analyze query returned an invalid total");
  const report = await query("query { reportTabs { totals { tabs } entries { tabId windowId url title description } } }");
  expect(report.data.reportTabs.totals.tabs >= 0, "reportTabs returned an invalid total");

  log("Step 6/8: readTabs markdown extraction");
  const readOpen = await query('mutation { openTabs(urls: ["https://example.com"], newWindow: true) { windowId tabs { tabId url title } } }');
  readWindow = readOpen.data.openTabs.windowId;
  readTab = readOpen.data.openTabs.tabs[0].tabId;
  createdWindowIds.add(readWindow);
  log(`Opened readTabs page: window=${readWindow} tab=${readTab}`);
  await new Promise((resolve) => setTimeout(resolve, 2_000));
  const read = await query(`query { readTabs(tabIds: [${readTab}], extract: true, maxChars: 50000, timeoutMs: 15000) { totals { tabs tasks } entries { tabId url title chars truncated extracted status emptyReason error markdown } } }`);
  const readEntry = read.data.readTabs.entries[0];
  expect(read.data.readTabs.totals.tabs === 1, "readTabs did not return exactly one tab");
  expect(readEntry.status === "READ", `readTabs status was ${readEntry.status}: ${readEntry.error || readEntry.emptyReason || "no detail"}`);
  expect(readEntry.chars > 0, "readTabs returned empty content");
  expect(readEntry.markdown.length > 0, "readTabs returned empty markdown");
  log(`readTabs extracted ${readEntry.chars} chars from ${readEntry.url}`);

  log("Step 7/8: mutation, undo, screenshot, and inspect checks");
  testGroup = `TEST-Smoke-${Date.now()}`;
  const opened = await query(`mutation { openTabs(urls: ["https://example.com", "https://example.org", "https://example.net"], newWindow: true, group: "${testGroup}") { windowId groupId tabs { tabId windowId url title groupId groupTitle } } }`);
  testWindow = opened.data.openTabs.windowId;
  createdWindowIds.add(testWindow);
  const firstTab = opened.data.openTabs.tabs[0].tabId;
  log(`Opened smoke window: window=${testWindow} group=${testGroup} firstTab=${firstTab}`);
  const windowCheck = await query(`query { window(id: ${testWindow}) { windowId tabs { tabId url groupTitle index } groups { groupId title color collapsed tabCount } } }`);
  expect(windowCheck.data.window.windowId === testWindow, "smoke window was not found after openTabs");

  const closed = await query(`mutation { closeTabs(tabIds: [${firstTab}], confirm: true) { txid closedTabs } }`);
  expect(closed.data.closeTabs.closedTabs === 1, "closeTabs did not close exactly one tab");
  const closeTxid = closed.data.closeTabs.txid;
  log(`Closed one tab: txid=${closeTxid}`);
  await query(`mutation { undoAction(txid: "${closeTxid}") { txid summary } }`);
  log("Undo close succeeded");

  const archived = await query(`mutation { archiveTabs(windowId: ${testWindow}) { txid archivedTabs } }`);
  expect(archived.data.archiveTabs.archivedTabs > 0, "archiveTabs did not archive any tabs");
  const archiveTxid = archived.data.archiveTabs.txid;
  log(`Archived smoke window: txid=${archiveTxid}`);
  await query(`mutation { undoAction(txid: "${archiveTxid}") { txid summary } }`);
  log("Undo archive succeeded");

  const restored = await query("query { tabs(limit: 200) { items { tabId windowId groupTitle } } }");
  const restoredTabs = restored.data.tabs.items.filter((tab) => tab.groupTitle === testGroup);
  expect(restoredTabs.length > 0, "undo archive did not restore any grouped smoke tabs");
  testWindow = restoredTabs[0].windowId;
  createdWindowIds.add(testWindow);
  const restoredFirstTab = restoredTabs[0].tabId;
  const screenshots = await query(`query { captureScreenshots(tabIds: [${restoredFirstTab}], mode: "viewport") { totals { tabs tiles } entries { tabId tiles { index width height } error { message } } } }`);
  expect(screenshots.data.captureScreenshots.totals.tabs === 1, "captureScreenshots did not return one tab");
  const inspected = await query(`query { inspectTabs(tabIds: [${restoredFirstTab}], signals: ["page-meta"], waitFor: "dom") { totals { tabs signals tasks } entries { tabId signals { name valueJson } } } }`);
  expect(inspected.data.inspectTabs.totals.tabs === 1, "inspectTabs did not return one tab");

  log("Step 8/8: cleanup");
  await cleanupSmokeTabs();
  createdWindowIds.clear();
  readTab = null;
  readWindow = null;
  testWindow = null;
  testGroup = null;
  log("Smoke test completed successfully");
}

main()
  .catch(async (err) => {
    log(`FAILED: ${err instanceof Error ? err.message : String(err)}`);
    process.exitCode = 1;
  })
  .finally(async () => {
    try {
      await finalizeSmokeRun();
    } catch (err) {
      log(`Cleanup failed: ${err instanceof Error ? err.message : String(err)}`);
      process.exitCode = 1;
    }
  });
