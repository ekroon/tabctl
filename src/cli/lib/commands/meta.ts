/**
 * Meta command handlers: version, ping, setup, skill, policy, history, undo
 * These commands don't operate on tab data and are relatively self-contained.
 */

import fs from "fs";
import os from "os";
import path from "path";
import { spawnSync } from "node:child_process";

import { VERSION, BASE_VERSION, GIT_SHA, DIRTY, HOST_NAME, HOST_DESCRIPTION, EXTENSION_ID_PATTERN, SKILL_NAME, SKILL_REPO, resolveConfig } from "../constants";
import { printJson, errorOut } from "../output";
import { sendRequest, createRequestId } from "../client";
import { defaultPolicyPath, defaultPolicyTemplate, summarizePolicy, type Policy } from "../policy";
import type { Options, PolicyContext } from "../types";

// ============================================================================
// Setup Command
// ============================================================================

function resolveBrowser(value: unknown): "edge" | "chrome" | null {
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim().toLowerCase();
  if (trimmed === "edge" || trimmed === "chrome") {
    return trimmed;
  }
  return null;
}

function resolveExtensionId(options: Options): string {
  const raw = typeof options["extension-id"] === "string"
    ? String(options["extension-id"])
    : (process.env.TABCTL_EXTENSION_ID || "");
  const value = raw.trim().toLowerCase();
  if (!value) {
    errorOut("Missing --extension-id (or TABCTL_EXTENSION_ID)");
  }
  if (!EXTENSION_ID_PATTERN.test(value)) {
    errorOut(`Extension ID looks unusual: ${raw}`);
  }
  return value;
}

function resolveNodePath(options: Options): string {
  const raw = typeof options.node === "string"
    ? String(options.node)
    : (process.env.TABCTL_NODE || process.execPath || "");
  const value = raw.trim();
  if (!value) {
    errorOut("Node binary not found. Set --node or TABCTL_NODE.");
  }
  if (!path.isAbsolute(value)) {
    errorOut(`Node path must be absolute: ${value}`);
  }
  try {
    fs.accessSync(value, fs.constants.X_OK);
  } catch {
    errorOut(`Node binary not executable: ${value}`);
  }
  return value;
}

function resolveHostPath(): string {
  const root = path.resolve(__dirname, "../../..");
  const hostPath = path.join(root, "host", "host.js");
  if (!fs.existsSync(hostPath)) {
    errorOut(`Host script not found at ${hostPath}. Run: npm run build`);
  }
  return hostPath;
}

function resolveManifestDir(browser: "edge" | "chrome"): string {
  const home = os.homedir();
  if (!home) {
    errorOut("Home directory not found.");
  }
  if (browser === "edge") {
    return path.join(home, "Library", "Application Support", "Microsoft Edge", "NativeMessagingHosts");
  }
  return path.join(home, "Library", "Application Support", "Google", "Chrome", "NativeMessagingHosts");
}

function writeWrapper(nodePath: string, hostPath: string): string {
  const { wrapperDir } = resolveConfig();
  fs.mkdirSync(wrapperDir, { recursive: true, mode: 0o700 });
  const wrapperPath = path.join(wrapperDir, "tabctl-host.sh");
  const escapedNode = nodePath.replace(/"/g, "\\\"");
  const escapedHost = hostPath.replace(/"/g, "\\\"");
  const wrapper = [
    "#!/usr/bin/env bash",
    "set -euo pipefail",
    `exec \"${escapedNode}\" \"${escapedHost}\"`,
    "",
  ].join("\n");
  fs.writeFileSync(wrapperPath, wrapper, "utf8");
  fs.chmodSync(wrapperPath, 0o700);
  return wrapperPath;
}

export function runSetup(options: Options, prettyOutput: boolean): void {
  if (process.platform !== "darwin") {
    errorOut("tabctl setup is only supported on macOS.");
  }

  const browser = resolveBrowser(options.browser);
  if (!browser) {
    errorOut("Missing or invalid --browser (edge|chrome)");
  }

  const extensionId = resolveExtensionId(options);
  const nodePath = resolveNodePath(options);
  const hostPath = resolveHostPath();
  const wrapperPath = writeWrapper(nodePath, hostPath);
  const manifestDir = resolveManifestDir(browser);
  fs.mkdirSync(manifestDir, { recursive: true });

  const manifestPath = path.join(manifestDir, `${HOST_NAME}.json`);
  const manifest = {
    name: HOST_NAME,
    description: HOST_DESCRIPTION,
    path: wrapperPath,
    type: "stdio",
    allowed_origins: [`chrome-extension://${extensionId}/`],
  };
  fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2), "utf8");

  printJson({
    ok: true,
    data: {
      browser,
      extensionId,
      manifestPath,
      hostPath,
      nodePath,
      wrapperPath,
    },
  }, prettyOutput);
}

// ============================================================================
// Skill Command
// ============================================================================

