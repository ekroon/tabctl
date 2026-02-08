/**
 * Central source of truth for CLI options and command metadata.
 * This eliminates duplication between argument parsing and help display.
 *
 * Data definitions live in options-commands.ts and options-groups.ts;
 * this module re-exports them and provides validation helpers.
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

// ============================================================================
// Re-exports from split modules
// ============================================================================

export { OPTION_GROUPS, SCREENSHOT_OPTIONS } from "./options-groups";
export { COMMANDS } from "./options-commands";

import { OPTION_GROUPS } from "./options-groups";
import { COMMANDS } from "./options-commands";

const GLOBAL_FLAGS = ["help", "json", "pretty"] as const;

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

  flags.add("profile");

  return flags;
}

export function getCommandAllowedFlags(command: string): Set<string> {
  const flags = new Set<string>(GLOBAL_FLAGS);
  flags.add("profile");
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
