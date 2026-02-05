#!/usr/bin/env node
"use strict";

const net = require("net");
const fs = require("fs");
const os = require("os");
const path = require("path");

const args = process.argv.slice(2);
const socketIndex = args.indexOf("--socket");
const socketArg = socketIndex >= 0 ? args[socketIndex + 1] : null;
const socketPath = socketArg
  || process.env.TABCTL_SOCKET
  || path.join(os.tmpdir(), `tabctl-mock-${process.pid}.sock`);

if (fs.existsSync(socketPath)) {
  fs.unlinkSync(socketPath);
}

const state = {
  windows: [],
  nextWindowId: 1,
  nextTabId: 1,
  nextGroupId: 1,
};

function createWindow(idOverride) {
  const id = Number.isFinite(idOverride) ? idOverride : state.nextWindowId;
  state.nextWindowId = Math.max(state.nextWindowId, id + 1);
  state.windows.forEach((win) => { win.focused = false; });
  const window = { windowId: id, focused: true, tabs: [], groups: [] };
  state.windows.push(window);
  return window;
}

function resolveWindow(params) {
  if (params.newWindow || params.windowId === "new" || state.windows.length === 0) {
    return createWindow();
  }
  if (typeof params.windowId === "number") {
    const existing = state.windows.find((win) => win.windowId === params.windowId);
    return existing || createWindow(params.windowId);
  }
  if (typeof params.windowId === "string") {
    return state.windows[0] || createWindow();
  }
  return state.windows[0] || createWindow();
}

function ensureGroup(window, title, color) {
  if (!title) {
    return { groupId: -1, groupTitle: null };
  }
  let group = window.groups.find((entry) => entry.title === title);
  if (!group) {
    group = { groupId: state.nextGroupId++, title, color: color || "grey", collapsed: false };
    window.groups.push(group);
  }
  return { groupId: group.groupId, groupTitle: group.title };
}

function handleOpen(request) {
  const params = request.params || {};
  const urls = Array.isArray(params.urls) ? params.urls.map(String) : [];
  const window = resolveWindow(params);
  const groupInfo = ensureGroup(window, params.groupTitle, params.color);
  const createdTabs = [];

  for (const url of urls) {
    const tab = {
      tabId: state.nextTabId++,
      windowId: window.windowId,
      index: window.tabs.length,
      groupId: groupInfo.groupId,
      groupTitle: groupInfo.groupTitle,
      title: `Mock ${url}`,
      url,
    };
    window.tabs.push(tab);
    createdTabs.push(tab);
  }

  return {
    windowId: window.windowId,
    groupId: groupInfo.groupId,
    openedTabs: createdTabs,
  };
}

function handleList() {
  return {
    windows: state.windows.map((win) => ({
      ...win,
      tabs: win.tabs.map((tab) => ({ ...tab })),
      groups: win.groups.map((group) => ({ groupId: group.groupId, title: group.title })),
    })),
  };
}

function handleGroupList(params) {
  const groups = [];
  for (const win of state.windows) {
    if (params.windowId != null && win.windowId !== params.windowId) {
      continue;
    }
    for (const group of win.groups) {
      groups.push({
        groupId: group.groupId,
        title: group.title,
        windowId: win.windowId,
        color: group.color,
        collapsed: group.collapsed,
      });
    }
  }
  return { groups };
}

function handleInspect(request) {
  const snapshot = handleList();
  const entries = snapshot.windows.flatMap((win) => win.tabs.map((tab) => ({
    tabId: tab.tabId,
    windowId: tab.windowId,
    groupId: tab.groupId,
    url: tab.url,
    title: tab.title,
    signals: {
      "page-meta": { ok: true, durationMs: 1, data: { title: tab.title, description: "Mock page", h1: tab.title } },
    },
  })));
  return { generatedAt: Date.now(), entries };
}

function handleScreenshot() {
  const snapshot = handleList();
  const entries = snapshot.windows.flatMap((win) => win.tabs.map((tab) => ({
    tabId: tab.tabId,
    windowId: tab.windowId,
    groupId: tab.groupId,
    url: tab.url,
    title: tab.title,
    tiles: [],
  })));
  return { generatedAt: Date.now(), totals: { tabs: entries.length, tiles: 0 }, entries };
}

function buildResponse(request, data) {
  const version = request.client && typeof request.client.version === "string"
    ? request.client.version
    : "0.0.0";
  return {
    ok: true,
    action: request.action,
    requestId: request.id,
    component: "host",
    version,
    data: {
      extensionVersion: version,
      extensionComponent: "extension",
      ...data,
    },
  };
}

const server = net.createServer((socket) => {
  socket.setEncoding("utf8");
  let buffer = "";

  socket.on("data", (data) => {
    buffer += data;
    let index;
    while ((index = buffer.indexOf("\n")) >= 0) {
      const line = buffer.slice(0, index).trim();
      buffer = buffer.slice(index + 1);
      if (!line) {
        continue;
      }
      let request;
      try {
        request = JSON.parse(line);
      } catch {
        socket.write(JSON.stringify({ ok: false, error: { message: "Invalid JSON" } }) + "\n");
        continue;
      }

      let payload;
      switch (request.action) {
        case "open":
          payload = handleOpen(request);
          break;
        case "list":
          payload = handleList();
          break;
        case "group-list":
          payload = handleGroupList(request.params || {});
          break;
        case "inspect":
          payload = handleInspect(request);
          break;
        case "screenshot":
          payload = handleScreenshot();
          break;
        case "ping":
        case "version":
          payload = { now: Date.now() };
          break;
        default:
          socket.write(JSON.stringify({
            ok: false,
            action: request.action,
            requestId: request.id,
            error: { message: `Mock host does not support ${request.action}` },
          }) + "\n");
          continue;
      }

      socket.write(JSON.stringify(buildResponse(request, payload)) + "\n");
    }
  });
});

server.listen(socketPath, () => {
  process.stdout.write(`[tabctl-mock] listening on ${socketPath}\n`);
});

function cleanup() {
  server.close(() => {
    if (fs.existsSync(socketPath)) {
      fs.unlinkSync(socketPath);
    }
    process.exit(0);
  });
}

process.on("SIGINT", cleanup);
process.on("SIGTERM", cleanup);
