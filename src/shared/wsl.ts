/**
 * WSL (Windows Subsystem for Linux) detection and utility functions.
 * Based on Raymond Kroon's implementation.
 */

import fs from "fs";
import path from "path";
import os from "os";
import { execSync } from "child_process";

/**
 * Detect if running inside WSL.
 * Checks for /proc/version containing "microsoft" or "WSL".
 */
export function isWSL(): boolean {
  if (process.platform !== "linux") {
    return false;
  }
  
  try {
    const version = fs.readFileSync("/proc/version", "utf-8").toLowerCase();
    return version.includes("microsoft") || version.includes("wsl");
  } catch {
    return false;
  }
}

/**
 * Get the Windows username from within WSL.
 * Uses wslvar or cmd.exe to retrieve USERNAME environment variable.
 */
export function getWindowsUsername(): string {
  try {
    // Try wslvar first (WSL 2 has this utility)
    try {
      const username = execSync("wslvar USERNAME", { encoding: "utf-8" }).trim();
      if (username) {
        return username;
      }
    } catch {
      // wslvar not available, fall through
    }

    // Fallback: use cmd.exe
    const username = execSync("cmd.exe /c echo %USERNAME%", { encoding: "utf-8" }).trim();
    return username;
  } catch (err) {
    const detail = err instanceof Error ? err.message : String(err);
    throw new Error(`Failed to get Windows username from WSL: ${detail}`);
  }
}

/**
 * Convert a WSL Unix path to a Windows path.
 * Examples:
 *   /home/user/file -> \\wsl.localhost\<distro>\home\user\file
 *   /mnt/c/Users/... -> C:\Users\...
 */
export function convertToWindowsPath(wslPath: string): string {
  try {
    // Use wslpath utility (available in WSL)
    const windowsPath = execSync(`wslpath -w "${wslPath}"`, { encoding: "utf-8" }).trim();
    return windowsPath;
  } catch (err) {
    const detail = err instanceof Error ? err.message : String(err);
    throw new Error(`Failed to convert WSL path to Windows path: ${detail}`);
  }
}

/**
 * Get the Windows LocalAppData directory path from within WSL.
 * Returns the Unix-mounted path (e.g., /mnt/c/Users/<username>/AppData/Local).
 */
export function getWindowsLocalAppData(): string {
  const username = getWindowsUsername();
  // Standard Windows LocalAppData path mounted via /mnt/c
  return `/mnt/c/Users/${username}/AppData/Local`;
}

/**
 * Get the current WSL distribution name.
 * Uses the WSL_DISTRO_NAME environment variable.
 */
export function getWSLDistroName(): string {
  const distro = process.env.WSL_DISTRO_NAME;
  if (!distro) {
    throw new Error("WSL_DISTRO_NAME environment variable not set. Are you running in WSL?");
  }
  return distro;
}
