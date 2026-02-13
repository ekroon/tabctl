/**
 * Setup command handler: browser profile configuration.
 * Extracted from meta.ts for modularity.
 */

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";

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

export type RuntimeEnvironment = "native-win32" | "native-linux" | "native-darwin" | "wsl";

export type WslWindowsPaths = {
  windowsLocalAppData: string;
  unixLocalAppData: string;
};

export function isWslEnvironment(): boolean {
  if (process.platform !== "linux") {
    return false;
  }
  if (process.env.WSL_DISTRO_NAME || process.env.WSL_INTEROP) {
    return true;
  }
  try {
    const osRelease = fs.readFileSync("/proc/sys/kernel/osrelease", "utf8").toLowerCase();
    if (osRelease.includes("microsoft")) {
      return true;
    }
  } catch {
    // ignore
  }
  try {
    const version = fs.readFileSync("/proc/version", "utf8").toLowerCase();
    return version.includes("microsoft") || version.includes("wsl");
  } catch {
    return false;
  }
}

export function detectRuntimeEnvironment(): RuntimeEnvironment {
  if (process.platform === "win32") return "native-win32";
  if (process.platform === "darwin") return "native-darwin";
  if (isWslEnvironment()) return "wsl";
  return "native-linux";
}

function normalizePackageVersion(spec: string): string {
  const match = spec.match(/\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?/);
  return match ? match[0] : "";
}

function resolvePackageRoot(startDir: string): string {
  let dir = startDir;
  for (;;) {
    const pkgPath = path.join(dir, "package.json");
    if (fs.existsSync(pkgPath)) {
      return dir;
    }
    const parent = path.dirname(dir);
    if (parent === dir) {
      errorOut("Could not resolve package root.");
    }
    dir = parent;
  }
}

function tryResolveWindowsLauncherFromLocal(packageRoot: string): string | null {
  try {
    const resolved = require.resolve("tabctl-win32-x64/tabctl-host.exe");
    if (fs.existsSync(resolved)) {
      return resolved;
    }
  } catch {
    // optional dependency may be absent on WSL/Linux
  }

  const localCandidates = [
    path.join(packageRoot, "node_modules", "tabctl-win32-x64", "tabctl-host.exe"),
    path.join(packageRoot, "packages", "win32-x64", "tabctl-host.exe"),
  ];
  for (const candidate of localCandidates) {
    if (fs.existsSync(candidate)) {
      return candidate;
    }
  }
  return null;
}

function resolveWindowsLauncherVersion(packageRoot: string): string {
  try {
    const pkg = JSON.parse(fs.readFileSync(path.join(packageRoot, "package.json"), "utf8")) as {
      version?: string;
      optionalDependencies?: Record<string, string>;
    };
    const fromPackage = typeof pkg.version === "string" ? normalizePackageVersion(pkg.version) : "";
    if (fromPackage) {
      return fromPackage;
    }
    const optionalSpec = pkg.optionalDependencies?.["tabctl-win32-x64"];
    const fromOptional = typeof optionalSpec === "string" ? normalizePackageVersion(optionalSpec) : "";
    if (fromOptional) {
      return fromOptional;
    }
  } catch {
    // handled below
  }
  errorOut("Could not resolve tabctl-win32-x64 version for WSL setup.");
}

function downloadWindowsLauncherViaNpm(packageRoot: string, cacheDir: string): string {
  const version = resolveWindowsLauncherVersion(packageRoot);
  fs.mkdirSync(cacheDir, { recursive: true });
  const cachedPath = path.join(cacheDir, `tabctl-host-${version}.exe`);
  if (fs.existsSync(cachedPath)) {
    return cachedPath;
  }

  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-win32-x64-"));
  try {
    const pack = spawnSync("npm", ["pack", `tabctl-win32-x64@${version}`, "--silent"], {
      cwd: tempDir,
      encoding: "utf8",
    });
    if (pack.status !== 0) {
      const detail = (pack.stderr || pack.stdout || "npm pack failed").trim();
      errorOut(`Failed to download tabctl-win32-x64@${version}. ${detail}`);
    }
    const tarball = (pack.stdout || "")
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean)
      .at(-1);
    if (!tarball) {
      errorOut(`Failed to download tabctl-win32-x64@${version}: npm pack did not return a tarball name.`);
    }
    const tarballPath = path.join(tempDir, tarball);
    const unpack = spawnSync("tar", ["-xzf", tarballPath], {
      cwd: tempDir,
      encoding: "utf8",
    });
    if (unpack.status !== 0) {
      const detail = (unpack.stderr || unpack.stdout || "tar extract failed").trim();
      errorOut(`Failed to extract ${tarball}. ${detail}`);
    }
    const exePath = path.join(tempDir, "package", "tabctl-host.exe");
    if (!fs.existsSync(exePath)) {
      errorOut(`Downloaded package ${tarball} does not contain tabctl-host.exe.`);
    }
    fs.copyFileSync(exePath, cachedPath);
    return cachedPath;
  } finally {
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
}

