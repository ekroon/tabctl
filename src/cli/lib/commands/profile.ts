import { resolveConfig } from "../constants";
import { printJson, errorOut } from "../output";
import { loadProfiles, saveProfiles, listProfiles, getActiveProfile, removeProfile } from "../../../shared/profiles";
import type { Options } from "../types";

export function runProfileList(options: Options, prettyOutput: boolean): void {
  const profiles = listProfiles();
  printJson({
    ok: true,
    action: "profile-list",
    data: {
      profiles: profiles.map(({ name, profile, isDefault }) => ({
        name,
        browser: profile.browser,
        hostImplementation: profile.hostImplementation || "node",
        dataDir: profile.dataDir,
        isDefault,
      })),
    },
  }, prettyOutput);
}

export function runProfileShow(options: Options, prettyOutput: boolean): void {
  const config = resolveConfig();
  const active = getActiveProfile();
  if (!active) {
    printJson({
      ok: true,
      action: "profile-show",
      data: {
        mode: "legacy",
        message: "No profiles configured. Run tabctl setup to create one.",
        dataDir: config.dataDir,
        socketPath: config.socketPath,
        policyPath: config.policyPath,
      },
    }, prettyOutput);
    return;
  }
  const registry = loadProfiles();
  printJson({
    ok: true,
    action: "profile-show",
    data: {
      name: active.name,
      browser: active.profile.browser,
      hostImplementation: active.profile.hostImplementation || "node",
      extensionId: active.profile.extensionId,
      dataDir: active.profile.dataDir,
      ...(active.profile.userDataDir ? { userDataDir: active.profile.userDataDir } : {}),
      socketPath: config.socketPath,
      policyPath: config.policyPath,
      isDefault: active.name === registry.default,
    },
  }, prettyOutput);
}

export function runProfileSwitch(options: Options, prettyOutput: boolean): void {
  const name = options._[0];
  if (!name) {
    errorOut("Usage: tabctl profile-switch <name>");
  }
  const registry = loadProfiles();
  if (!(name in registry.profiles)) {
    errorOut(`Profile "${name}" not found`);
  }
  registry.default = name;
  saveProfiles(registry);
  printJson({
    ok: true,
    action: "profile-switch",
    data: { name, message: `Default profile set to "${name}"` },
  }, prettyOutput);
}

export function runProfileRemove(options: Options, prettyOutput: boolean): void {
  const name = options._[0];
  if (!name) {
    errorOut("Usage: tabctl profile-remove <name>");
  }
  try {
    const registry = removeProfile(name);
    printJson({
      ok: true,
      action: "profile-remove",
      data: { name, newDefault: registry.default, message: `Profile "${name}" removed` },
    }, prettyOutput);
  } catch (err) {
    errorOut((err as Error).message);
  }
}