function formatCliArgValue(value: unknown): string {
  const raw = String(value);
  if (!raw) {
    return raw;
  }
  if (/[\s"]/g.test(raw)) {
    const escaped = raw.replace(/"/g, "\\\"");
    return `"${escaped}"`;
  }
  return raw;
}

function resolveProjectRoot(): string {
  try {
    return fs.realpathSync(process.cwd());
  } catch {
    return path.resolve(process.cwd());
  }
}

function resolveConfigHome(): string {
  return process.env.XDG_CONFIG_HOME || path.join(os.homedir(), ".config");
}

function resolveSkillTargetDir(globalInstall: boolean): string {
  if (globalInstall) {
    return path.join(resolveConfigHome(), "opencode", "skills", SKILL_NAME);
  }
  return path.join(resolveProjectRoot(), ".opencode", "skills", SKILL_NAME);
}

function runSkillsCli(args: string[]): void {
  const result = spawnSync("npx", ["skills", ...args], { stdio: "pipe" });
  if (result.error) {
    errorOut(`Failed to run skills CLI: ${result.error.message}`);
  }
  if (typeof result.status === "number" && result.status !== 0) {
    const stderr = result.stderr ? result.stderr.toString().trim() : "";
    const stdout = result.stdout ? result.stdout.toString().trim() : "";
    const detail = stderr || stdout;
    const message = detail ? `skills CLI failed: ${detail}` : `skills CLI exited with status ${result.status}`;
    errorOut(message);
  }
}

export function runSkillInstall(options: Options, prettyOutput: boolean): void {
  const globalInstall = options.global === true;
  const installTarget = resolveSkillTargetDir(globalInstall);
  const agents = Array.isArray(options.agent)
    ? (options.agent as string[]).filter((value) => typeof value === "string" && value.trim())
    : [];
  const args = ["add", SKILL_REPO, "--skill", SKILL_NAME];
  if (agents.length > 0) {
    for (const agent of agents) {
      args.push("-a", agent);
    }
  }
  if (globalInstall) {
    args.push("-g");
  }
  const hintAgents = agents.length > 0 ? agents.map((agent) => `-a ${formatCliArgValue(agent)}`).join(" ") : "";
  const installHintParts = ["npx skills add", formatCliArgValue(SKILL_REPO), "--skill", SKILL_NAME];
  if (hintAgents) {
    installHintParts.push(hintAgents);
  }
  if (globalInstall) {
    installHintParts.push("-g");
  }
  const installHint = installHintParts.join(" ").trim();

  runSkillsCli(args);

  printJson({
    ok: true,
    data: {
      name: SKILL_NAME,
      targetDir: installTarget,
      scope: globalInstall ? "global" : "project",
      installHint,
      tool: "skills",
    },
  }, prettyOutput);
}

// ============================================================================
// Version Command
// ============================================================================

export function runVersion(prettyOutput: boolean): void {
  printJson({
    ok: true,
    data: {
      version: VERSION,
      baseVersion: BASE_VERSION,
      gitSha: GIT_SHA,
      dirty: DIRTY,
      component: "cli",
    },
  }, prettyOutput);
}

// ============================================================================
// Policy Command
// ============================================================================

export function runPolicy(options: Options, policyContext: PolicyContext, prettyOutput: boolean): void {
  const policyPath = defaultPolicyPath();
  if (options.init) {
    if (fs.existsSync(policyPath)) {
      printJson({
        ok: true,
        data: {
          status: "exists",
          path: policyPath,
        },
      }, prettyOutput);
      return;
    }

    const dir = path.dirname(policyPath);
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(policyPath, JSON.stringify(defaultPolicyTemplate(), null, 2), "utf8");
    printJson({
      ok: true,
      data: {
        status: "created",
        path: policyPath,
      },
    }, prettyOutput);
    return;
  }

  const policySummary = summarizePolicy(policyContext.policy, policyContext.path);
  printJson({
    ok: true,
    data: {
      ...policySummary,
      path: policyPath,
    },
  }, prettyOutput);
}

// ============================================================================
// History Command
// ============================================================================

export async function runHistory(options: Options, prettyOutput: boolean): Promise<void> {
  const params: Record<string, unknown> = {
    limit: options.limit ? Number(options.limit) : undefined,
  };

  const response = await sendRequest({
    id: createRequestId(),
    action: "history",
    params,
    client: {
      component: "cli",
      version: VERSION,
      baseVersion: BASE_VERSION,
      gitSha: GIT_SHA,
      dirty: DIRTY,
    },
  });

  printJson(response, prettyOutput);
  if (!response.ok) {
    process.exit(1);
  }
}

// ============================================================================
// Undo Command
// ============================================================================

export async function runUndo(options: Options, prettyOutput: boolean): Promise<void> {
  const positionalTxid = options._[0];
  const flagTxid = options.txid;
  const useLatest = options.latest === true;

  if (useLatest && (positionalTxid || flagTxid)) {
    errorOut("--latest cannot be combined with a txid argument or --txid");
  }

  const txid = positionalTxid || flagTxid;

  const params: Record<string, unknown> = {
    txid,
    latest: useLatest,
  };

  const response = await sendRequest({
    id: createRequestId(),
    action: "undo",
    params,
    client: {
      component: "cli",
      version: VERSION,
      baseVersion: BASE_VERSION,
      gitSha: GIT_SHA,
      dirty: DIRTY,
    },
  });

  printJson(response, prettyOutput);
  if (!response.ok) {
    process.exit(1);
  }
}

// ============================================================================
// Ping Command
// ============================================================================

export async function runPing(prettyOutput: boolean): Promise<void> {
  const response = await sendRequest({
    id: createRequestId(),
    action: "ping",
    params: {},
    client: {
      component: "cli",
      version: VERSION,
      baseVersion: BASE_VERSION,
      gitSha: GIT_SHA,
      dirty: DIRTY,
    },
  });

  printJson(response, prettyOutput);
  if (!response.ok) {
    process.exit(1);
  }
}
