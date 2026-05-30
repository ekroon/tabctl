#!/usr/bin/env node
"use strict";

const { spawn } = require("node:child_process");
const path = require("node:path");

const tabctl = process.env.TABCTL_BIN || "./rust/target/debug/tabctl";
const shortTmpRoot = process.env.TABCTL_TEST_TMP_ROOT || path.join("/tmp", "tctl-it");
let smokeBrowser = null;
let smokeProfile = null;
let readTab = null;
let testWindow = null;
let testGroup = null;

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
    const child = spawn(command, args, {
      stdio: ["ignore", "pipe", "pipe"],
      env: {
        ...process.env,
        TABCTL_TEST_TMP_ROOT: shortTmpRoot,
        TABCTL_BOOTSTRAP_TMP_ROOT: process.env.TABCTL_BOOTSTRAP_TMP_ROOT || shortTmpRoot,
      },
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

async function runJson(command, args) {
  const output = await run(command, args, { capture: true });
  const parsed = JSON.parse(output);
  if (parsed.errors && parsed.errors.length > 0) {
    throw new Error(JSON.stringify(parsed.errors, null, 2));
  }
  return parsed;
}

async function query(source) {
  return runJson(tabctl, ["query", "--profile", smokeProfile, source]);
}

async function startSmokeBrowser() {
  log("Starting isolated smoke browser");
  smokeBrowser = spawn(process.execPath, ["scripts/smoke-browser.js"], {
    stdio: ["ignore", "pipe", "inherit"],
    env: process.env,
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
          if (ready.ok && ready.profile) {
            clearTimeout(timeout);
            smokeProfile = ready.profile;
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

  if (readTab) {
    log(`Cleaning up readTabs tab ${readTab}`);
    try {
      await query(`mutation { closeTabs(tabIds: [${readTab}], confirm: true) { txid closedTabs } }`);
    } catch (err) {
      log(`Cleanup warning for readTabs tab: ${err.message}`);
    }
  }

  if (testGroup) {
    log(`Cleaning up smoke group ${testGroup}`);
    try {
      const tabs = await query("query { tabs(limit: 200) { items { tabId groupTitle } } }");
      const ids = tabs.data.tabs.items
        .filter((tab) => tab.groupTitle === testGroup)
        .map((tab) => tab.tabId);
      if (ids.length > 0) {
        await query(`mutation { closeTabs(tabIds: [${ids.join(",")}], confirm: true) { txid closedTabs } }`);
      }
    } catch (err) {
      log(`Cleanup warning for smoke group: ${err.message}`);
    }
  }
}

async function stopSmokeBrowser() {
  if (!smokeBrowser || smokeBrowser.exitCode !== null) return;
  log("Stopping isolated smoke browser");
  smokeBrowser.kill("SIGTERM");
  await new Promise((resolve) => {
    const timeout = setTimeout(() => {
      if (smokeBrowser.exitCode === null) smokeBrowser.kill("SIGKILL");
      resolve();
    }, 3_000);
    smokeBrowser.on("exit", () => {
      clearTimeout(timeout);
      resolve();
    });
  });
}

async function main() {
  log("Step 1/8: build");
  await run("npm", ["run", "build"]);

  log("Step 2/8: Rust verify");
  await run("npm", ["run", "rust:verify"]);

  log("Step 3/8: integration tests");
  await run("npm", ["run", "test:integration"]);

  log("Step 4/8: start isolated browser");
  await startSmokeBrowser();

  log("Step 5/8: connectivity and read-only GraphQL checks");
  await run(tabctl, ["ping", "--profile", smokeProfile]);
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
  readTab = readOpen.data.openTabs.tabs[0].tabId;
  log(`Opened readTabs page: tab=${readTab}`);
  await new Promise((resolve) => setTimeout(resolve, 2_000));
  const read = await query(`query { readTabs(tabIds: [${readTab}], extract: true, maxChars: 50000, timeoutMs: 15000) { totals { tabs tasks } entries { tabId url title chars truncated extracted markdown } } }`);
  const readEntry = read.data.readTabs.entries[0];
  expect(read.data.readTabs.totals.tabs === 1, "readTabs did not return exactly one tab");
  expect(readEntry.chars > 0, "readTabs returned empty content");
  expect(readEntry.markdown.length > 0, "readTabs returned empty markdown");
  log(`readTabs extracted ${readEntry.chars} chars from ${readEntry.url}`);

  log("Step 7/8: mutation, undo, screenshot, and inspect checks");
  testGroup = `TEST-Smoke-${Date.now()}`;
  const opened = await query(`mutation { openTabs(urls: ["https://example.com", "https://example.org", "https://example.net"], newWindow: true, group: "${testGroup}") { windowId groupId tabs { tabId windowId url title groupId groupTitle } } }`);
  testWindow = opened.data.openTabs.windowId;
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
  const restoredFirstTab = restoredTabs[0].tabId;
  const screenshots = await query(`query { captureScreenshots(tabIds: [${restoredFirstTab}], mode: "viewport") { totals { tabs tiles } entries { tabId tiles { index width height } error { message } } } }`);
  expect(screenshots.data.captureScreenshots.totals.tabs === 1, "captureScreenshots did not return one tab");
  const inspected = await query(`query { inspectTabs(tabIds: [${restoredFirstTab}], signals: ["page-meta"], waitFor: "dom") { totals { tabs signals tasks } entries { tabId signals { name valueJson } } } }`);
  expect(inspected.data.inspectTabs.totals.tabs === 1, "inspectTabs did not return one tab");

  log("Step 8/8: cleanup");
  await cleanupSmokeTabs();
  readTab = null;
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
    await cleanupSmokeTabs();
    await stopSmokeBrowser();
  });
