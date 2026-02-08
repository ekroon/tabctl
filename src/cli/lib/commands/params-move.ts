/**
 * Movement-related parameter builders.
 */

import { parseWindowScope } from "./params";
import type { Options } from "../types";

export function buildMoveTabParams(options: Options): Record<string, unknown> {
  const windowValue = parseWindowScope(options.window, { allowNew: false });
  return {
    tabId: options.tab ? Number((options.tab as string[])[0]) : undefined,
    tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
    beforeTabId: options["before-tab"] ? Number(options["before-tab"]) : undefined,
    afterTabId: options["after-tab"] ? Number(options["after-tab"]) : undefined,
    beforeGroupTitle: options["before-group"],
    afterGroupTitle: options["after-group"],
    windowId: windowValue,
    newWindow: options["new-window"] === true,
  };
}

export function buildMoveGroupParams(options: Options): Record<string, unknown> {
  const windowValue = parseWindowScope(options.window, { allowNew: false });
  return {
    groupTitle: options.group,
    groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
    beforeTabId: options["before-tab"] ? Number(options["before-tab"]) : undefined,
    afterTabId: options["after-tab"] ? Number(options["after-tab"]) : undefined,
    beforeGroupTitle: options["before-group"],
    afterGroupTitle: options["after-group"],
    windowId: windowValue,
    newWindow: options["new-window"] === true,
  };
}

export function buildMergeWindowParams(options: Options): Record<string, unknown> {
  return {
    fromWindowId: options.from ? Number(options.from) : undefined,
    toWindowId: options.to ? Number(options.to) : undefined,
    windowId: options.from ? Number(options.from) : undefined,
    closeSource: options["close-source"] === true,
    confirmed: options.confirm === true,
  };
}
