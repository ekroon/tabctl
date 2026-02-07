import path from "path";
import fs from "fs";
import { resolveConfig } from "./config";

export const EXTENSION_DIR_NAME = "extension";

export function resolveBundledExtensionDir(): string {
  const dir = path.resolve(__dirname, "../extension");
  const manifest = path.join(dir, "manifest.json");
  if (!fs.existsSync(dir) || !fs.existsSync(manifest)) {
    throw new Error(`Bundled extension not found at ${dir}`);
  }
  return dir;
}

export function resolveInstalledExtensionDir(dataDir?: string): string {
  const dir = dataDir ?? resolveConfig().dataDir;
  return path.join(dir, EXTENSION_DIR_NAME);
}

export function readExtensionVersion(extensionDir: string): string | null {
  try {
    const raw = fs.readFileSync(path.join(extensionDir, "manifest.json"), "utf-8");
    const manifest = JSON.parse(raw);
    return typeof manifest.version === "string" ? manifest.version : null;
  } catch {
    return null;
  }
}

export function syncExtension(dataDir?: string): {
  synced: boolean;
  bundledVersion: string | null;
  installedVersion: string | null;
  extensionDir: string;
} {
  const bundledDir = resolveBundledExtensionDir();
  const installedDir = resolveInstalledExtensionDir(dataDir);
  const bundledVersion = readExtensionVersion(bundledDir);
  const installedVersion = readExtensionVersion(installedDir);

  const needsCopy = !fs.existsSync(installedDir) || bundledVersion !== installedVersion;

  if (needsCopy) {
    fs.mkdirSync(installedDir, { recursive: true });
    fs.cpSync(bundledDir, installedDir, { recursive: true });
  }

  return {
    synced: needsCopy,
    bundledVersion,
    installedVersion,
    extensionDir: installedDir,
  };
}

export function checkExtensionSync(dataDir?: string): {
  needsSync: boolean;
  needsReload: boolean;
  bundledVersion: string | null;
  installedVersion: string | null;
  extensionDir: string;
} {
  const bundledDir = resolveBundledExtensionDir();
  const installedDir = resolveInstalledExtensionDir(dataDir);
  const bundledVersion = readExtensionVersion(bundledDir);
  const installedVersion = readExtensionVersion(installedDir);

  const exists = fs.existsSync(installedDir) && installedVersion !== null;
  const needsSync = !exists || bundledVersion !== installedVersion;
  const needsReload = exists && bundledVersion !== installedVersion;

  return {
    needsSync,
    needsReload,
    bundledVersion,
    installedVersion,
    extensionDir: installedDir,
  };
}
