/**
 * Central source of truth for CLI options and command metadata.
 * This eliminates duplication between argument parsing and help display.
 */

// ============================================================================
// Type Definitions
// ============================================================================

export type OptionDef = {
  flag: string;
  desc: string;
  repeatable?: boolean;
};

export type OptionGroup = {
  name: string;
  description: string;
  options: readonly OptionDef[];
};

export type CommandMeta = {
  description: string;
  groups?: readonly string[];
  options?: readonly OptionDef[];
  aliases?: readonly string[];
};

const GLOBAL_FLAGS = ["help", "json", "pretty"] as const;

// ============================================================================
// Option Group Definitions
// ============================================================================

export const OPTION_GROUPS: Record<string, OptionGroup> = {
  scope: {
    name: "Scope Options",
    description: "Filter which tabs/groups to operate on",
    options: [
      { flag: "--tab <id>", desc: "Target specific tab(s) by ID", repeatable: true },
      { flag: "--group <name>", desc: "Target tabs in group by title" },
      { flag: "--group-id <id>", desc: "Target group by ID (use -1 for ungrouped)" },
       { flag: "--ungrouped", desc: "Alias for --group-id -1" },
       { flag: "--window <id|active|last-focused>", desc: "Target tabs in specific window" },
      { flag: "--all", desc: "Target all eligible tabs" },
    ],
  },
  pagination: {
    name: "Pagination Options",
    description: "Control result paging (default limit: 100)",
    options: [
      { flag: "--limit <n>", desc: "Maximum items to return" },
      { flag: "--offset <n>", desc: "Skip first n items" },
      { flag: "--no-page", desc: "Disable pagination, return all results" },
    ],
  },
} as const;

export const SCREENSHOT_OPTIONS: OptionGroup = {
  name: "Screenshot Options",
  description: "Control screenshot capture",
  options: [
    { flag: "--mode viewport|full", desc: "Capture mode" },
    { flag: "--format png|jpeg", desc: "Image format" },
    { flag: "--quality <n>", desc: "JPEG quality (0-100)" },
    { flag: "--tile-max-dim <px>", desc: "Max tile dimension in pixels" },
    { flag: "--max-bytes <n>", desc: "Max bytes per tile" },
    { flag: "--wait-for load|dom|settle|none", desc: "Wait for page readiness before capture" },
    { flag: "--wait-timeout-ms <ms>", desc: "Timeout for page readiness wait" },
    { flag: "--out <dir>", desc: "Write files to directory" },
    { flag: "--progress", desc: "Show progress during capture" },
  ],
};

// ============================================================================
// Command Metadata
// ============================================================================

