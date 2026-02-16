import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";

import { HOST_NAME } from "../constants";
import { errorOut } from "../output";

export type RuntimeEnvironment = "native-win32" | "native-linux" | "native-darwin" | "wsl";
export type WslLauncherSource = "windows-npm";

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

function resolveWindowsLauncherPathForWsl(): { path: string; source: WslLauncherSource } {
  return { path: resolveWindowsLauncherFromWindowsNpm(), source: "windows-npm" };
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
  wslWindowsPaths?: WslWindowsPaths,
): { wrapperPath: string; launcherSource: WslLauncherSource; distro: string } {
  const paths = wslWindowsPaths || resolveWslWindowsPaths();
  const distro = resolveWslDistroName();
  const launcherDir = path.join(paths.unixLocalAppData, "tabctl", "profiles", profileName);
  const launcher = resolveWindowsLauncherPathForWsl();
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
