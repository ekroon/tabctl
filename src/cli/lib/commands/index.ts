/**
 * Command module index.
 * Re-exports all command handlers and parameter builders.
 */

// Meta commands (version, ping, setup, skill, policy, history, undo)
export {
  runSetup,
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
