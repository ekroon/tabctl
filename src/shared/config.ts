import os from "node:os";
import path from "node:path";
import crypto from "node:crypto";
import fs from "node:fs";

function defaultConfigBase(): string {
  if (process.platform === "win32") {
    return process.env.APPDATA || path.join(os.homedir(), "AppData", "Roaming");
  }
  return path.join(os.homedir(), ".config");
}

function defaultStateBase(): string {
  if (process.platform === "win32") {
    return process.env.LOCALAPPDATA || path.join(os.homedir(), "AppData", "Local");
  }
  return path.join(os.homedir(), ".local", "state");
}

/** Detect if running in WSL */
export function isWSL(): boolean {
  if (process.platform !== "linux") return false;
  return Boolean(process.env.WSL_DISTRO_NAME || process.env.WSL_INTEROP);
}

/** Resolve the IPC socket/pipe path for the given data directory. */
/** Default TCP port for WSL → Windows communication. Host tries this first, then port+1, port+2, etc. */
export const DEFAULT_WSL_TCP_PORT = 24050;

export function resolveSocketPath(dataDir: string): string {
  // WSL should default to TCP to connect to Windows host
  if (isWSL()) {
    // Try to read port from Windows host's port file
    const port = readTcpPortFromHost(dataDir);
    if (port) {
      return `tcp:${port}`;
    }
    // Fallback to default port if file not found (host not running yet)
    return `tcp:${DEFAULT_WSL_TCP_PORT}`;
  }
  if (process.platform === "win32") {
    // Windows: use named pipes (Unix domain sockets are unreliable)
    const hash = crypto.createHash("sha256").update(dataDir).digest("hex").slice(0, 12);
    return `\\\\.\\pipe\\tabctl-${hash}`;
  }
  return path.join(dataDir, "tabctl.sock");
}

/** Read TCP port from Windows host's port file (WSL → Windows access) */
export function readTcpPortFromHost(dataDir: string): number | null {
  try {
    // Convert WSL path to Windows path that we can read via /mnt/c/...
    // WSL path: /home/user/.local/state/tabctl/profiles/chrome
    // Windows path: C:\Users\user\AppData\Local\tabctl-state\tabctl\profiles\chrome
    // Accessible from WSL as: /mnt/c/Users/user/AppData/Local/tabctl-state/tabctl/profiles/chrome
    
    const windowsDataDir = convertWSLPathToWindowsMount(dataDir);
    if (!windowsDataDir) {
      return null; // Can't determine Windows path
    }
    
    const portFile = path.join(windowsDataDir, "tcp-port.txt");
    if (!fs.existsSync(portFile)) {
      return null; // Port file doesn't exist (host not running)
    }
    
    const content = fs.readFileSync(portFile, "utf8").trim();
    const port = parseInt(content, 10);
    
    if (isNaN(port) || port < 1024 || port > 65535) {
      return null; // Invalid port
    }
    
    return port;
  } catch {
    return null; // Any error, fall back to calculated port
  }
}

/** Extract Windows username from $PATH in WSL */
export function getWindowsUsernameFromPath(): string | null {
  const pathEnv = process.env.PATH;
  if (!pathEnv) {
    return null;
  }
  
  // Look for /mnt/c/Users/<username>/ in PATH
  // Typical PATH entry: /mnt/c/Users/KroonRaymond/AppData/Local/Programs/...
  const match = pathEnv.match(/\/mnt\/c\/Users\/([^/:]+)/);
  if (match) {
    return match[1];
  }
  
  return null;
}

/** Convert WSL path to Windows mount path for reading Windows files */
function convertWSLPathToWindowsMount(wslPath: string): string | null {
  // Find Windows username from $PATH (WSL username might differ)
  const windowsUsername = getWindowsUsernameFromPath();
  if (!windowsUsername) {
    return null; // Can't determine Windows username
  }
  
  const homeDir = os.homedir(); // e.g., /home/raymond
  const stateHome = process.env.XDG_STATE_HOME || path.join(homeDir, ".local", "state");
  const normalized = path.normalize(wslPath);
  
  if (!normalized.startsWith(path.normalize(stateHome))) {
    return null; // Not in standard location
  }
  
  // Extract the path relative to state home
  // e.g., tabctl/profiles/chrome
  const relativePath = path.relative(stateHome, normalized);
  
  // Construct Windows mount path using detected Windows username
  // Try: /mnt/c/Users/KroonRaymond/AppData/Local/tabctl/profiles/chrome
  const windowsMountPath = path.join("/mnt/c/Users", windowsUsername, "AppData/Local", relativePath);
  
  if (fs.existsSync(windowsMountPath)) {
    return windowsMountPath;
  }
  
  // Fallback: Try with tabctl-state subdirectory
  // e.g., /mnt/c/Users/KroonRaymond/AppData/Local/tabctl-state/tabctl/profiles/chrome
  const altPath = path.join("/mnt/c/Users", windowsUsername, "AppData/Local/tabctl-state", relativePath);
  if (fs.existsSync(altPath)) {
    return altPath;
  }
  
  return null; // Could not find Windows path
}

/** Write TCP port to file for WSL access (Windows host) */
export function writeTcpPortForWSL(dataDir: string, port: number): void {
  try {
    const portFile = path.join(dataDir, "tcp-port.txt");
    fs.writeFileSync(portFile, port.toString(), "utf8");
  } catch (error) {
    // Non-fatal: WSL users can still connect via calculated port
  }
}

/** Parse socket path to determine connection type and address */
export function parseSocketPath(socketPath: string): { type: "pipe" | "unix" | "tcp"; path?: string; host?: string; port?: number } {
  // TCP format: tcp://host:port or tcp:port
  if (socketPath.startsWith("tcp://")) {
    const url = new URL(socketPath);
    return { type: "tcp", host: url.hostname || "127.0.0.1", port: parseInt(url.port, 10) };
  }
  if (socketPath.startsWith("tcp:")) {
    const port = parseInt(socketPath.slice(4), 10);
    return { type: "tcp", host: "127.0.0.1", port };
  }
  // Windows pipe format
  if (socketPath.startsWith("\\\\.\\pipe\\")) {
    return { type: "pipe", path: socketPath };
  }
  // Unix socket
  return { type: "unix", path: socketPath };
}

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
    || path.join(process.env.XDG_CONFIG_HOME || defaultConfigBase(), "tabctl");

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
      process.env.XDG_STATE_HOME || defaultStateBase(),
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
    socketPath: process.env.TABCTL_SOCKET || resolveSocketPath(dataDir),
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
