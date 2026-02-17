/**
 * Doctor command handler: diagnose and repair profile health.
 *
 * Checks each profile's wrapper for valid Node/host paths, verifies
 * extension sync status, and optionally auto-repairs broken wrappers.
 */

import fs from "node:fs";
import path from "node:path";

import { resolveConfig } from "../../../shared/config";
import { loadProfiles, type ProfileEntry } from "../../../shared/profiles";
import { checkWrapper, resolveWrapperPath } from "../../../shared/wrapper-health";
import { checkExtensionSync, resolveInstalledHostPath } from "../../../shared/extension-sync";
import { writeWrapper } from "./setup";
import { printJson, errorOut } from "../output";
import type { Options } from "../types";

type ProfileCheck = {
  ok: boolean;
  browser: string;
  dataDir: string;
  wrapperPath: string;
  issues: string[];
  fixed: boolean;
};

type DoctorResult = {
  ok: boolean;
  profiles: Record<string, ProfileCheck>;
  extension: {
    ok: boolean;
    synced: boolean;
    bundledVersion: string | null;
    installedVersion: string | null;
  };
  summary: {
    total: number;
    healthy: number;
    broken: number;
    fixed: number;
  };
};

function checkProfile(
  name: string,
  entry: ProfileEntry,
  fix: boolean,
): ProfileCheck {
  const wrapperPath = resolveWrapperPath(entry.dataDir);
  const check = checkWrapper(wrapperPath);
  const issues = [...check.issues];
  let fixed = false;

  if (fix && !check.ok && check.info) {
    const needsNodeFix = !fs.existsSync(check.info.nodePath);
    const needsHostFix = !fs.existsSync(check.info.hostPath);
    const hostImplementation = entry.hostImplementation === "rust" || check.info.hostImplementation === "rust"
      ? "rust"
      : "node";

    if (needsNodeFix || needsHostFix) {
      if (hostImplementation === "rust" && !fs.existsSync(entry.hostPath)) {
        issues.push(
          `Rust host path not found: ${entry.hostPath} — run: tabctl setup --browser ${entry.browser} --host-impl rust --rust-host-bin <absolute-path>`,
        );
      }

      const newNodePath = needsNodeFix
        ? (hostImplementation === "rust" ? entry.hostPath : process.execPath)
        : check.info.nodePath;
      let newHostPath = check.info.hostPath;
      if (needsHostFix) {
        if (hostImplementation === "rust") {
          newHostPath = entry.hostPath;
        } else {
          // Use the stable bundled host path
          try {
            const config = resolveConfig();
            newHostPath = resolveInstalledHostPath(config.baseDataDir);
            if (!fs.existsSync(newHostPath)) {
              issues.push(`Bundled host not found at ${newHostPath} — run: tabctl setup --browser ${entry.browser}`);
            }
          } catch {
            issues.push("Could not resolve bundled host path");
          }
        }
      }

      const canWrite = hostImplementation !== "rust" || fs.existsSync(entry.hostPath);
      if (canWrite) {
        try {
          writeWrapper(
            newNodePath,
            newHostPath,
            check.info.profileName,
            path.dirname(wrapperPath),
            hostImplementation,
          );
          fixed = true;

          // Update issue messages to show they were fixed
          const fixedIssues: string[] = [];
          if (needsNodeFix) {
            fixedIssues.push(`Fixed Node path: ${check.info.nodePath} → ${newNodePath}`);
          }
          if (needsHostFix) {
            fixedIssues.push(`Fixed host path: ${check.info.hostPath} → ${newHostPath}`);
          }
          // Replace original issues with fixed messages
          issues.length = 0;
          issues.push(...fixedIssues);
        } catch (err) {
          issues.push(`Failed to fix wrapper: ${err instanceof Error ? err.message : String(err)}`);
        }
      }
    }
  }

  return {
    ok: check.ok || fixed,
    browser: entry.browser,
    dataDir: entry.dataDir,
    wrapperPath,
    issues,
    fixed,
  };
}

export function runDoctor(options: Options, prettyOutput: boolean): void {
  const fix = options.fix === true;
  const config = resolveConfig();
  const registry = loadProfiles(config.configDir);

  const profileNames = Object.keys(registry.profiles);
  if (profileNames.length === 0) {
    errorOut("No profiles configured. Run: tabctl setup --browser <edge|chrome>");
  }

  // Check each profile
  const profiles: Record<string, ProfileCheck> = {};
  for (const name of profileNames) {
    profiles[name] = checkProfile(name, registry.profiles[name], fix);
  }

  // Check extension sync status
  let extensionCheck: DoctorResult["extension"];
  try {
    const sync = checkExtensionSync(config.baseDataDir);
    extensionCheck = {
      ok: !sync.needsSync,
      synced: !sync.needsSync,
      bundledVersion: sync.bundledVersion,
      installedVersion: sync.installedVersion,
    };
  } catch {
    extensionCheck = {
      ok: false,
      synced: false,
      bundledVersion: null,
      installedVersion: null,
    };
  }

  // Summary
  const total = profileNames.length;
  const healthy = Object.values(profiles).filter(p => p.ok).length;
  const broken = total - healthy;
  const fixed = Object.values(profiles).filter(p => p.fixed).length;

  const allOk = broken === 0 && extensionCheck.ok;

  printJson({
    ok: allOk,
    action: "doctor",
    data: {
      profiles,
      extension: extensionCheck,
      summary: { total, healthy, broken, fixed },
    },
  }, prettyOutput);

  // Helpful stderr hints
  if (!allOk && !fix) {
    const brokenNames = Object.entries(profiles)
      .filter(([, p]) => !p.ok)
      .map(([n]) => n);
    if (brokenNames.length > 0) {
      process.stderr.write(`\nBroken profiles: ${brokenNames.join(", ")}\n`);
      process.stderr.write("Run: tabctl doctor --fix\n\n");
    }
  }
  if (fixed > 0) {
    process.stderr.write(`\nFixed ${fixed} profile(s). Verify: tabctl ping\n\n`);
  }
}
