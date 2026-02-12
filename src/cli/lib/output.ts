import { VERSION, BASE_VERSION, DEV_BUILD } from "../../shared/version";
import { getActiveProfile } from "../../shared/profiles";
import { syncExtension, syncHost, resolveInstalledHostPath, compareBaseVersions } from "../../shared/extension-sync";
import { resolveConfig } from "../../shared/config";
import { checkWrapper, resolveWrapperPath } from "../../shared/wrapper-health";
import { sendFireAndForget } from "./client";
import { createRequestId } from "./client";
import { writeWrapper } from "./commands/setup";

export function printJson(payload: Record<string, unknown>, pretty = true): void {
  try {
    const active = getActiveProfile();
    if (active) {
      payload.profile = active.name;
      payload.browser = active.profile.browser;
    }
  } catch {
    // Don't let profile errors break output
  }
  const output = pretty ? JSON.stringify(payload, null, 2) : JSON.stringify(payload);
  process.stdout.write(`${output}\n`);
}

export function errorOut(message: string): never {
  // On ENOENT (socket missing), try syncing host + extension before showing error
  if (message.includes("ENOENT")) {
    try {
      const config = resolveConfig();
      const hostResult = syncHost(config.baseDataDir);
      const extResult = syncExtension(config.baseDataDir);
      if (hostResult.synced || extResult.synced) {
        process.stderr.write(`[tabctl] synced host and extension to ${config.baseDataDir}\n`);
      }
    } catch {
      // Sync is best-effort
    }
  }

  const hints: Record<string, string> = {
    "Unknown option: --format": "Use --json for JSON output. --format is only for report.",
    "ENOENT": "Native host not running. Ensure the browser extension is loaded and active. If you recently upgraded, run: tabctl setup",
  };
  const hint = Object.entries(hints).find(([key]) => message.includes(key))?.[1];
  if (hint) {
    printJson({ ok: false, error: { message, hint } });
  } else {
    printJson({ ok: false, error: { message } });
  }
  process.exit(1);
  throw new Error(message);
}

export function setupStdoutErrorHandling(): void {
  process.stdout.on("error", (error: NodeJS.ErrnoException) => {
    if (error.code === "EPIPE") {
      process.exit(0);
    }
    throw error;
  });
}

export function emitVersionWarnings(response: Record<string, unknown>, fallbackAction: string): void {
  const hostVersion = typeof response.version === "string" ? response.version : null;
  const data = response.data as Record<string, unknown> | undefined;
  const hostBaseVersion = data && typeof data.hostBaseVersion === "string" ? data.hostBaseVersion : null;
  const isDevCli = DEV_BUILD;

  // CLI ↔ host BASE_VERSION mismatch: auto-upgrade (sync files + trigger reload).
  // Dev builds never sync — they use whatever host is already installed.
  const effectiveHostBase = hostBaseVersion ?? hostVersion;
  if (effectiveHostBase && effectiveHostBase !== BASE_VERSION && !isDevCli) {
    // Downgrade: host is newer than CLI — warn but don't sync/reload
    if (compareBaseVersions(BASE_VERSION, effectiveHostBase) < 0) {
      process.stderr.write(`[tabctl] cli (${BASE_VERSION}) is older than host (${effectiveHostBase}). Consider upgrading: npm install -g tabctl\n`);
    } else {
      try {
        const config = resolveConfig();
        const hostResult = syncHost(config.baseDataDir);
        const extResult = syncExtension(config.baseDataDir);
        const anySynced = hostResult.synced || extResult.synced;
        if (anySynced) {
          process.stderr.write(`[tabctl] upgraded: ${effectiveHostBase} → ${BASE_VERSION}. Reloading extension...\n`);
        } else {
          process.stderr.write(`[tabctl] host is stale (${effectiveHostBase}), reloading extension...\n`);
        }
        sendFireAndForget({ id: createRequestId(), action: "reload", params: {} });
        try {
          repairActiveWrapper(config.baseDataDir);
        } catch {
          // Wrapper repair is best-effort
        }
      } catch {
        process.stderr.write(`[tabctl] version mismatch: cli ${BASE_VERSION}, host ${effectiveHostBase}. Run: tabctl setup\n`);
      }
    }
  }

  const extensionVersion = data && typeof data.extensionVersion === "string" ? (data.extensionVersion as string) : null;
  const extensionComponent = data && typeof data.extensionComponent === "string" ? (data.extensionComponent as string) : null;
  if (extensionVersion && hostVersion && extensionVersion !== hostVersion) {
    process.stderr.write(`[tabctl] version mismatch: host ${hostVersion}, extension ${extensionVersion}. Reload the extension in your browser\n`);
  }
  if (extensionComponent && extensionComponent !== "extension") {
    process.stderr.write(`[tabctl] unexpected extension component: ${extensionComponent}\n`);
  }

  const action = (response.action as string | undefined) || fallbackAction;
  const extensionExpected = !["history", "version"].includes(action);
  if (extensionExpected && !extensionVersion) {
    process.stderr.write("[tabctl] extension version unavailable. Reload the extension in your browser\n");
  }
}

/**
 * Repair the active profile's wrapper if the Node path is broken.
 * Conservative: only fixes paths that are confirmed missing.
 */
function repairActiveWrapper(baseDataDir: string): void {
  const active = getActiveProfile();
  if (!active) return;

  const wrapperPath = resolveWrapperPath(active.profile.dataDir);
  const check = checkWrapper(wrapperPath);
  if (check.ok || !check.info) return;

  const fs = require("node:fs") as typeof import("node:fs");
  const path = require("node:path") as typeof import("node:path");
  const needsNodeFix = !fs.existsSync(check.info.nodePath);
  const needsHostFix = !fs.existsSync(check.info.hostPath);

  if (!needsNodeFix && !needsHostFix) return;

  const newNodePath = needsNodeFix ? process.execPath : check.info.nodePath;
  const newHostPath = needsHostFix
    ? resolveInstalledHostPath(baseDataDir)
    : check.info.hostPath;

  writeWrapper(newNodePath, newHostPath, check.info.profileName, path.dirname(wrapperPath));

  if (needsNodeFix) {
    process.stderr.write(`[tabctl] fixed wrapper Node path: ${check.info.nodePath} → ${newNodePath}\n`);
  }
  if (needsHostFix) {
    process.stderr.write(`[tabctl] fixed wrapper host path: ${check.info.hostPath} → ${newHostPath}\n`);
  }
}
