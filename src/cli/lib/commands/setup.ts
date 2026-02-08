/**
 * Setup command handler: interactive browser profile configuration.
 * Extracted from meta.ts for modularity.
 */

import fs from "fs";
import os from "os";
import path from "path";
import readline from "readline";
import { spawn } from "node:child_process";

import { HOST_NAME, HOST_DESCRIPTION, EXTENSION_ID_PATTERN, resolveConfig } from "../constants";
import { printJson, errorOut } from "../output";
import type { Options } from "../types";
import { addProfile, validateProfileName } from "../../../shared/profiles";
import { resetConfig } from "../../../shared/config";
import { syncExtension, syncHost, deriveExtensionId, resolveInstalledExtensionDir } from "../../../shared/extension-sync";

export function resolveBrowser(value: unknown): "edge" | "chrome" | "chrome-canary" | "chrome-beta" | "chrome-dev" | "chromium" | "brave" | "opera" | "vivaldi" | null {
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim().toLowerCase();
  const valid = ["edge", "chrome", "chrome-canary", "chrome-beta", "chrome-dev", "chromium", "brave", "opera", "vivaldi"];
  if (valid.includes(trimmed)) {
    return trimmed as "edge" | "chrome" | "chrome-canary" | "chrome-beta" | "chrome-dev" | "chromium" | "brave" | "opera" | "vivaldi";
  }
  return null;
}

export function resolveExtensionId(options: Options, required: true): string;
export function resolveExtensionId(options: Options, required: false): string | null;
export function resolveExtensionId(options: Options, required: boolean): string | null {
  const raw = typeof options["extension-id"] === "string"
    ? String(options["extension-id"])
    : (process.env.TABCTL_EXTENSION_ID || "");
  const value = raw.trim().toLowerCase();
  if (!value) {
    if (!required) return null;
    errorOut("Missing --extension-id (or TABCTL_EXTENSION_ID)");
  }
  if (!EXTENSION_ID_PATTERN.test(value)) {
    errorOut(`Extension ID looks unusual: ${raw}`);
  }
  return value;
}

export async function promptExtensionId(browser: string): Promise<string> {
  const maxAttempts = 3;
  // Map browser to extensions page URL (all Chromium browsers use chrome:// or edge:// scheme)
  let extPage: string;
  if (browser === "edge") {
    extPage = "edge://extensions";
  } else if (browser === "opera") {
    extPage = "opera://extensions";
  } else if (browser === "vivaldi") {
    extPage = "vivaldi://extensions";
  } else {
    // chrome, chrome-canary, chrome-beta, chrome-dev, chromium, brave all use chrome://extensions
    extPage = "chrome://extensions";
  }
  
  const instructions = [
    "",
    "Next steps:",
    `  1. Open ${extPage}`,
    "  2. Enable Developer mode",
    '  3. Click "Load unpacked" and select the path above',
    "  4. Copy the extension ID shown on the extensions page",
    "",
  ].join("\n");
  process.stderr.write(instructions);

  // Collect lines from stdin and provide them on demand
  const lines: string[] = [];
  let closed = false;
  let waiting: ((line: string | null) => void) | null = null;

  const rl = readline.createInterface({ input: process.stdin, output: process.stderr, terminal: false });
  rl.on("line", (line) => {
    if (waiting) {
      const cb = waiting;
      waiting = null;
      cb(line.trim());
    } else {
      lines.push(line.trim());
    }
  });
  rl.on("close", () => {
    closed = true;
    if (waiting) {
      const cb = waiting;
      waiting = null;
      cb(null);
    }
  });

  const nextLine = (prompt: string): Promise<string | null> => {
    process.stderr.write(prompt);
    if (lines.length > 0) {
      return Promise.resolve(lines.shift()!);
    }
    if (closed) return Promise.resolve(null);
    return new Promise((resolve) => { waiting = resolve; });
  };

  try {
    for (let attempt = 1; attempt <= maxAttempts; attempt++) {
      const raw = await nextLine("Paste the extension ID: ");
      if (raw === null) {
        errorOut("No input received (stdin closed).");
      }
      const value = raw.toLowerCase();
      if (EXTENSION_ID_PATTERN.test(value)) {
        return value;
      }
      const remaining = maxAttempts - attempt;
      if (remaining > 0) {
        process.stderr.write(`Invalid extension ID (expected 32 lowercase a-p characters). ${remaining} attempt(s) remaining.\n`);
      } else {
        errorOut("Invalid extension ID after 3 attempts.");
      }
    }
  } finally {
    rl.close();
  }
  // unreachable due to errorOut, but satisfies TypeScript
  return "";
}