function resolveWindowsLauncherPathForWsl(packageRoot: string, cacheDir: string): { path: string; source: "local" | "npm" } {
  const local = tryResolveWindowsLauncherFromLocal(packageRoot);
  if (local) {
    return { path: local, source: "local" };
  }
  return { path: downloadWindowsLauncherViaNpm(packageRoot, cacheDir), source: "npm" };
}

function wslPathToWindows(unixPath: string): string {
  const result = spawnSync("wslpath", ["-w", unixPath], { encoding: "utf8" });
  if (result.status !== 0 || !result.stdout) {
    const detail = (result.stderr || result.stdout || "wslpath conversion failed").trim();
    errorOut(`Failed to convert WSL path to Windows path: ${unixPath}. ${detail}`);
  }
  return result.stdout.trim();
}

function windowsPathToWsl(windowsPath: string): string {
  const result = spawnSync("wslpath", ["-u", windowsPath], { encoding: "utf8" });
  if (result.status !== 0 || !result.stdout) {
    const detail = (result.stderr || result.stdout || "wslpath conversion failed").trim();
    errorOut(`Failed to convert Windows path to WSL path: ${windowsPath}. ${detail}`);
  }
  return result.stdout.trim();
}

function resolveWslWindowsPaths(): WslWindowsPaths {
  const localAppDataCmd = spawnSync("cmd.exe", ["/d", "/s", "/c", "echo", "%LOCALAPPDATA%"], { encoding: "utf8" });
  if (localAppDataCmd.status !== 0) {
    const detail = (localAppDataCmd.stderr || localAppDataCmd.stdout || "cmd.exe failed").trim();
    errorOut(`Failed to read Windows LOCALAPPDATA from WSL. ${detail}`);
  }
  const windowsLocalAppData = (localAppDataCmd.stdout || "")
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .find((line) => /^[A-Za-z]:\\/.test(line));
  if (!windowsLocalAppData) {
    errorOut("Could not resolve Windows LOCALAPPDATA from WSL.");
  }
  const unixLocalAppData = windowsPathToWsl(windowsLocalAppData);
  if (!fs.existsSync(unixLocalAppData)) {
    errorOut(`Resolved Windows LOCALAPPDATA is not accessible from WSL: ${unixLocalAppData}`);
  }
  return { windowsLocalAppData, unixLocalAppData };
}

function resolveWslDistroName(): string {
  const envValue = (process.env.WSL_DISTRO_NAME || "").trim();
  if (envValue) {
    return envValue;
  }
  const list = spawnSync("wsl.exe", ["-l", "-q"], { encoding: "utf8" });
  if (list.status === 0) {
    const distro = (list.stdout || "")
      .split(/\r?\n/)
      .map((line) => line.trim())
      .find(Boolean);
    if (distro) {
      return distro;
    }
  }
  errorOut("Could not resolve WSL distro name. Set WSL_DISTRO_NAME and retry setup.");
}

function registerWindowsNativeHost(browser: "edge" | "chrome", windowsManifestPath: string): void {
  const regKey = browser === "edge"
    ? `HKCU\\Software\\Microsoft\\Edge\\NativeMessagingHosts\\${HOST_NAME}`
    : `HKCU\\Software\\Google\\Chrome\\NativeMessagingHosts\\${HOST_NAME}`;
  const result = spawnSync("cmd.exe", [
    "/d", "/s", "/c",
    "reg", "add", regKey, "/ve", "/t", "REG_SZ", "/d", windowsManifestPath, "/f",
  ], { encoding: "utf8" });
  if (result.status !== 0) {
    const detail = (result.stderr || result.stdout || "reg add failed").trim();
    errorOut(`Failed to register Windows native host for ${browser}. ${detail}`);
  }
}

