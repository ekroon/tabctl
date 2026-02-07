import path from "path";
import fs from "fs";
import { resolveConfig } from "./config";

export type ProfileEntry = {
  browser: "edge" | "chrome";
  extensionId: string;
  nodePath: string;
  hostPath: string;
  dataDir: string;
  userDataDir?: string;
};

export type ProfileRegistry = {
  default: string | null;
  profiles: Record<string, ProfileEntry>;
};

export const PROFILE_NAME_PATTERN = /^[a-z0-9-]+$/;
export const PROFILES_FILE = "profiles.json";

export function validateProfileName(name: string): void {
  if (!PROFILE_NAME_PATTERN.test(name)) {
    throw new Error(
      `Invalid profile name "${name}": only lowercase letters, digits, and hyphens are allowed`,
    );
  }
}

export function loadProfiles(configDir?: string): ProfileRegistry {
  const dir = configDir ?? resolveConfig().configDir;
  try {
    const raw = fs.readFileSync(path.join(dir, PROFILES_FILE), "utf-8");
    return JSON.parse(raw) as ProfileRegistry;
  } catch {
    return { default: null, profiles: {} };
  }
}

export function saveProfiles(registry: ProfileRegistry, configDir?: string): void {
  const dir = configDir ?? resolveConfig().configDir;
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, PROFILES_FILE), JSON.stringify(registry, null, 2) + "\n");
}

export function addProfile(name: string, entry: ProfileEntry, configDir?: string): ProfileRegistry {
  validateProfileName(name);
  const registry = loadProfiles(configDir);
  const isFirst = Object.keys(registry.profiles).length === 0;
  registry.profiles[name] = entry;
  if (isFirst) {
    registry.default = name;
  }
  saveProfiles(registry, configDir);
  return registry;
}

export function removeProfile(name: string, configDir?: string): ProfileRegistry {
  const registry = loadProfiles(configDir);
  if (!(name in registry.profiles)) {
    throw new Error(`Profile "${name}" does not exist`);
  }
  delete registry.profiles[name];
  if (registry.default === name) {
    const remaining = Object.keys(registry.profiles);
    registry.default = remaining.length > 0 ? remaining[0] : null;
  }
  saveProfiles(registry, configDir);
  return registry;
}

export function getActiveProfile(
  overrideName?: string,
  configDir?: string,
): { name: string; profile: ProfileEntry } | null {
  const registry = loadProfiles(configDir);
  const name = overrideName ?? process.env.TABCTL_PROFILE ?? registry.default;
  if (!name || !(name in registry.profiles)) {
    return null;
  }
  return { name, profile: registry.profiles[name] };
}

export function listProfiles(
  configDir?: string,
): Array<{ name: string; profile: ProfileEntry; isDefault: boolean }> {
  const registry = loadProfiles(configDir);
  return Object.entries(registry.profiles).map(([name, profile]) => ({
    name,
    profile,
    isDefault: name === registry.default,
  }));
}
