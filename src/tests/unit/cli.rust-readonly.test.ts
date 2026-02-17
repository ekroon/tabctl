import assert from "node:assert/strict";
import path from "node:path";
import test from "node:test";
import { runCli, parseOutput } from "./cli-helpers";

const rustMockCli = path.join(__dirname, "fixtures", "rust-cli-mock.js");

test("rust CLI path delegates version command by default when binary is configured", async () => {
  const result = await runCli(["version"], undefined, {
    TABCTL_RUST_CLI_BIN: rustMockCli,
  });
  assert.equal(result.status, 0);
  const output = parseOutput(result);
  assert.equal(output.ok, true);
  assert.equal(output.data?.delegated, true);
  assert.deepEqual(output.data?.argv, ["version"]);
  assert.equal(typeof output.data?.version, "string");
});

test("rust CLI path can be rolled back to node with TABCTL_CLI_IMPL=node", async () => {
  const result = await runCli(["version"], undefined, {
    TABCTL_CLI_IMPL: "node",
    TABCTL_RUST_CLI_BIN: rustMockCli,
  });
  assert.equal(result.status, 0);
  const output = parseOutput(result);
  assert.equal(output.ok, true);
  assert.equal(output.data?.delegated, undefined);
  assert.equal(output.data?.component, "cli");
});

test("rust CLI path delegates ping and forwards TABCTL_SOCKET", async () => {
  const socketPath = path.join(process.cwd(), "tabctl-test-rust.sock");
  const result = await runCli(["ping"], socketPath, {
    TABCTL_CLI_IMPL: "rust",
    TABCTL_RUST_CLI_BIN: rustMockCli,
  });
  assert.equal(result.status, 0);
  const output = parseOutput(result);
  assert.equal(output.ok, true);
  assert.equal(output.data?.delegated, true);
  assert.deepEqual(output.data?.argv, ["ping"]);
  assert.equal(output.data?.socket, socketPath);
});

test("rust CLI path delegates history args", async () => {
  const result = await runCli(["history", "--limit", "5", "--json"], undefined, {
    TABCTL_CLI_IMPL: "rust",
    TABCTL_RUST_CLI_BIN: rustMockCli,
  });
  assert.equal(result.status, 0);
  const output = parseOutput(result);
  assert.equal(output.ok, true);
  assert.equal(output.data?.delegated, true);
  assert.deepEqual(output.data?.argv, ["history", "--limit", "5", "--json"]);
});

test("rust CLI path delegates mutating close args", async () => {
  const result = await runCli(["close", "--tab", "123", "--confirm", "--json"], undefined, {
    TABCTL_CLI_IMPL: "rust",
    TABCTL_RUST_CLI_BIN: rustMockCli,
  });
  assert.equal(result.status, 0);
  const output = parseOutput(result);
  assert.equal(output.ok, true);
  assert.equal(output.data?.delegated, true);
  assert.deepEqual(output.data?.argv, ["close", "--tab", "123", "--confirm", "--json"]);
  assert.equal(typeof output.data?.nodeCli, "string");
  assert.equal(typeof output.data?.nodeExec, "string");
});

test("rust CLI path preserves delegated exit code for mutating commands", async () => {
  const result = await runCli(["reload", "--json"], undefined, {
    TABCTL_CLI_IMPL: "rust",
    TABCTL_RUST_CLI_BIN: rustMockCli,
    TABCTL_RUST_MOCK_EXIT_CODE: "9",
  });
  assert.equal(result.status, 9);
});

test("rust CLI path does not intercept unsupported commands", async () => {
  const result = await runCli(["help", "--json"], undefined, {
    TABCTL_CLI_IMPL: "rust",
    TABCTL_RUST_CLI_BIN: rustMockCli,
  });
  assert.equal(result.status, 0);
  const output = parseOutput(result);
  assert.equal(output.ok, true);
  assert.ok(Array.isArray(output.data?.commands));
  assert.equal(output.data?.delegated, undefined);
});
