import fs from "fs";
import os from "os";
import path from "path";

export type Browser = "edge" | "chrome";

export type TabctlConfig = {
  browser?: Browser;
  socketName?: string;
};

export function configHome(): string {
  return process.env.XDG_CONFIG_HOME || path.join(os.homedir(), ".config");
}

export function configPath(): string {
  return path.join(configHome(), "tabctl", "config.json");
}

export function stateHome(): string {
  return process.env.XDG_STATE_HOME || path.join(os.homedir(), ".local", "state");
}

export function parseBrowser(value: unknown): Browser | null {
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim().toLowerCase();
  if (trimmed === "edge" || trimmed === "chrome") {
    return trimmed;
  }
  return null;
}

export function parseSocketName(value: unknown): string | null {
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim();
  if (!trimmed) {
    return null;
  }
  if (trimmed.includes("/") || trimmed.includes("\\")) {
    return null;
  }
  return trimmed;
}

export function loadConfig(): TabctlConfig | null {
  const resolvedPath = configPath();
  if (!fs.existsSync(resolvedPath)) {
    return null;
  }
  try {
    const raw = fs.readFileSync(resolvedPath, "utf8");
    const parsed = JSON.parse(raw) as Record<string, unknown> | null;
    if (parsed === null || typeof parsed !== "object") {
      return null;
    }
    const config: TabctlConfig = {};
    const browser = parseBrowser(parsed.browser);
    if (browser) {
      config.browser = browser;
    }
    const socketName = parseSocketName(parsed.socketName);
    if (socketName) {
      config.socketName = socketName;
    }
    return config;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    process.stderr.write(`[tabctl] Failed to read config at ${resolvedPath}: ${message}\n`);
    return null;
  }
}

export function resolveBrowser(config: TabctlConfig | null): Browser {
  return config?.browser === "edge" ? "edge" : "chrome";
}

export function resolveSocketName(config: TabctlConfig | null, browser: Browser): string {
  const socketName = config?.socketName ? config.socketName : "";
  if (socketName) {
    return socketName.endsWith(".sock") ? socketName : `${socketName}.sock`;
  }
  return browser === "chrome" ? "tabctl-chrome.sock" : "tabctl-edge.sock";
}

export function resolveSocketPath(stateHome: string, browser: Browser, config?: TabctlConfig | null): string {
  const socketName = resolveSocketName(config || null, browser);
  return path.join(stateHome, "tabctl", socketName);
}