export function resolveNodePath(options: Options): string {
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
  if (process.platform !== "win32") {
    try {
      fs.accessSync(value, fs.constants.X_OK);
    } catch {
      errorOut(`Node binary not executable: ${value}`);
    }
  } else {
    try {
      fs.accessSync(value, fs.constants.R_OK);
    } catch {
      errorOut(`Node binary not found: ${value}`);
    }
  }
  return value;
}

function resolveHostPath(dataDir: string): string {
  // Sync host bundle to stable path so wrapper survives npm upgrades
  try {
    const result = syncHost(dataDir);
    return result.hostPath;
  } catch (err) {
    const detail = err instanceof Error ? err.message : String(err);
    errorOut(`Failed to resolve native host. Make sure the CLI is built (run: npm run build). Details: ${detail}`);
  }
}

export function resolveManifestDir(browser: "edge" | "chrome" | "chrome-canary" | "chrome-beta" | "chrome-dev" | "chromium" | "brave" | "opera" | "vivaldi"): string {
  const home = os.homedir();
  if (!home) {
    errorOut("Home directory not found.");
  }
  
  if (process.platform === "win32") {
    // Windows: registry-based is preferred, but file-based works with --user-data-dir.
    // For system-wide, we point to the per-user NativeMessagingHosts under LOCALAPPDATA.
    const base = process.env.LOCALAPPDATA || path.join(home, "AppData", "Local");
    
    const browserPaths: Record<string, string> = {
      "edge": path.join(base, "Microsoft", "Edge", "User Data", "NativeMessagingHosts"),
      "chrome": path.join(base, "Google", "Chrome", "User Data", "NativeMessagingHosts"),
      "chrome-canary": path.join(base, "Google", "Chrome SxS", "User Data", "NativeMessagingHosts"),
      "chrome-beta": path.join(base, "Google", "Chrome Beta", "User Data", "NativeMessagingHosts"),
      "chrome-dev": path.join(base, "Google", "Chrome Dev", "User Data", "NativeMessagingHosts"),
      "chromium": path.join(base, "Chromium", "User Data", "NativeMessagingHosts"),
      "brave": path.join(base, "BraveSoftware", "Brave-Browser", "User Data", "NativeMessagingHosts"),
      "opera": path.join(base, "Opera Software", "Opera Stable", "User Data", "NativeMessagingHosts"),
      "vivaldi": path.join(base, "Vivaldi", "User Data", "NativeMessagingHosts"),
    };
    return browserPaths[browser];
  }
  
  if (process.platform === "linux") {
    const browserPaths: Record<string, string> = {
      "edge": path.join(home, ".config", "microsoft-edge", "NativeMessagingHosts"),
      "chrome": path.join(home, ".config", "google-chrome", "NativeMessagingHosts"),
      "chrome-canary": path.join(home, ".config", "google-chrome-canary", "NativeMessagingHosts"),
      "chrome-beta": path.join(home, ".config", "google-chrome-beta", "NativeMessagingHosts"),
      "chrome-dev": path.join(home, ".config", "google-chrome-unstable", "NativeMessagingHosts"),
      "chromium": path.join(home, ".config", "chromium", "NativeMessagingHosts"),
      "brave": path.join(home, ".config", "BraveSoftware", "Brave-Browser", "NativeMessagingHosts"),
      "opera": path.join(home, ".config", "opera", "NativeMessagingHosts"),
      "vivaldi": path.join(home, ".config", "vivaldi", "NativeMessagingHosts"),
    };
    return browserPaths[browser];
  }
  
  // macOS
  const browserPaths: Record<string, string> = {
    "edge": path.join(home, "Library", "Application Support", "Microsoft Edge", "NativeMessagingHosts"),
    "chrome": path.join(home, "Library", "Application Support", "Google", "Chrome", "NativeMessagingHosts"),
    "chrome-canary": path.join(home, "Library", "Application Support", "Google", "Chrome Canary", "NativeMessagingHosts"),
    "chrome-beta": path.join(home, "Library", "Application Support", "Google", "Chrome Beta", "NativeMessagingHosts"),
    "chrome-dev": path.join(home, "Library", "Application Support", "Google", "Chrome Dev", "NativeMessagingHosts"),
    "chromium": path.join(home, "Library", "Application Support", "Chromium", "NativeMessagingHosts"),
    "brave": path.join(home, "Library", "Application Support", "BraveSoftware", "Brave-Browser", "NativeMessagingHosts"),
    "opera": path.join(home, "Library", "Application Support", "com.operasoftware.Opera", "NativeMessagingHosts"),
    "vivaldi": path.join(home, "Library", "Application Support", "Vivaldi", "NativeMessagingHosts"),
  };
  return browserPaths[browser];
}