export const COMMANDS: Record<string, CommandMeta> = {
  help: {
    description: "Show help information",
  },
  list: {
    description: "List browser tabs",
    groups: ["scope", "pagination"],
    options: [
      { flag: "--groups", desc: "Alias for group-list command" },
    ],
  },
  analyze: {
    description: "Analyze tabs for duplicates and stale content",
    groups: ["scope"],
    options: [
      { flag: "--stale-days <n>", desc: "Days threshold for stale tabs" },
      { flag: "--github", desc: "Enable GitHub PR/issue status checking" },
      { flag: "--github-concurrency <n>", desc: "Max concurrent GitHub API requests" },
      { flag: "--github-timeout-ms <ms>", desc: "Timeout for GitHub API requests" },
      { flag: "--window-title", desc: "Include active window title in output" },
      { flag: "--progress", desc: "Show progress during analysis" },
    ],
  },
  dedupe: {
    description: "Interactively deduplicate tabs",
    groups: ["scope"],
    options: [
      { flag: "--stale-days <n>", desc: "Days threshold for stale tabs" },
      { flag: "--github", desc: "Enable GitHub PR/issue status checking" },
      { flag: "--github-concurrency <n>", desc: "Max concurrent GitHub API requests" },
      { flag: "--github-timeout-ms <ms>", desc: "Timeout for GitHub API requests" },
      { flag: "--include-stale", desc: "Include stale tabs in deduplication" },
      { flag: "--window-title", desc: "Include active window title in output" },
      { flag: "--progress", desc: "Show progress during analysis" },
      { flag: "--confirm", desc: "Execute changes without prompting" },
    ],
  },
  inspect: {
    description: "Extract signals from tab content",
    groups: ["scope", "pagination"],
    options: [
      { flag: "--signal-config <path>", desc: "Path to signal configuration file" },
      { flag: "--signal <id>", desc: "Signal ID to extract", repeatable: true },
       { flag: "--selector <name=css|json>", desc: "Custom selector definition (attr: href-url/src-url supported; text/textMode supported)", repeatable: true },
      { flag: "--selector-attr <attr>", desc: "Default selector attr (text|href|src|href-url|src-url)" },
      { flag: "--signal-concurrency <n>", desc: "Max concurrent signal extractions" },
      { flag: "--signal-timeout-ms <ms>", desc: "Timeout for signal extraction" },
      { flag: "--wait-for load|dom|settle|none", desc: "Wait for page readiness before inspection" },
      { flag: "--wait-timeout-ms <ms>", desc: "Timeout for page readiness wait" },
      { flag: "--progress", desc: "Show progress during inspection" },
    ],
  },
  screenshot: {
    description: "Capture screenshots from tabs",
    groups: ["scope"],
    options: SCREENSHOT_OPTIONS.options,
  },
  focus: {
    description: "Focus a specific tab",
    options: [
      { flag: "--tab <id>", desc: "Tab ID to focus" },
    ],
  },
  refresh: {
    description: "Refresh a specific tab",
    options: [
      { flag: "--tab <id>", desc: "Tab ID to refresh" },
    ],
  },
  open: {
    description: "Open new tabs with URLs",
    options: [
      { flag: "--url <url>", desc: "URL to open", repeatable: true },
      { flag: "--group <name>", desc: "Add tabs to group by name" },
      { flag: "--color <name>", desc: "Group color (if creating)" },
      { flag: "--before-tab <id>", desc: "Position before this tab" },
      { flag: "--after-tab <id>", desc: "Position after this tab" },
      { flag: "--after-group <name>", desc: "Position after this group" },
      { flag: "--window <id|active|last-focused|new>", desc: "Target window ID" },
      { flag: "--new-window", desc: "Open in new window" },
      { flag: "--window-group <name>", desc: "Find window containing group" },
      { flag: "--window-tab <id>", desc: "Find window containing tab" },
      { flag: "--window-url <substring>", desc: "Find window containing URL" },
    ],
  },
  "group-list": {
    description: "List tab groups",
    groups: ["scope", "pagination"],
  },
  group: {
    description: "Alias for group-list",
    aliases: ["group-list"],
    groups: ["scope", "pagination"],
  },
  "group-update": {
    description: "Update tab group properties",
    options: [
      { flag: "--group <name>", desc: "Target group by title" },
      { flag: "--group-id <id>", desc: "Target group by ID" },
      { flag: "--window <id|active|last-focused>", desc: "Target window ID" },
      { flag: "--title <name>", desc: "New group title" },
      { flag: "--color <name>", desc: "New group color" },
      { flag: "--collapsed", desc: "Collapse the group" },
      { flag: "--expanded", desc: "Expand the group" },
    ],
  },
  "group-ungroup": {
    description: "Remove tabs from a group",
    options: [
      { flag: "--group <name>", desc: "Target group by title" },
      { flag: "--group-id <id>", desc: "Target group by ID" },
      { flag: "--window <id|active|last-focused>", desc: "Target window ID" },
    ],
  },
  "group-assign": {
    description: "Assign tabs to a group",
    options: [
      { flag: "--tab <id>", desc: "Tab ID(s) to assign", repeatable: true },
      { flag: "--group <name>", desc: "Target group by title" },
      { flag: "--group-id <id>", desc: "Target group by ID" },
      { flag: "--window <id|active|last-focused>", desc: "Target window ID" },
      { flag: "--create", desc: "Create group if not exists" },
      { flag: "--color <name>", desc: "Group color (if creating)" },
      { flag: "--collapsed", desc: "Collapse group after assign" },
      { flag: "--expanded", desc: "Expand group after assign" },
    ],
  },
  "move-tab": {
    description: "Move a tab to a new position",
    options: [
      { flag: "--tab <id>", desc: "Tab ID to move" },
      { flag: "--before-tab <id>", desc: "Position before this tab" },
      { flag: "--after-tab <id>", desc: "Position after this tab" },
      { flag: "--before-group <name>", desc: "Position before this group" },
      { flag: "--after-group <name>", desc: "Position after this group" },
      { flag: "--window <id|active|last-focused>", desc: "Target window ID" },
      { flag: "--new-window", desc: "Move to new window" },
    ],
  },
  "move-group": {
    description: "Move a tab group to a new position",
    options: [
      { flag: "--group <name>", desc: "Target group by title" },
      { flag: "--group-id <id>", desc: "Target group by ID" },
      { flag: "--before-tab <id>", desc: "Position before this tab" },
      { flag: "--after-tab <id>", desc: "Position after this tab" },
      { flag: "--before-group <name>", desc: "Position before this group" },
      { flag: "--after-group <name>", desc: "Position after this group" },
      { flag: "--window <id|active|last-focused>", desc: "Target window ID" },
      { flag: "--new-window", desc: "Move to new window" },
    ],
  },
  "merge-window": {
    description: "Merge tabs from one window to another",
    options: [
      { flag: "--from <id>", desc: "Source window ID" },
      { flag: "--to <id>", desc: "Destination window ID" },
      { flag: "--close-source", desc: "Close source window after merge" },
      { flag: "--confirm", desc: "Execute without prompting" },
    ],
  },
  setup: {
    description: "Configure tabctl connection",
    options: [
      { flag: "--browser edge|chrome", desc: "Browser type" },
      { flag: "--extension-id <id>", desc: "Extension ID to connect to" },
      { flag: "--node <path>", desc: "Path to Node.js executable" },
    ],
  },
  policy: {
    description: "Manage browser policies",
    options: [
      { flag: "--init", desc: "Initialize policy configuration" },
    ],
  },
  archive: {
    description: "Archive tabs to storage",
    groups: ["scope"],
  },
  close: {
    description: "Close tabs",
    options: [
      { flag: "--apply <analysisId>", desc: "Apply analysis results" },
      { flag: "--tab <id>", desc: "Tab ID(s) to close", repeatable: true },
      { flag: "--group <name>", desc: "Close tabs in group by title" },
      { flag: "--group-id <id>", desc: "Close tabs in group by ID" },
      { flag: "--ungrouped", desc: "Close ungrouped tabs" },
      { flag: "--window <id|active|last-focused>", desc: "Target window ID" },
      { flag: "--confirm", desc: "Execute without prompting" },
      { flag: "--dry-run", desc: "Show what would be closed" },
    ],
  },
  report: {
    description: "Generate tab reports",
    groups: ["scope", "pagination"],
    options: [
      { flag: "--format json|md|csv", desc: "Output format" },
      { flag: "--out <path>", desc: "Output file path" },
    ],
  },
  undo: {
    description: "Undo a previous operation",
    options: [
      { flag: "<txid>", desc: "Transaction ID (positional)" },
      { flag: "--txid <id>", desc: "Transaction ID" },
      { flag: "--latest", desc: "Undo most recent transaction" },
    ],
  },
  history: {
    description: "Show operation history",
    options: [
      { flag: "--limit <n>", desc: "Maximum entries to show" },
    ],
  },
  skill: {
    description: "Show AI agent skill documentation",
    options: [
      { flag: "--agent <name>", desc: "Target agent(s)", repeatable: true },
      { flag: "--global", desc: "Show global skill info" },
    ],
  },
  version: {
    description: "Show version information",
  },
  ping: {
    description: "Test connection to browser extension",
  },
  "reload-extension": {
    description: "Reload the browser extension",
  },
  "extension-reload": {
    description: "Alias for reload-extension",
    aliases: ["reload-extension"],
  },
} as const;

