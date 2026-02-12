/**
 * Setup command handler: browser profile configuration.
 * Extracted from meta.ts for modularity.
 */

import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { HOST_NAME, HOST_DESCRIPTION, EXTENSION_ID_PATTERN, resolveConfig } from "../constants";
import { printJson, errorOut } from "../output";
import type { Options } from "../types";
import { addProfile, validateProfileName } from "../../../shared/profiles";
import { resetConfig } from "../../../shared/config";
import { syncExtension, syncHost, deriveExtensionId, resolveInstalledExtensionDir } from "../../../shared/extension-sync";

export function resolveBrowser(value: unknown): "edge" | "chrome" | null {
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim().toLowerCase();
  if (trimmed === "edge" || trimmed === "chrome") {
    return trimmed;
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
    const result = syncHost(dataDir, { force: true });
    return result.hostPath;
  } catch (err) {
    const detail = err instanceof Error ? err.message : String(err);
    errorOut(`Failed to resolve native host. Make sure the CLI is built (run: npm run build). Details: ${detail}`);
  }
}

export function resolveManifestDir(browser: "edge" | "chrome"): string {
  const home = os.homedir();
  if (!home) {
    errorOut("Home directory not found.");
  }
  if (process.platform === "win32") {
    // Windows: registry-based is preferred, but file-based works with --user-data-dir.
    // For system-wide, we point to the per-user NativeMessagingHosts under LOCALAPPDATA.
    const base = process.env.LOCALAPPDATA || path.join(home, "AppData", "Local");
    if (browser === "edge") {
      return path.join(base, "Microsoft", "Edge", "User Data", "NativeMessagingHosts");
    }
    return path.join(base, "Google", "Chrome", "User Data", "NativeMessagingHosts");
  }
  if (process.platform === "linux") {
    if (browser === "edge") {
      return path.join(home, ".config", "microsoft-edge", "NativeMessagingHosts");
    }
    return path.join(home, ".config", "google-chrome", "NativeMessagingHosts");
  }
  // macOS
  if (browser === "edge") {
    return path.join(home, "Library", "Application Support", "Microsoft Edge", "NativeMessagingHosts");
  }
  return path.join(home, "Library", "Application Support", "Google", "Chrome", "NativeMessagingHosts");
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

export function runSetup(options: Options, prettyOutput: boolean): void {
  const browser = resolveBrowser(options.browser);
  if (!browser) {
    errorOut("Missing or invalid --browser (edge|chrome)");
  }

  const nodePath = resolveNodePath(options);

  // Sync extension + host to stable paths (before extensionId so interactive mode can show it)
  const config = resolveConfig();
  const hostPath = resolveHostPath(config.baseDataDir);
  let extensionSync;
  try {
    extensionSync = syncExtension(config.baseDataDir, { force: true });
  } catch {
    extensionSync = null;
  }

  // Resolve extension ID: explicit flag, derived from install path, or interactive prompt
  let extensionId = resolveExtensionId(options, false);
  if (!extensionId) {
    // Auto-derive from the installed extension path (Chromium uses SHA256 of the path)
    // Prefer the just-synced path; fall back to resolving independently
    const installedDir = extensionSync?.extensionDir ?? resolveInstalledExtensionDir(config.baseDataDir);
    if (fs.existsSync(path.join(installedDir, "manifest.json"))) {
      extensionId = deriveExtensionId(installedDir);
      process.stderr.write(`Extension ID derived from: ${installedDir}\n`);
    }
  }
  if (!extensionId) {
    errorOut("Could not derive extension ID (extension not synced). Use --extension-id <id> or set TABCTL_EXTENSION_ID.");
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
    `If ping fails, ensure the ${browser === "edge" ? "Edge" : "Chrome"} extension is active.`,
    "",
  ].join("\n"));
}