export function writeWrapper(nodePath: string, hostPath: string, profileName: string | null, wrapperDir: string): string {
  fs.mkdirSync(wrapperDir, { recursive: true });
  if (process.platform !== "win32") {
    try { fs.chmodSync(wrapperDir, 0o700); } catch { /* ignore */ }
  }

  if (process.platform === "win32") {
    // Prefer the Go launcher binary from the platform package.
    // Falls back to a .cmd wrapper if unavailable (dev/testing only —
    // .cmd wrappers don't work for Chrome native messaging).
    let exeSrc: string | undefined;
    try {
      exeSrc = require.resolve("tabctl-win32-x64/tabctl-host.exe");
    } catch {
      // Not installed
    }

    if (exeSrc) {
      const exeDst = path.join(wrapperDir, "tabctl-host.exe");
      fs.copyFileSync(exeSrc, exeDst);

      const cfgLines = [nodePath, hostPath];
      if (profileName) {
        cfgLines.push(`TABCTL_PROFILE=${profileName}`);
      }
      cfgLines.push("");
      fs.writeFileSync(path.join(wrapperDir, "host-launcher.cfg"), cfgLines.join("\r\n"), "utf8");

      return exeDst;
    }

    // Fallback: .cmd wrapper (won't work with Chrome native messaging)
    const wrapperPath = path.join(wrapperDir, "tabctl-host.cmd");
    const lines = ["@echo off"];
    if (profileName) {
      lines.push(`set TABCTL_PROFILE=${profileName}`);
    }
    lines.push(`"${nodePath}" "${hostPath}" %*`);
    lines.push("");
    fs.writeFileSync(wrapperPath, lines.join("\r\n"), "utf8");
    return wrapperPath;
  }

  const wrapperPath = path.join(wrapperDir, "tabctl-host.sh");
  const escapedNode = nodePath.replace(/"/g, "\\\"");
  const escapedHost = hostPath.replace(/"/g, "\\\"");
  const lines = [
    "#!/usr/bin/env bash",
    "set -euo pipefail",
  ];
  if (profileName) {
    lines.push(`export TABCTL_PROFILE="${profileName}"`);
  }
  lines.push(`exec \"${escapedNode}\" \"${escapedHost}\"`);
  lines.push("");
  const wrapper = lines.join("\n");
  fs.writeFileSync(wrapperPath, wrapper, "utf8");
  fs.chmodSync(wrapperPath, 0o700);
  return wrapperPath;
}

function getBrowserDisplayName(browser: string): string {
  const displayNames: Record<string, string> = {
    "edge": "Edge",
    "chrome": "Chrome",
    "chrome-canary": "Chrome Canary",
    "chrome-beta": "Chrome Beta",
    "chrome-dev": "Chrome Dev",
    "chromium": "Chromium",
    "brave": "Brave",
    "opera": "Opera",
    "vivaldi": "Vivaldi",
  };
  return displayNames[browser] || browser;
}

