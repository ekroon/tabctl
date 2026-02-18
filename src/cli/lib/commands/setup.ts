/**
 * Setup command handler: browser profile configuration.
 * Extracted from meta.ts for modularity.
 */

import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import {
  HOST_NAME,
  HOST_DESCRIPTION,
  EXTENSION_ID_PATTERN,
  VERSION,
  BASE_VERSION,
  GIT_SHA,
  DIRTY,
  resolveConfig,
} from "../constants";
import { printJson, errorOut } from "../output";
import type { Options } from "../types";
import { addProfile, validateProfileName } from "../../../shared/profiles";
import { resetConfig } from "../../../shared/config";
import { syncExtension, syncHost, deriveExtensionId, resolveInstalledExtensionDir, canonicalizeExtensionPath } from "../../../shared/extension-sync";
import { sendRequest, createRequestId } from "../client";

export type RuntimeEnvironment = "native-win32" | "native-linux" | "native-darwin";

type SetupVerification = {
  attempted: boolean;
  ok: boolean;
  reason: string | null;
  detail: string | null;
  expectedExtensionId: string | null;
  runtimeExtensionId: string | null;
  socketPath: string | null;
  manualSteps: string[];
};

const SETUP_VERIFY_TIMEOUT_MS = 4000;

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

function buildManualWindowsSteps(
  browser: "edge" | "chrome",
  profileName: string,
  extensionDir: string | null,
  extensionId: string,
  manifestPath: string,
): string[] {
  const extensionsUrl = browser === "edge" ? "edge://extensions" : "chrome://extensions";
  const browserName = browser === "edge" ? "Edge" : "Chrome";
  const setupCommand = `tabctl setup --browser ${browser} --extension-id <id>`;
  return [
    `Open ${extensionsUrl} in ${browserName} and enable Developer mode.`,
    extensionDir
      ? `Load unpacked extension from: ${extensionDir}`
      : "Load unpacked extension from the path printed by tabctl setup.",
    `Confirm extension ID in ${extensionsUrl} (current expected: ${extensionId}).`,
    `If ID differs, rerun setup with explicit ID: ${setupCommand}`,
    `Verify native host manifest exists at: ${manifestPath}`,
    `Verify connection: tabctl --profile ${profileName} ping`,
  ];
}

async function verifyWindowsSetupConnectivity(
  profileName: string,
  browser: "edge" | "chrome",
  extensionDir: string | null,
  extensionId: string,
  manifestPath: string,
): Promise<SetupVerification> {
  if (process.platform !== "win32") {
    return {
      attempted: false,
      ok: true,
      reason: null,
      detail: null,
      expectedExtensionId: null,
      runtimeExtensionId: null,
      socketPath: null,
      manualSteps: [],
    };
  }

  let socketPath: string | null = null;
  try {
    socketPath = resolveConfig(profileName).socketPath;
  } catch {
    // Keep null and rely on generic message below.
  }
  const manualSteps = buildManualWindowsSteps(browser, profileName, extensionDir, extensionId, manifestPath);

  let timeoutHandle: ReturnType<typeof setTimeout> | null = null;
  try {
    const response = await Promise.race<Record<string, unknown>>([
      sendRequest({
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
      }),
      new Promise<Record<string, unknown>>((_, reject) => {
        timeoutHandle = setTimeout(() => {
          reject(new Error(`timed out after ${SETUP_VERIFY_TIMEOUT_MS}ms`));
        }, SETUP_VERIFY_TIMEOUT_MS);
      }),
    ]);
    if (timeoutHandle) {
      clearTimeout(timeoutHandle);
    }
    if (response.ok !== true) {
      const responseError = response.error as { message?: string } | undefined;
      return {
        attempted: true,
        ok: false,
        reason: "ping-not-ok",
        detail: responseError?.message || "ping returned non-ok response",
        expectedExtensionId: extensionId,
        runtimeExtensionId: null,
        socketPath,
        manualSteps,
      };
    }
    const responseData = response.data as Record<string, unknown> | undefined;
    const runtimeExtensionId = typeof responseData?.runtimeId === "string"
      ? responseData.runtimeId.trim().toLowerCase()
      : null;
    if (runtimeExtensionId && runtimeExtensionId !== extensionId) {
      return {
        attempted: true,
        ok: true,
        reason: "extension-id-mismatch",
        detail: `expected ${extensionId} but extension reported ${runtimeExtensionId}`,
        expectedExtensionId: extensionId,
        runtimeExtensionId,
        socketPath,
        manualSteps: [],
      };
    }
    return {
      attempted: true,
      ok: true,
      reason: null,
      detail: null,
      expectedExtensionId: extensionId,
      runtimeExtensionId,
      socketPath,
      manualSteps: [],
    };
  } catch (err) {
    if (timeoutHandle) {
      clearTimeout(timeoutHandle);
    }
    const detail = err instanceof Error ? err.message : String(err);
    const reason = detail.includes("timed out")
      ? "ping-timeout"
      : detail.includes("ENOENT")
        ? "socket-not-found"
        : detail.includes("ECONNREFUSED")
          ? "socket-refused"
          : "ping-error";
    return {
      attempted: true,
      ok: false,
      reason,
      detail,
      expectedExtensionId: extensionId,
      runtimeExtensionId: null,
      socketPath,
      manualSteps,
    };
  }
}

