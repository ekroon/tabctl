#!/usr/bin/env node
"use strict";

const payload = {
  ok: true,
  data: {
    delegated: true,
    argv: process.argv.slice(2),
    nodeCli: process.env.TABCTL_NODE_CLI_BIN || null,
    nodeExec: process.env.TABCTL_NODE_EXEC || null,
    socket: process.env.TABCTL_SOCKET || null,
    version: process.env.TABCTL_VERSION || null,
    baseVersion: process.env.TABCTL_BASE_VERSION || null,
    gitSha: process.env.TABCTL_GIT_SHA || null,
    dirty: process.env.TABCTL_DIRTY === "1",
  },
};

process.stdout.write(`${JSON.stringify(payload)}\n`);
process.exit(Number(process.env.TABCTL_RUST_MOCK_EXIT_CODE || "0"));
