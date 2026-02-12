import path from "node:path";
import fs from "node:fs";
import crypto from "node:crypto";
import { resolveConfig } from "./config";
import { GIT_SHA } from "./version";

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

/**
 * Compare two semver versions by their base (major.minor.patch) components.
 * Strips any prerelease/build metadata before comparing.
 * Returns -1 if a < b, 0 if equal, 1 if a > b.
 */
export function compareBaseVersions(a: string, b: string): -1 | 0 | 1 {
  const strip = (v: string) => v.replace(/[-+].*$/, "");
  const pa = strip(a).split(".").map(Number);
  const pb = strip(b).split(".").map(Number);
  for (let i = 0; i < 3; i++) {
    const va = pa[i] ?? 0;
    const vb = pb[i] ?? 0;
    if (va < vb) return -1;
    if (va > vb) return 1;
  }
  return 0;
}

/** Returns true when the current CLI is a dev build (has a git SHA). */
export function isDevBuild(): boolean {
  return GIT_SHA !== null;
}

export interface SyncOptions {
  /** Skip dev-build and downgrade guards. Used by setup and tests. */
  force?: boolean;
}

export function syncExtension(dataDir?: string, options?: SyncOptions): {
  synced: boolean;
  bundledVersion: string | null;
  installedVersion: string | null;
  extensionDir: string;
} {
  const bundledDir = resolveBundledExtensionDir();
  const installedDir = resolveInstalledExtensionDir(dataDir);
  const bundledVersion = readExtensionVersion(bundledDir);
  const installedVersion = readExtensionVersion(installedDir);

  if (!options?.force) {
    // Dev builds never overwrite installed files
    if (isDevBuild()) {
      return { synced: false, bundledVersion, installedVersion, extensionDir: installedDir };
    }
    // Downgrade protection: don't replace a newer installed version
    if (bundledVersion && installedVersion && compareBaseVersions(bundledVersion, installedVersion) < 0) {
      return { synced: false, bundledVersion, installedVersion, extensionDir: installedDir };
    }
  }

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

export function syncHost(dataDir?: string, options?: SyncOptions): {
  synced: boolean;
  bundledVersion: string | null;
  installedVersion: string | null;
  hostPath: string;
} {
  const bundledPath = resolveBundledHostPath();
  const installedPath = resolveInstalledHostPath(dataDir);
  const bundledVersion = readHostVersion(bundledPath);
  const installedVersion = readHostVersion(installedPath);

  if (!options?.force) {
    // Dev builds never overwrite installed files
    if (isDevBuild()) {
      return { synced: false, bundledVersion, installedVersion, hostPath: installedPath };
    }
    // Downgrade protection: don't replace a newer installed version
    if (bundledVersion && installedVersion && compareBaseVersions(bundledVersion, installedVersion) < 0) {
      return { synced: false, bundledVersion, installedVersion, hostPath: installedPath };
    }
  }

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
