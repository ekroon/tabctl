import { GROUP_COLORS, SUPPORTED_SIGNALS, SUPPORTED_SIGNAL_SET } from "./constants";
import { getAllowedFlags, getBooleanFlags, getCommandAllowedFlags, COMMANDS } from "./options";
import { errorOut } from "./output";
import type { Options } from "./types";

export function normalizeGroupColor(value: unknown): string | undefined {
  if (typeof value !== "string") {
    return undefined;
  }
  const trimmed = value.trim().toLowerCase();
  if (!trimmed) {
    return undefined;
  }
  if (!GROUP_COLORS.has(trimmed)) {
    errorOut(`Invalid color: ${value}. Use one of: ${Array.from(GROUP_COLORS).join(", ")}`);
  }
  return trimmed;
}

export function normalizeSignals(value: unknown): string[] {
  if (!Array.isArray(value)) {
    return [] as string[];
  }
  return value.map((signal) => String(signal).trim()).filter(Boolean);
}

export function validateSignals(signals: string[]): void {
  for (const signal of signals) {
    if (!SUPPORTED_SIGNAL_SET.has(signal)) {
      errorOut(`Unknown signal: ${signal}. Use one of: ${SUPPORTED_SIGNALS.join(", ")}`);
    }
  }
}

function normalizeCommand(value: string | undefined): string | undefined {
  if (!value) {
    return value;
  }
  if (value === "groups" || value === "group") {
    return "group-list";
  }
  const meta = COMMANDS[value];
  if (!meta?.aliases || meta.aliases.length === 0) {
    return value;
  }
  return meta.aliases[0] ?? value;
}

export function parseArgs(argv: string[]): { command: string | undefined; options: Options; warnings: string[] } {
  const args = [...argv];
  let command: string | undefined;
  const options: Options = { _: [] };
  const warnings: string[] = [];
  const pendingFlags: string[] = [];
  const allowedFlags = getAllowedFlags();
  const booleanFlags = getBooleanFlags();

  while (args.length > 0) {
    const arg = args.shift() as string;
    if (!arg.startsWith("--")) {
      if (!command) {
        command = normalizeCommand(arg);
        if (command) {
          const commandAllowedFlags = getCommandAllowedFlags(command);
          for (const pending of pendingFlags) {
            if (!commandAllowedFlags.has(pending)) {
              warnings.push(`--${pending} is not supported by ${command}`);
            }
          }
        }
        continue;
      }
      options._.push(arg);
      continue;
    }

    const key = arg.slice(2);
    if (!allowedFlags.has(key)) {
      if (key === "format") {
        errorOut("Unknown option: --format");
      }
      errorOut(`Unknown option: --${key}`);
    }

    if (command) {
      const commandAllowedFlags = getCommandAllowedFlags(command);
      if (!commandAllowedFlags.has(key)) {
        warnings.push(`--${key} is not supported by ${command}`);
      }
    } else {
      pendingFlags.push(key);
    }
    
    // Boolean flags (no value needed)
    if (booleanFlags.has(key)) {
      options[key] = true;
      continue;
    }

    // Value required
    const value = args.shift();
    if (value == null) {
      errorOut(`Missing value for --${key}`);
    }
    
    // Repeatable flags (accumulate into arrays)
    if (key === "signal") {
      if (!options.signal) {
        options.signal = [];
      }
      (options.signal as string[]).push(value as string);
      continue;
    }
    if (key === "tab") {
      if (!options.tab) {
        options.tab = [];
      }
      (options.tab as string[]).push(value as string);
      continue;
    }
    if (key === "agent") {
      if (!options.agent) {
        options.agent = [];
      }
      (options.agent as string[]).push(value as string);
      continue;
    }
    if (key === "url") {
      if (!options.url) {
        options.url = [];
      }
      (options.url as string[]).push(value as string);
      continue;
    }
    if (key === "selector") {
      if (!options.selector) {
        options.selector = [];
      }
      (options.selector as string[]).push(value as string);
      continue;
    }
    
    // Single value flags
    options[key] = value;
  }

  return { command, options, warnings };
}
