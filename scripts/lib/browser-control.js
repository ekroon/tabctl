"use strict";

const fs = require("node:fs");
const net = require("node:net");
const path = require("node:path");

async function seedBrowser(action, params) {
  switch (action) {
    case "tab-create":
      return chrome.tabs.create(params);
    case "tab-update": {
      const { tabId, ...properties } = params;
      return chrome.tabs.update(tabId, properties);
    }
    case "tab-group":
      return { groupId: await chrome.tabs.group(params) };
    case "group-update": {
      const { groupId, ...properties } = params;
      return chrome.tabGroups.update(groupId, properties);
    }
    case "window-remove":
      await chrome.windows.remove(params.windowId);
      return { removed: true };
    default:
      throw new Error(`Unsupported fixture operation: ${action}`);
  }
}

async function startBrowserControl(root, extensionId, sendCDP) {
  async function execute(request) {
    const { targetInfos } = await sendCDP("Target.getTargets");
    const worker = targetInfos.find((target) =>
      target.type === "service_worker" && target.url.startsWith(`chrome-extension://${extensionId}/`));
    if (!worker) throw new Error("Test extension service worker is unavailable");
    const { sessionId } = await sendCDP("Target.attachToTarget", { targetId: worker.targetId, flatten: true });
    try {
      const result = await sendCDP("Runtime.evaluate", {
        expression: `(${seedBrowser.toString()})(${JSON.stringify(request.action)}, ${JSON.stringify(request.params)})`,
        awaitPromise: true,
        returnByValue: true,
      }, sessionId);
      if (result.exceptionDetails) {
        throw new Error(result.exceptionDetails.exception?.description || result.exceptionDetails.text);
      }
      return result.result.value;
    } finally {
      await sendCDP("Target.detachFromTarget", { sessionId });
    }
  }

  const connections = new Set();
  const server = net.createServer((socket) => {
    connections.add(socket);
    socket.on("close", () => connections.delete(socket));
    socket.on("error", (error) => process.stderr.write(`[browser-control] ${error.message}\n`));
    let buffer = "";
    let handled = false;
    socket.setEncoding("utf8");
    socket.on("data", (chunk) => {
      if (handled) return;
      buffer += chunk;
      if (Buffer.byteLength(buffer) > 64 * 1024) {
        handled = true;
        socket.end(`${JSON.stringify({ ok: false, error: { message: "Fixture request too large" } })}\n`);
        return;
      }
      const newline = buffer.indexOf("\n");
      if (newline < 0) return;
      handled = true;
      Promise.resolve().then(() => execute(JSON.parse(buffer.slice(0, newline))))
        .then((data) => socket.end(`${JSON.stringify({ ok: true, data })}\n`))
        .catch((error) => socket.end(`${JSON.stringify({ ok: false, error: { message: error.message } })}\n`));
    });
  });
  const socketPath = path.join(root, "control.sock");
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(socketPath, resolve);
  });
  fs.chmodSync(socketPath, 0o600);
  return {
    close: () => new Promise((resolve, reject) => {
      for (const socket of connections) socket.destroy();
      server.close((error) => error ? reject(error) : resolve());
    }),
  };
}

module.exports = { startBrowserControl };
