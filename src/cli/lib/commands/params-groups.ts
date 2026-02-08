/**
 * Group-related parameter builders.
 */

import { parseWindowScope } from "./params";
import type { Options } from "../types";

export function buildGroupUpdateParams(options: Options): Record<string, unknown> {
  const windowValue = parseWindowScope(options.window, { allowNew: false });
  return {
    groupTitle: options.group,
    groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
    windowId: windowValue,
    title: options.title,
    color: options.color,
    collapsed: options.collapsed === true ? true : options.expanded === true ? false : undefined,
  };
}

export function buildGroupUngroupParams(options: Options): Record<string, unknown> {
  const windowValue = parseWindowScope(options.window, { allowNew: false });
  return {
    groupTitle: options.group,
    groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
    windowId: windowValue,
  };
}

export function buildGroupAssignParams(options: Options): Record<string, unknown> {
  const windowValue = parseWindowScope(options.window, { allowNew: false });
  return {
    tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
    groupTitle: options.group,
    groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
    windowId: windowValue,
    create: Boolean(options.create),
    color: options.color,
    collapsed: options.collapsed === true ? true : options.expanded === true ? false : undefined,
  };
}
