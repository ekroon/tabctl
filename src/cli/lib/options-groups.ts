/**
 * Option group definitions for CLI commands.
 */
import type { OptionGroup } from "./options";

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