export function detectRuntimeEnvironment(): RuntimeEnvironment {
  if (process.platform === "win32") return "native-win32";
  if (process.platform === "darwin") return "native-darwin";
  return "native-linux";
}

export function resolveManifestDir(
  browser: "edge" | "chrome",
  runtimeEnv: RuntimeEnvironment = detectRuntimeEnvironment(),
): string {
  const home = os.homedir();
  if (!home) {
    errorOut("Home directory not found.");
  }
  if (runtimeEnv === "native-win32") {
    // Windows: registry-based is preferred, but file-based works with --user-data-dir.
    // For system-wide, we point to the per-user NativeMessagingHosts under LOCALAPPDATA.
    const base = process.env.LOCALAPPDATA || path.join(home, "AppData", "Local");
    if (browser === "edge") {
      return path.join(base, "Microsoft", "Edge", "User Data", "NativeMessagingHosts");
    }
    return path.join(base, "Google", "Chrome", "User Data", "NativeMessagingHosts");
  }
  if (runtimeEnv === "native-linux") {
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

export function resolveSetupWrapperPath(
  nodePath: string,
  hostPath: string,
  profileName: string,
  profileDataDir: string,
): { wrapperPath: string } {
  return { wrapperPath: writeWrapper(nodePath, hostPath, profileName, profileDataDir) };
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

export async function runSetup(options: Options, prettyOutput: boolean): Promise<void> {
  const browser = resolveBrowser(options.browser);
  if (!browser) {
    errorOut("Missing or invalid --browser (edge|chrome)");
  }
  const extensionsUrl = browser === "edge" ? "edge://extensions" : "chrome://extensions";

  const nodePath = resolveNodePath(options);
  const runtimeEnv = detectRuntimeEnvironment();

  // Sync extension + host to stable paths (before extensionId so interactive mode can show it)
  const config = resolveConfig();
  const hostPath = resolveHostPath(config.baseDataDir);
  let extensionSync;
  try {
    extensionSync = syncExtension(config.baseDataDir, { force: true });
  } catch {
    extensionSync = null;
  }
  const hasProvidedExtensionId = typeof options["extension-id"] === "string"
    ? options["extension-id"].trim().length > 0
    : (process.env.TABCTL_EXTENSION_ID || "").trim().length > 0;

  // Resolve extension ID: explicit flag, derived from install path, or interactive prompt
  let extensionId = resolveExtensionId(options, false);
  if (!extensionId) {
    // Auto-derive from the installed extension path (Chromium uses SHA256 of the path)
    // Prefer the just-synced path; fall back to resolving independently
    const installedDir = extensionSync?.extensionDir ?? resolveInstalledExtensionDir(config.baseDataDir);
    if (fs.existsSync(path.join(installedDir, "manifest.json"))) {
      const derivedFromPath = canonicalizeExtensionPath(installedDir);
      extensionId = deriveExtensionId(installedDir);
      process.stderr.write(`Extension ID derived from: ${derivedFromPath}\n`);
      process.stderr.write(`Derived extension ID: ${extensionId}\n`);
    }
  }
  if (!extensionId) {
    errorOut("Could not derive extension ID (extension not synced). Use --extension-id <id> or set TABCTL_EXTENSION_ID.");
  }
  if (process.platform === "win32" && hasProvidedExtensionId) {
    const installedDir = extensionSync?.extensionDir ?? resolveInstalledExtensionDir(config.baseDataDir);
    if (fs.existsSync(path.join(installedDir, "manifest.json"))) {
      const derivedFromPath = canonicalizeExtensionPath(installedDir);
      const derivedExtensionId = deriveExtensionId(installedDir);
      if (derivedExtensionId !== extensionId) {
        process.stderr.write([
          "[tabctl] Provided extension ID differs from installed extension path derivation.",
          `  Provided: ${extensionId}`,
          `  Derived : ${derivedExtensionId}`,
          `  Path    : ${derivedFromPath}`,
          `  If native messaging is forbidden/disconnected, use the ID shown in ${extensionsUrl} or rerun setup without --extension-id.`,
          "",
        ].join("\n"));
      }
    }
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
  const wrapperInfo = resolveSetupWrapperPath(
    nodePath,
    hostPath,
    profileName,
    profileDataDir,
  );
  const wrapperPath = wrapperInfo.wrapperPath;

  // Resolve manifest directory: custom user-data-dir or system-wide
  const rawUserDataDir = typeof options["user-data-dir"] === "string"
    ? options["user-data-dir"].trim()
    : "";
  const userDataDir = rawUserDataDir
    ? path.resolve(rawUserDataDir)
    : "";
  const manifestDir = userDataDir
    ? path.join(userDataDir, "NativeMessagingHosts")
    : resolveManifestDir(browser, runtimeEnv);
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
  const extensionDir = extensionSync?.extensionDir || null;
  const verification = await verifyWindowsSetupConnectivity(
    profileName,
    browser,
    extensionDir,
    extensionId,
    manifestPath,
  );

  const setupData = {
    profileName,
    browser,
    extensionId,
    manifestPath,
    hostPath,
    nodePath,
    wrapperPath,
    runtimeEnv,
    dataDir: profileDataDir,
    ...(userDataDir ? { userDataDir } : {}),
    isDefault: registry.default === profileName,
    extensionDir,
    extensionSynced: extensionSync?.synced || false,
    verification,
  };

  if (!verification.ok) {
    const browserName = browser === "edge" ? "Edge" : "Chrome";
    printJson({
      ok: false,
      action: "setup",
      error: {
        message: "Windows setup verification failed",
      },
      data: setupData,
    }, prettyOutput);
    process.stderr.write([
      "",
      `[tabctl] Windows setup verification failed for ${browserName} profile "${profileName}".`,
      verification.socketPath ? `Socket: ${verification.socketPath}` : null,
      verification.detail ? `Reason: ${verification.detail}` : null,
      verification.expectedExtensionId ? `Expected extension ID: ${verification.expectedExtensionId}` : null,
      verification.runtimeExtensionId ? `Runtime extension ID: ${verification.runtimeExtensionId}` : null,
      "Manual installation steps:",
      ...verification.manualSteps.map((step, index) => `  ${index + 1}. ${step}`),
      "",
    ].filter(Boolean).join("\n"));
    process.exit(1);
    return;
  }

  if (verification.reason === "extension-id-mismatch") {
    process.stderr.write([
      "",
      "[tabctl] Windows setup verification warning: runtime extension ID mismatch.",
      verification.detail ? `Reason: ${verification.detail}` : null,
      verification.expectedExtensionId ? `Expected extension ID: ${verification.expectedExtensionId}` : null,
      verification.runtimeExtensionId ? `Runtime extension ID: ${verification.runtimeExtensionId}` : null,
      "Setup completed, but this profile may be connected to a different loaded extension setup.",
      `Check ${extensionsUrl} and confirm the intended extension ID for this profile before continuing.`,
      "",
    ].filter(Boolean).join("\n"));
  }

  printJson({
    ok: true,
    action: "setup",
    data: setupData,
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
  const browserName = browser === "edge" ? "Edge" : "Chrome";
  const extensionPathDisplay = extensionDir;
  const loadSteps = extensionDir
    ? [
        `Load the extension: ${extensionsUrl} → Developer mode → Load unpacked`,
        `  Path: ${extensionPathDisplay}`,
        process.platform === "darwin" ? "  Tip: press Cmd+Shift+G in the file dialog to paste the path" : null,
      ].filter(Boolean).join("\n")
    : `Load the extension: ${extensionsUrl} → Developer mode → Load unpacked`;
  process.stderr.write([
    loadSteps,
    `Verify connection: tabctl --profile ${profileName} ping`,
    `If ping fails, ensure the ${browserName} extension is active.`,
    "",
  ].join("\n"));
}
