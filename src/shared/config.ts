import os from "os";
import path from "path";
import fs from "fs";

export type TabctlConfig = {
  configDir: string;
  dataDir: string;
  socketPath: string;
  undoLog: string;
  wrapperDir: string;
  policyPath: string;
};

let cached: TabctlConfig | undefined;

export function resetConfig(): void {
  cached = undefined;
}

export function expandEnvVars(value: string): string {
  return value.replace(/\$\{(\w+)\}|\$(\w+)/g, (match, braced, bare) => {
    const varName = braced || bare;
    return process.env[varName] ?? match;
  });
}

export function resolveConfig(): TabctlConfig {
  if (cached) return cached;

  // Config dir resolution
  const configDir = process.env.TABCTL_CONFIG_DIR
    || path.join(process.env.XDG_CONFIG_HOME || path.join(os.homedir(), ".config"), "tabctl");

  // Read optional config.json
  let fileConfig: Record<string, unknown> = {};
  try {
    const raw = fs.readFileSync(path.join(configDir, "config.json"), "utf-8");
    fileConfig = JSON.parse(raw);
  } catch {
    // missing or malformed — treat as empty
  }

  // Data dir resolution
  let dataDir: string;
  if (typeof fileConfig.dataDir === "string" && fileConfig.dataDir) {
    dataDir = expandEnvVars(fileConfig.dataDir);
    if (!path.isAbsolute(dataDir)) {
      throw new Error(`dataDir in config.json must be an absolute path (got: ${dataDir}). Use $HOME or full paths.`);
    }
  } else if (process.env.TABCTL_CONFIG_DIR) {
    dataDir = path.join(configDir, "data");
  } else {
    dataDir = path.join(
      process.env.XDG_STATE_HOME || path.join(os.homedir(), ".local", "state"),
      "tabctl",
    );
  }

  cached = {
    configDir,
    dataDir,
    socketPath: process.env.TABCTL_SOCKET || path.join(dataDir, "tabctl.sock"),
    undoLog: path.join(dataDir, "undo.jsonl"),
    wrapperDir: dataDir,
    policyPath: path.join(configDir, "policy.json"),
  };

  return cached;
}
