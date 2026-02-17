import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";

export const cliPath = path.resolve(__dirname, "../../cli/tabctl.js");
export const pkgVersion = JSON.parse(fs.readFileSync(path.resolve(__dirname, "../../../package.json"), "utf8")).version;
export const testConfigHome = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-test-config-"));
const runCliTimeoutMs = Number(process.env.TABCTL_TEST_CLI_TIMEOUT_MS || "2000");

export async function runCli(
  args: string[],
  socketPath?: string,
  extraEnv?: Record<string, string>,
  cliOverride?: string,
  npxOverride?: string,
) {
  const env = { ...process.env };
  const hasCliImplOverride = Boolean(extraEnv && Object.prototype.hasOwnProperty.call(extraEnv, "TABCTL_CLI_IMPL"));
  const hasRustCliBinOverride = Boolean(extraEnv && Object.prototype.hasOwnProperty.call(extraEnv, "TABCTL_RUST_CLI_BIN"));
  if (!hasCliImplOverride && !hasRustCliBinOverride) {
    env.TABCTL_CLI_IMPL = "node";
  }
  if (socketPath) {
    env.TABCTL_SOCKET = socketPath;
  }
  if (extraEnv) {
    Object.assign(env, extraEnv);
  }
  const hasCustomConfig = extraEnv && Object.prototype.hasOwnProperty.call(extraEnv, "XDG_CONFIG_HOME");
  if (!hasCustomConfig) {
    env.XDG_CONFIG_HOME = testConfigHome;
  }

  return new Promise<{ status: number | null; stdout: string; stderr: string }>((resolve, reject) => {
    const effectiveCli = cliOverride || cliPath;
    const effectiveEnv = { ...env };
    if (npxOverride) {
      effectiveEnv.PATH = `${path.dirname(npxOverride)}${path.delimiter}${env.PATH || ""}`;
    }
    const child = spawn(process.execPath, [effectiveCli, ...args], { env: effectiveEnv });
    let stdout = "";
    let stderr = "";

    const timeout = setTimeout(() => {
      child.kill("SIGKILL");
      reject(new Error("CLI timeout"));
    }, runCliTimeoutMs);

    child.stdout.on("data", (data) => {
      stdout += data.toString();
    });

    child.stderr.on("data", (data) => {
      stderr += data.toString();
    });

    child.on("error", (error) => {
      clearTimeout(timeout);
      reject(error);
    });

    child.on("exit", (code) => {
      clearTimeout(timeout);
      resolve({ status: code, stdout, stderr });
    });
  });
}

export async function runCliWithStdin(
  args: string[],
  stdinData: string,
  extraEnv?: Record<string, string>,
) {
  const env = { ...process.env };
  const hasCliImplOverride = Boolean(extraEnv && Object.prototype.hasOwnProperty.call(extraEnv, "TABCTL_CLI_IMPL"));
  const hasRustCliBinOverride = Boolean(extraEnv && Object.prototype.hasOwnProperty.call(extraEnv, "TABCTL_RUST_CLI_BIN"));
  if (!hasCliImplOverride && !hasRustCliBinOverride) {
    env.TABCTL_CLI_IMPL = "node";
  }
  if (extraEnv) {
    Object.assign(env, extraEnv);
  }
  const hasCustomConfig = extraEnv && Object.prototype.hasOwnProperty.call(extraEnv, "XDG_CONFIG_HOME");
  if (!hasCustomConfig) {
    env.XDG_CONFIG_HOME = testConfigHome;
  }

  return new Promise<{ status: number | null; stdout: string; stderr: string }>((resolve, reject) => {
    const child = spawn(process.execPath, [cliPath, ...args], { env, stdio: ["pipe", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";

    const timeout = setTimeout(() => {
      child.kill("SIGKILL");
      reject(new Error("CLI timeout"));
    }, 5000);

    child.stdout.on("data", (data) => {
      stdout += data.toString();
    });

    child.stderr.on("data", (data) => {
      stderr += data.toString();
    });

    child.on("error", (error) => {
      clearTimeout(timeout);
      reject(error);
    });

    child.on("exit", (code) => {
      clearTimeout(timeout);
      resolve({ status: code, stdout, stderr });
    });

    child.stdin.write(stdinData);
    child.stdin.end();
  });
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export function parseOutput(result: { stdout: string }): any {
  return JSON.parse(result.stdout.trim());
}

export function mockResponse(req: Record<string, unknown>, data: Record<string, unknown> = {}): Record<string, unknown> {
  return { ok: true, action: req.action, requestId: req.id, data };
}

export function assertVersion(version: string | undefined) {
  assert.ok(version);
  if (version && version.includes("-dev.")) {
    const re = new RegExp(`^${pkgVersion.replace(/\./g, "\\.")}-dev\\.[0-9a-f]{8}(\\.dirty)?$`);
    assert.match(version, re);
  } else {
    assert.equal(version, pkgVersion);
  }
}