export async function runSetup(options: Options, prettyOutput: boolean): Promise<void> {
  const browser = resolveBrowser(options.browser);
  if (!browser) {
    errorOut("Missing or invalid --browser (edge|chrome|chrome-canary|chrome-beta|chrome-dev|chromium|brave|opera|vivaldi)");
  }

  const nodePath = resolveNodePath(options);

  // Sync extension + host to stable paths (before extensionId so interactive mode can show it)
  const config = resolveConfig();
  const hostPath = resolveHostPath(config.baseDataDir);
  let extensionSync;
  try {
    extensionSync = syncExtension(config.baseDataDir);
  } catch {
    extensionSync = null;
  }

  // Resolve extension ID: explicit flag, derived from install path, or interactive prompt
  let extensionId = resolveExtensionId(options, false);
  if (!extensionId) {
    // Auto-derive from the installed extension path (Chromium uses SHA256 of the path)
    const installedDir = resolveInstalledExtensionDir(config.baseDataDir);
    if (fs.existsSync(path.join(installedDir, "manifest.json"))) {
      extensionId = deriveExtensionId(installedDir);
      process.stderr.write(`Extension ID derived from: ${installedDir}\n`);
    }
  }
  if (!extensionId) {
    // Interactive mode: sync hadn't happened or path doesn't exist
    if (extensionSync?.extensionDir) {
      process.stderr.write(`\nExtension synced to: ${extensionSync.extensionDir}\n`);
      try {
        const clipArgs: string[] = [];
        let clipCmd: string;
        if (process.platform === "darwin") {
          clipCmd = "pbcopy";
        } else if (process.platform === "win32") {
          clipCmd = "clip";
        } else {
          clipCmd = "xclip";
          clipArgs.push("-selection", "clipboard");
        }
        const clip = spawn(clipCmd, clipArgs, { stdio: ["pipe", "ignore", "ignore"] });
        clip.stdin.end(extensionSync.extensionDir);
        clip.on("exit", (code) => {
          if (code === 0) process.stderr.write("(Path copied to clipboard)\n");
        });
      } catch {
        // clipboard copy is best-effort
      }
    }
    extensionId = await promptExtensionId(browser);
  }

  // Profile name: --name flag or browser type
  const profileName = typeof options.name === "string" && options.name.trim()
    ? options.name.trim().toLowerCase()
    : browser;

  try {
    validateProfileName(profileName);
  } catch (err) {
    errorOut((err as Error).message);
  }

  // Profile data dir (use baseDataDir to avoid nesting under another profile)
  const profileDataDir = path.join(config.baseDataDir, "profiles", profileName);
  fs.mkdirSync(profileDataDir, { recursive: true });

  // Write profile-specific wrapper
  const wrapperPath = writeWrapper(nodePath, hostPath, profileName, profileDataDir);

  // Resolve manifest directory: custom user-data-dir or system-wide
  const rawUserDataDir = typeof options["user-data-dir"] === "string"
    ? options["user-data-dir"].trim()
    : "";
  const userDataDir = rawUserDataDir ? path.resolve(rawUserDataDir) : "";
  const manifestDir = userDataDir
    ? path.join(userDataDir, "NativeMessagingHosts")
    : resolveManifestDir(browser);
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

  // Register profile
  const profileEntry: Parameters<typeof addProfile>[1] = {
    browser,
    extensionId,
    nodePath,
    hostPath,
    dataDir: profileDataDir,
  };
  if (userDataDir) {
    profileEntry.userDataDir = userDataDir;
  }
  const registry = addProfile(profileName, profileEntry);

  // Ensure printJson footer reflects the newly-created profile
  resetConfig();
  process.env.TABCTL_PROFILE = profileName;

  printJson({
    ok: true,
    action: "setup",
    data: {
      profileName,
      browser,
      extensionId,
      manifestPath,
      hostPath,
      nodePath,
      wrapperPath,
      dataDir: profileDataDir,
      ...(userDataDir ? { userDataDir } : {}),
      isDefault: registry.default === profileName,
      extensionDir: extensionSync?.extensionDir || null,
      extensionSynced: extensionSync?.synced || false,
    },
  }, prettyOutput);

  if (registry.default !== profileName) {
    process.stderr.write([
      "",
      `Profile "${profileName}" created (current default: "${registry.default}").`,
      `  To use:          tabctl --profile ${profileName} <command>`,
      `  To make default: tabctl profile-switch ${profileName}`,
      "",
    ].join("\n"));
  }
  process.stderr.write([
    `Verify connection: tabctl --profile ${profileName} ping`,
    `If ping fails, ensure the ${getBrowserDisplayName(browser)} extension is active.`,
    "",
  ].join("\n"));
}