function createWslWindowsLauncher(
  nodePath: string,
  hostPath: string,
  profileName: string,
  packageRoot: string,
  wslWindowsPaths: WslWindowsPaths,
): { wrapperPath: string; launcherSource: "local" | "npm"; distro: string } {
  const distro = resolveWslDistroName();
  const launcherDir = path.join(wslWindowsPaths.unixLocalAppData, "tabctl", "profiles", profileName);
  const cacheDir = path.join(wslWindowsPaths.unixLocalAppData, "tabctl", "cache");
  const launcher = resolveWindowsLauncherPathForWsl(packageRoot, cacheDir);
  fs.mkdirSync(launcherDir, { recursive: true });

  const launcherExePath = path.join(launcherDir, "tabctl-host.exe");
  fs.copyFileSync(launcher.path, launcherExePath);

  const cfgPath = path.join(launcherDir, "host-launcher.cfg");
  const cfgLines = [
    nodePath,
    hostPath,
    "LAUNCH_MODE=wsl",
    `WSL_DISTRO=${distro}`,
    `XDG_CONFIG_HOME=${process.env.XDG_CONFIG_HOME || path.join(os.homedir(), ".config")}`,
    `XDG_STATE_HOME=${process.env.XDG_STATE_HOME || path.join(os.homedir(), ".local", "state")}`,
    `TABCTL_PROFILE=${profileName}`,
    "",
  ];
  fs.writeFileSync(cfgPath, cfgLines.join("\r\n"), "utf8");

  return {
    wrapperPath: wslPathToWindows(launcherExePath),
    launcherSource: launcher.source,
    distro,
  };
}

export function resolveManifestDir(
  browser: "edge" | "chrome",
  runtimeEnv: RuntimeEnvironment = detectRuntimeEnvironment(),
  wslWindowsPaths?: WslWindowsPaths,
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
  if (runtimeEnv === "wsl") {
    const paths = wslWindowsPaths || resolveWslWindowsPaths();
    if (browser === "edge") {
      return path.join(paths.unixLocalAppData, "Microsoft", "Edge", "User Data", "NativeMessagingHosts");
    }
    return path.join(paths.unixLocalAppData, "Google", "Chrome", "User Data", "NativeMessagingHosts");
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
  runtimeEnv: RuntimeEnvironment,
  wslWindowsPaths?: WslWindowsPaths,
): { wrapperPath: string; unixWrapperPath?: string; launcherSource?: "local" | "npm"; distro?: string } {
  const unixWrapperPath = writeWrapper(nodePath, hostPath, profileName, profileDataDir);
  if (runtimeEnv !== "wsl") {
    return { wrapperPath: unixWrapperPath };
  }
  const packageRoot = resolvePackageRoot(__dirname);
  const paths = wslWindowsPaths || resolveWslWindowsPaths();
  const launcher = createWslWindowsLauncher(nodePath, hostPath, profileName, packageRoot, paths);
  return {
    wrapperPath: launcher.wrapperPath,
    unixWrapperPath,
    launcherSource: launcher.launcherSource,
    distro: launcher.distro,
  };
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
  const runtimeEnv = detectRuntimeEnvironment();
  const wslWindowsPaths = runtimeEnv === "wsl" ? resolveWslWindowsPaths() : undefined;

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
  const wrapperInfo = resolveSetupWrapperPath(
    nodePath,
    hostPath,
    profileName,
    profileDataDir,
    runtimeEnv,
    wslWindowsPaths,
  );
  const wrapperPath = wrapperInfo.wrapperPath;

  // Resolve manifest directory: custom user-data-dir or system-wide
  const rawUserDataDir = typeof options["user-data-dir"] === "string"
    ? options["user-data-dir"].trim()
    : "";
  const userDataDir = rawUserDataDir
    ? (runtimeEnv === "wsl" && /^[A-Za-z]:\\/.test(rawUserDataDir)
      ? windowsPathToWsl(rawUserDataDir)
      : path.resolve(rawUserDataDir))
    : "";
  const manifestDir = userDataDir
    ? path.join(userDataDir, "NativeMessagingHosts")
    : resolveManifestDir(browser, runtimeEnv, wslWindowsPaths);
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

  let windowsManifestPath: string | null = null;
  if (runtimeEnv === "wsl") {
    windowsManifestPath = wslPathToWindows(manifestPath);
    registerWindowsNativeHost(browser, windowsManifestPath);
  }

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
      runtimeEnv,
      ...(wrapperInfo.unixWrapperPath ? { unixWrapperPath: wrapperInfo.unixWrapperPath } : {}),
      ...(wrapperInfo.launcherSource ? { launcherSource: wrapperInfo.launcherSource } : {}),
      ...(wrapperInfo.distro ? { wslDistro: wrapperInfo.distro } : {}),
      ...(windowsManifestPath ? { windowsManifestPath } : {}),
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
  const extensionsUrl = browser === "edge" ? "edge://extensions" : "chrome://extensions";
  const browserName = browser === "edge" ? "Edge" : "Chrome";
  const extensionDir = extensionSync?.extensionDir || null;
  const extensionPathDisplay = extensionDir && runtimeEnv === "wsl"
    ? wslPathToWindows(extensionDir)
    : extensionDir;
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
