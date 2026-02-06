import fs from "fs";
import os from "os";
import path from "path";

export type Browser = "edge" | "chrome";

export type TabctlConfig = {
  browser?: Browser;
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
    const browser = parsed.browser;
    if (browser === "edge" || browser === "chrome") {
      return { browser };
    }
    return {};
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    process.stderr.write(`[tabctl] Failed to read config at ${resolvedPath}: ${message}\n`);
    return null;
  }
}

export function resolveBrowser(config: TabctlConfig | null): Browser {
  return config?.browser === "chrome" ? "chrome" : "edge";
}

export function resolveSocketPath(stateHome: string, browser: Browser): string {
  const socketName = browser === "chrome" ? "tabctl-chrome.sock" : "tabctl.sock";
  return path.join(stateHome, "tabctl", socketName);
}
