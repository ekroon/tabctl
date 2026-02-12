/**
 * Command module index.
 * Re-exports all command handlers and parameter builders.
 */

// Setup command
export { runSetup } from "./setup";

// Doctor command
export { runDoctor } from "./doctor";

// Meta commands (version, ping, skill, policy, history, undo)
export {
  runSkillInstall,
  runVersion,
  runPolicy,
  runHistory,
  runUndo,
  runPing,
} from "./meta";

// List commands (list, group-list)
export {
  runList,
  runGroupList,
} from "./list";

// Profile commands (profile-list, profile-show, profile-switch, profile-remove)
export { runProfileList, runProfileShow, runProfileSwitch, runProfileRemove } from "./profile";

// Parameter builders for all commands
export {
  buildAnalyzeParams,
  buildInspectParams,
  buildFocusParams,
  buildRefreshParams,
  buildOpenParams,
  buildGroupUpdateParams,
  buildGroupUngroupParams,
  buildGroupAssignParams,
  buildGroupGatherParams,
  buildMoveTabParams,
  buildMoveGroupParams,
  buildMergeWindowParams,
  buildArchiveParams,
  buildCloseParams,
  buildReportParams,
  buildScreenshotParams,
  buildHistoryParams,
  buildUndoParams,
} from "./params";
