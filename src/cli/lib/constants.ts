import os from "os";
import path from "path";
import { stateHome } from "../../shared/config";

// Re-export version info from shared module
export { VERSION, BASE_VERSION, GIT_SHA, DIRTY } from "../../shared/version";

export const STATE_HOME = stateHome();
// Default socket path for Chrome installs (tabctl-chrome.sock).
export const SOCKET_PATH = path.join(STATE_HOME, "tabctl", "tabctl-chrome.sock");
export const LEGACY_SOCKET_PATH = path.join(os.homedir(), ".tabarchive", "tabarchive.sock");
export const HOST_NAME = "com.erwinkroon.tabctl";
export const HOST_DESCRIPTION = "Tab archive native host";
export const EXTENSION_ID_PATTERN = /^[a-p]{32}$/;

export const GROUP_COLORS = new Set([
  "grey",
  "blue",
  "red",
  "yellow",
  "green",
  "pink",
  "purple",
  "cyan",
  "orange",
]);

export const DEFAULT_PAGE_LIMIT = 100;
export const SKILL_NAME = "tabctl";
export const SKILL_REPO = process.env.TABCTL_SKILL_REPO || "https://github.com/ekroon/tabctl";

export const SUPPORTED_SIGNALS = ["page-meta", "github-state", "selector"] as const;
export const SUPPORTED_SIGNAL_SET = new Set<string>(SUPPORTED_SIGNALS);
