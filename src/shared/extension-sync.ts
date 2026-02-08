import path from "path";
import fs from "fs";
import crypto from "crypto";
import { resolveConfig } from "./config";

export const EXTENSION_DIR_NAME = "extension";
export const HOST_BUNDLE_NAME = "host.bundle.js";

/**
 * Derive the Chrome/Edge extension ID for an unpacked extension path.
 * Chromium computes: SHA256(absolute_path) → first 32 hex chars → map 0-f to a-p.
 */
export function deriveExtensionId(extensionDir: string): string {
  const hash = crypto.createHash("sha256").update(extensionDir).digest("hex").slice(0, 32);
  return hash.split("").map(c => String.fromCharCode("a".charCodeAt(0) + parseInt(c, 16))).join("");
}

export function resolveBundledExtensionDir(): string {
  const dir = path.resolve(__dirname, "../extension");
  const manifest = path.join(dir, "manifest.json");
  if (!fs.existsSync(dir) || !fs.existsSync(manifest)) {
    throw new Error(`Bundled extension not found at ${dir}`);
  }
  return dir;
}

export function resolveBundledHostPath(): string {
  const p = path.resolve(__dirname, "../host", HOST_BUNDLE_NAME);
  if (!fs.existsSync(p)) {
    throw new Error(`Bundled host not found at ${p}`);
  }
  return p;
}

export function resolveInstalledExtensionDir(dataDir?: string): string {
  const dir = dataDir ?? resolveConfig().baseDataDir;
  return path.join(dir, EXTENSION_DIR_NAME);
}

export function resolveInstalledHostPath(dataDir?: string): string {
  const dir = dataDir ?? resolveConfig().baseDataDir;
  return path.join(dir, HOST_BUNDLE_NAME);
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

/** Read the BASE_VERSION constant from a bundled host.bundle.js file. */
export function readHostVersion(hostPath: string): string | null {
  try {
    const content = fs.readFileSync(hostPath, "utf-8");
    const match = content.match(/\bBASE_VERSION\s*=\s*"([^"]+)"/);
    return match ? match[1] : null;
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

export function syncHost(dataDir?: string): {
  synced: boolean;
  bundledVersion: string | null;
  installedVersion: string | null;
  hostPath: string;
} {
  const bundledPath = resolveBundledHostPath();
  const installedPath = resolveInstalledHostPath(dataDir);
  const bundledVersion = readHostVersion(bundledPath);
  const installedVersion = readHostVersion(installedPath);

  const needsCopy = !fs.existsSync(installedPath) || bundledVersion !== installedVersion;

  if (needsCopy) {
    fs.mkdirSync(path.dirname(installedPath), { recursive: true });
    fs.copyFileSync(bundledPath, installedPath);
  }

  return {
    synced: needsCopy,
    bundledVersion,
    installedVersion,
    hostPath: installedPath,
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
