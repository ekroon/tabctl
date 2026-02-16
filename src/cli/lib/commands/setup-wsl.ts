import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";

import { HOST_NAME } from "../constants";
import { errorOut } from "../output";

export type RuntimeEnvironment = "native-win32" | "native-linux" | "native-darwin" | "wsl";
export type WslLauncherSource = "local" | "npm" | "windows-npm";
type WslSetupMode = "legacy" | "windows-npm";

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

function resolveWslSetupMode(): WslSetupMode {
  const mode = (process.env.TABCTL_WSL_SETUP_MODE || "").trim().toLowerCase();
  if (!mode || mode === "legacy") {
    return "legacy";
  }
  if (mode === "windows-npm") {
    return "windows-npm";
  }
  errorOut(`Unsupported TABCTL_WSL_SETUP_MODE value: ${mode}. Supported values: legacy, windows-npm.`);
}

function parseWindowsPaths(output: string): string[] {
  return output
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => /^[A-Za-z]:\\/.test(line));
}

function resolveWindowsLauncherFromWindowsNpm(): string {
  const whereHost = spawnSync("cmd.exe", ["/d", "/s", "/c", "where", "tabctl-host.exe"], { encoding: "utf8" });
  const npmRoot = spawnSync("cmd.exe", ["/d", "/s", "/c", "npm", "root", "-g"], { encoding: "utf8" });

  const candidates: string[] = [];
  candidates.push(...parseWindowsPaths(whereHost.stdout || ""));
  for (const root of parseWindowsPaths(npmRoot.stdout || "")) {
    candidates.push(path.win32.join(root, "tabctl-win32-x64", "tabctl-host.exe"));
    candidates.push(path.win32.join(root, "tabctl", "node_modules", "tabctl-win32-x64", "tabctl-host.exe"));
  }

  const seen = new Set<string>();
  for (const windowsCandidate of candidates) {
    if (seen.has(windowsCandidate)) {
      continue;
    }
    seen.add(windowsCandidate);
    const unixCandidate = windowsPathToWsl(windowsCandidate);
    if (fs.existsSync(unixCandidate)) {
      return unixCandidate;
    }
  }

  const whereDetail = (whereHost.stderr || whereHost.stdout || "where tabctl-host.exe failed").trim();
  const npmRootDetail = (npmRoot.stderr || npmRoot.stdout || "npm root -g failed").trim();
  errorOut(
    `Failed to resolve tabctl-host.exe from Windows npm installation in WSL mode. ` +
    `where status=${whereHost.status}, npm root status=${npmRoot.status}. ` +
    `where output: ${whereDetail}. npm root output: ${npmRootDetail}`,
  );
}

function resolveWindowsLauncherPathForWsl(packageRoot: string, cacheDir: string): { path: string; source: WslLauncherSource } {
  if (resolveWslSetupMode() === "windows-npm") {
    return { path: resolveWindowsLauncherFromWindowsNpm(), source: "windows-npm" };
  }
  const local = tryResolveWindowsLauncherFromLocal(packageRoot);
  if (local) {
    return { path: local, source: "local" };
  }
  return { path: downloadWindowsLauncherViaNpm(packageRoot, cacheDir), source: "npm" };
}

export function wslPathToWindows(unixPath: string): string {
  const result = spawnSync("wslpath", ["-w", unixPath], { encoding: "utf8" });
  if (result.status !== 0 || !result.stdout) {
    const detail = (result.stderr || result.stdout || "wslpath conversion failed").trim();
    errorOut(`Failed to convert WSL path to Windows path: ${unixPath}. ${detail}`);
  }
  return result.stdout.trim();
}

export function windowsPathToWsl(windowsPath: string): string {
  const result = spawnSync("wslpath", ["-u", windowsPath], { encoding: "utf8" });
  if (result.status !== 0 || !result.stdout) {
    const detail = (result.stderr || result.stdout || "wslpath conversion failed").trim();
    errorOut(`Failed to convert Windows path to WSL path: ${windowsPath}. ${detail}`);
  }
  return result.stdout.trim();
}

export function resolveWslWindowsPaths(): WslWindowsPaths {
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

export function resolveWslManifestDir(
  browser: "edge" | "chrome",
  wslWindowsPaths?: WslWindowsPaths,
): string {
  const paths = wslWindowsPaths || resolveWslWindowsPaths();
  if (browser === "edge") {
    return path.join(paths.unixLocalAppData, "Microsoft", "Edge", "User Data", "NativeMessagingHosts");
  }
  return path.join(paths.unixLocalAppData, "Google", "Chrome", "User Data", "NativeMessagingHosts");
}

export function registerWslWindowsNativeHost(browser: "edge" | "chrome", windowsManifestPath: string): void {
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

export function createWslWindowsLauncher(
  nodePath: string,
  hostPath: string,
  profileName: string,
  startDir: string,
  wslWindowsPaths?: WslWindowsPaths,
): { wrapperPath: string; launcherSource: WslLauncherSource; distro: string } {
  const paths = wslWindowsPaths || resolveWslWindowsPaths();
  const packageRoot = resolvePackageRoot(startDir);
  const distro = resolveWslDistroName();
  const launcherDir = path.join(paths.unixLocalAppData, "tabctl", "profiles", profileName);
  const cacheDir = path.join(paths.unixLocalAppData, "tabctl", "cache");
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
