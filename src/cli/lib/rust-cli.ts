import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { BASE_VERSION, DIRTY, GIT_SHA, VERSION, resolveConfig } from "./constants";
import { errorOut } from "./output";

type CliImplementationMode = "node" | "rust-auto" | "rust-force";

const RUST_IMPL_FORCE_VALUES = new Set(["1", "true", "rust"]);
const RUST_IMPL_DISABLE_VALUES = new Set(["0", "false", "node"]);
const RUST_DELEGATED_COMMANDS = new Set([
  "version",
  "ping",
  "history",
  "open",
  "focus",
  "refresh",
  "group-update",
  "group-ungroup",
  "group-assign",
  "group-gather",
  "move-tab",
  "move-group",
  "merge-window",
  "archive",
  "close",
  "reload",
]);

function resolveCliImplementationMode(): CliImplementationMode {
  const value = String(process.env.TABCTL_CLI_IMPL || "").trim().toLowerCase();
  if (!value || value === "auto" || value === "default") {
    return "rust-auto";
  }
  if (RUST_IMPL_FORCE_VALUES.has(value)) {
    return "rust-force";
  }
  if (RUST_IMPL_DISABLE_VALUES.has(value)) {
    return "node";
  }
  return "node";
}

function shouldUseRustCli(mode: CliImplementationMode): boolean {
  return mode !== "node";
}

function rustCliCandidates(mode: CliImplementationMode): string[] {
  const explicit = String(process.env.TABCTL_RUST_CLI_BIN || "").trim();
  if (explicit) {
    return [explicit];
  }
  const executable = process.platform === "win32"
    ? "tabctl-rust-cli-readonly.exe"
    : "tabctl-rust-cli-readonly";
  const bundled = path.resolve(__dirname, "..", "rust", executable);
  const candidates: string[] = [];
  if (fs.existsSync(bundled)) {
    candidates.push(bundled);
  }
  if (mode === "rust-force") {
    candidates.push("tabctl-rust-cli-readonly");
  }
  return candidates;
}

function resolveRustInvocation(binary: string, args: string[]): { command: string; args: string[] } {
  if (binary.endsWith(".js")) {
    return { command: process.execPath, args: [binary, ...args] };
  }
  return { command: binary, args };
}

function buildRustEnv(command: string): NodeJS.ProcessEnv {
  const env = { ...process.env };
  env.TABCTL_VERSION = VERSION;
  env.TABCTL_BASE_VERSION = BASE_VERSION;
  env.TABCTL_GIT_SHA = GIT_SHA || "";
  env.TABCTL_DIRTY = DIRTY ? "1" : "0";
  env.TABCTL_NODE_EXEC = process.execPath;
  if (typeof process.argv[1] === "string" && process.argv[1].trim()) {
    env.TABCTL_NODE_CLI_BIN = process.argv[1];
  }

  if (command !== "version" && !env.TABCTL_SOCKET) {
    try {
      env.TABCTL_SOCKET = resolveConfig().socketPath;
    } catch {
      // Fall back to Rust defaults if Node config lookup fails.
    }
  }

  return env;
}

export function maybeDelegateToRustCli(command: string | undefined, args: string[]): boolean {
  const mode = resolveCliImplementationMode();
  if (!command || !RUST_DELEGATED_COMMANDS.has(command) || !shouldUseRustCli(mode)) {
    return false;
  }

  const env = buildRustEnv(command);
  const candidates = rustCliCandidates(mode);

  for (const candidate of candidates) {
    const invocation = resolveRustInvocation(candidate, args);
    const result = spawnSync(invocation.command, invocation.args, { stdio: "inherit", env });
    if (result.error) {
      const code = (result.error as NodeJS.ErrnoException).code;
      if (code === "ENOENT") {
        continue;
      }
      errorOut(`Failed to run Rust CLI: ${result.error.message}`);
    }
    process.exit(typeof result.status === "number" ? result.status : 1);
  }

  return false;
}
