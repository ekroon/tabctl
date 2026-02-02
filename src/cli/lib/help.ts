/**
 * Help generation using option groups from options.ts as the source of truth.
 * This eliminates duplication by referencing option groups instead of repeating
 * options for every command.
 */

import { VERSION } from "./constants";
import { OPTION_GROUPS, COMMANDS, SCREENSHOT_OPTIONS, type OptionDef } from "./options";
import { printJson } from "./output";

// ============================================================================
// Types
// ============================================================================

type HelpOptionGroup = {
  name: string;
  description: string;
  options: string[];
};

type HelpCommand = {
  name: string;
  description: string;
  groups?: string[];
  options?: string[];
};

type HelpData = {
  version: string;
  usage: string;
  commands: HelpCommand[];
  optionGroups: HelpOptionGroup[];
  globalOptions: string[];
  notes: string[];
};

// ============================================================================
// Help Data Generation
// ============================================================================

function formatOption(opt: OptionDef): string {
  if (opt.repeatable) {
    return `${opt.flag} (repeatable)`;
  }
  return opt.flag;
}

/**
 * Build structured help data from COMMANDS and OPTION_GROUPS.
 */
export function buildHelpData(): HelpData {
  // Build option groups
  const optionGroups: HelpOptionGroup[] = Object.entries(OPTION_GROUPS).map(
    ([_key, group]) => ({
      name: group.name,
      description: group.description,
      options: group.options.map(formatOption),
    })
  );
  optionGroups.push({
    name: SCREENSHOT_OPTIONS.name,
    description: SCREENSHOT_OPTIONS.description,
    options: SCREENSHOT_OPTIONS.options.map(formatOption),
  });

  // Build commands list with their groups and specific options
  const commands: HelpCommand[] = Object.entries(COMMANDS)
    .map(([name, meta]) => {
      const cmd: HelpCommand = {
        name,
        description: meta.description,
      };

      if (meta.groups && meta.groups.length > 0) {
        cmd.groups = [...meta.groups];
      }

      if (meta.options && meta.options.length > 0) {
        cmd.options = meta.options.map(formatOption);
      }

      return cmd;
    });

  // Global options
  const globalOptions = ["--help", "--json", "--pretty"];

  // Notes
  const notes = [
    "--before-group/--after-group only position tabs; use group-assign to move tabs into a group.",
    "undo accepts a txid as a positional arg (or --txid) and supports --latest.",
    "screenshot uses --out to write per-tab folders under the target directory.",
    "Use selector attr href-url/src-url to resolve absolute http(s) links.",
  ];

  return {
    version: VERSION,
    usage: "tabctl <command> [options]",
    commands,
    optionGroups,
    globalOptions,
    notes,
  };
}

// ============================================================================
// Text Output Formatting
// ============================================================================

/**
 * Print help in human-readable text format.
 */
function printHelpText(data: HelpData): void {
  const lines: string[] = [];

  // Header
  lines.push("tabctl - Edge tab management CLI");
  lines.push(`Version: ${data.version}`);
  lines.push("");
  lines.push(`Usage: ${data.usage}`);
  lines.push("");

  // Commands grouped by category
  lines.push("Commands:");
  const commandNames = data.commands.map((c) => c.name);
  lines.push(`  ${commandNames.join(", ")}`);
  lines.push("");

  // Option Groups
  lines.push("Option Groups:");
  lines.push("  (Commands reference these groups; see command details below)");
  lines.push("");
  for (const group of data.optionGroups) {
    lines.push(`  [${group.name}] - ${group.description}`);
    for (const opt of group.options) {
      lines.push(`    ${opt}`);
    }
    lines.push("");
  }

  // Command Details
  lines.push("Command Details:");
  for (const cmd of data.commands) {
    const parts: string[] = [`  ${cmd.name}`];
    if (cmd.description) {
      parts.push(`- ${cmd.description}`);
    }
    lines.push(parts.join(" "));

    // Show which groups the command uses
    if (cmd.groups && cmd.groups.length > 0) {
      const groupRefs = cmd.groups.map((g) => {
        const group = OPTION_GROUPS[g];
        return group ? `[${group.name}]` : `[${g}]`;
      });
      lines.push(`    Uses: ${groupRefs.join(", ")}`);
    }

    // Show command-specific options
    if (cmd.options && cmd.options.length > 0) {
      lines.push("    Options:");
      for (const opt of cmd.options) {
        lines.push(`      ${opt}`);
      }
    }
  }
  lines.push("");

  // Global Options
  lines.push("Global Options:");
  for (const opt of data.globalOptions) {
    lines.push(`  ${opt}`);
  }
  lines.push("");

  // Notes
  lines.push("Notes:");
  for (const note of data.notes) {
    lines.push(`  ${note}`);
  }
  lines.push("");

  // Policy location
  lines.push("Policy: $XDG_CONFIG_HOME/tabctl/policy.json (or ~/.config/tabctl/policy.json)");
  lines.push("Policy is enforced when the file exists; missing file means no policy.");

  process.stdout.write(lines.join("\n") + "\n");
}

// ============================================================================
// Public API
// ============================================================================

/**
 * Print help output in either JSON or text format.
 */
export function printHelp(jsonOutput: boolean): void {
  const data = buildHelpData();
  if (jsonOutput) {
    printJson({ ok: true, data });
    return;
  }
  printHelpText(data);
}
