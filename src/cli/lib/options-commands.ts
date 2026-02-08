/**
 * Command metadata definitions for CLI commands.
 */
import type { CommandMeta } from "./options";
import { SCREENSHOT_OPTIONS } from "./options-groups";

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
      { flag: "--window-title", desc: "Include active window title in output" },
      { flag: "--progress", desc: "Show progress during analysis" },
    ],
  },
  dedupe: {
    description: "Interactively deduplicate tabs",
    groups: ["scope"],
    options: [
      { flag: "--stale-days <n>", desc: "Days threshold for stale tabs" },
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
      { flag: "--name <name>", desc: "Profile name (default: browser type)" },
      { flag: "--user-data-dir <path>", desc: "Chrome/Edge user data directory for custom profiles" },
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
  "profile-list": {
    description: "List configured profiles",
  },
  profile: {
    description: "Alias for profile-list",
    aliases: ["profile-list"],
  },
  "profile-show": {
    description: "Show active profile details",
  },
  "profile-switch": {
    description: "Switch the default profile",
    options: [
      { flag: "<name>", desc: "Profile name (positional)" },
    ],
  },
  "profile-remove": {
    description: "Remove a profile",
    options: [
      { flag: "<name>", desc: "Profile name (positional)" },
    ],
  },
  version: {
    description: "Show version information",
  },
  ping: {
    description: "Test connection to browser extension",
  },
  reload: {
    description: "Reload the browser extension (internal, used for upgrades)",
  },
} as const;
