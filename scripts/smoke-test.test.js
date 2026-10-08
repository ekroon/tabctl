"use strict";

const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const { once } = require("node:events");
const { test } = require("node:test");
const { stopSmokeBrowser } = require("./smoke-test");

test("a hung fixture is killed before reporting cleanup timeout", async () => {
  const child = spawn(process.execPath, ["-e", `
    process.on("SIGTERM", () => {});
    setInterval(() => {}, 1000);
    console.log("ready");
  `], { stdio: ["ignore", "pipe", "ignore"] });
  try {
    await once(child.stdout, "data");
    await assert.rejects(stopSmokeBrowser(child, 50), /did not finish cleanup/);
    await Promise.race([
      once(child, "exit"),
      new Promise((_, reject) => {
        const timer = setTimeout(() => reject(new Error("fixture survived timeout")), 500);
        timer.unref();
      }),
    ]);
    assert.equal(child.signalCode, "SIGKILL");
    assert.throws(() => process.kill(child.pid, 0), { code: "ESRCH" });
  } finally {
    if (child.exitCode === null && child.signalCode === null) {
      const exited = once(child, "exit");
      child.kill("SIGKILL");
      await exited;
    }
  }
});

test("graceful fixture teardown remains successful", async () => {
  const child = spawn(process.execPath, ["-e", `
    process.on("SIGTERM", () => process.exit(0));
    setInterval(() => {}, 1000);
    console.log("ready");
  `], { stdio: ["ignore", "pipe", "ignore"] });
  try {
    await once(child.stdout, "data");
    await stopSmokeBrowser(child, 1000);
    assert.equal(child.exitCode, 0);
    await stopSmokeBrowser(child, 50);
  } finally {
    if (child.exitCode === null && child.signalCode === null) {
      const exited = once(child, "exit");
      child.kill("SIGKILL");
      await exited;
    }
  }
});