// ============================================================================
// Helper Functions
// ============================================================================

/**
 * Get all allowed flags for argument parsing validation.
 */
export function getBooleanFlags(): Set<string> {
  const flags = new Set<string>();

  GLOBAL_FLAGS.forEach((flag) => flags.add(flag));

  const addFromOptions = (options: readonly OptionDef[]) => {
    for (const opt of options) {
      if (!/^--[a-z-]+$/.test(opt.flag.trim())) {
        continue;
      }
      const match = opt.flag.match(/^--([a-z-]+)/);
      if (match) {
        flags.add(match[1]);
      }
    }
  };

  for (const group of Object.values(OPTION_GROUPS)) {
    addFromOptions(group.options);
  }

  for (const cmd of Object.values(COMMANDS)) {
    if (cmd.options) {
      addFromOptions(cmd.options);
    }
  }

  return flags;
}

export function getAllowedFlags(): Set<string> {
  const flags = new Set<string>();

  for (const flag of getBooleanFlags()) {
    flags.add(flag);
  }

  // Add value flags from option groups
  for (const group of Object.values(OPTION_GROUPS)) {
    for (const opt of group.options) {
      const match = opt.flag.match(/^--([a-z-]+)/);
      if (match) flags.add(match[1]);
    }
  }

  // Add value flags from command options
  for (const cmd of Object.values(COMMANDS)) {
    for (const opt of cmd.options || []) {
      const match = opt.flag.match(/^--([a-z-]+)/);
      if (match) flags.add(match[1]);
    }
  }

  return flags;
}

export function getCommandAllowedFlags(command: string): Set<string> {
  const flags = new Set<string>(GLOBAL_FLAGS);
  const meta = COMMANDS[command];

  const addOptions = (options: readonly OptionDef[] | undefined) => {
    if (!options) {
      return;
    }
    for (const opt of options) {
      const match = opt.flag.match(/^--([a-z-]+)/);
      if (match) {
        flags.add(match[1]);
      }
    }
  };

  if (meta?.groups) {
    for (const groupKey of meta.groups) {
      addOptions(OPTION_GROUPS[groupKey]?.options);
    }
  }

  addOptions(meta?.options);

  return flags;
}

/**
 * Get the option groups for a command.
 */
export function getCommandGroups(command: string): string[] {
  const meta = COMMANDS[command];
  return meta?.groups ? [...meta.groups] : [];
}

/**
 * Get command-specific options (not from groups).
 */
export function getCommandOptions(command: string): OptionDef[] {
  const meta = COMMANDS[command];
  return meta?.options ? [...meta.options] : [];
}

/**
 * Check if a command supports a specific option group.
 */
export function commandSupportsGroup(command: string, group: string): boolean {
  const meta = COMMANDS[command];
  return meta?.groups?.includes(group) ?? false;
}
