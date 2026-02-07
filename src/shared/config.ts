import os from "os";
import path from "path";
import fs from "fs";

export type TabctlConfig = {
  configDir: string;
  dataDir: string;
  baseDataDir: string;
  socketPath: string;
  undoLog: string;
  wrapperDir: string;
  policyPath: string;
  activeProfileName?: string;
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

export function resolveConfig(profileName?: string): TabctlConfig {
  // Use cache only for no-arg calls (legacy mode)
  if (!profileName && cached) return cached;

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

  const baseDataDir = dataDir;

  // Profile resolution (read profiles.json inline to avoid circular import)
  const explicitProfile = profileName;
  const effectiveProfile = profileName || process.env.TABCTL_PROFILE;
  let activeProfileName: string | undefined;

  if (effectiveProfile) {
    try {
      const raw = fs.readFileSync(path.join(configDir, "profiles.json"), "utf-8");
      const registry = JSON.parse(raw) as { default: string | null; profiles: Record<string, { dataDir: string }> };
      const profile = registry.profiles[effectiveProfile];
      if (profile) {
        dataDir = profile.dataDir;
        activeProfileName = effectiveProfile;
      } else if (explicitProfile) {
        throw new Error(`Profile "${explicitProfile}" not found in profiles.json`);
      }
    } catch (err) {
      // Re-throw profile-not-found errors
      if (err instanceof Error && err.message.includes("not found in profiles.json")) {
        throw err;
      }
      // If profiles.json is missing/malformed and profile was explicitly requested, error
      if (explicitProfile) {
        throw new Error(`Profile "${explicitProfile}" not found: no profiles.json`);
      }
      // Otherwise (env var only), silently fall through to legacy mode
    }
  } else {
    // No explicit profile — check for a default in profiles.json
    try {
      const raw = fs.readFileSync(path.join(configDir, "profiles.json"), "utf-8");
      const registry = JSON.parse(raw) as { default: string | null; profiles: Record<string, { dataDir: string }> };
      if (registry.default && registry.profiles[registry.default]) {
        dataDir = registry.profiles[registry.default].dataDir;
        activeProfileName = registry.default;
      }
    } catch {
      // No profiles.json — legacy mode
    }
  }

  const result: TabctlConfig = {
    configDir,
    dataDir,
    baseDataDir,
    socketPath: process.env.TABCTL_SOCKET || path.join(dataDir, "tabctl.sock"),
    undoLog: path.join(dataDir, "undo.jsonl"),
    wrapperDir: dataDir,
    policyPath: path.join(configDir, "policy.json"),
    activeProfileName,
  };

  // Only cache no-arg calls
  if (!profileName) {
    cached = result;
  }

  return result;
}
