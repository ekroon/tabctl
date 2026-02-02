// Re-export PolicyContext for convenience
import type { PolicyContext } from "./policy";
export type { PolicyContext };

/**
 * Parsed CLI arguments from minimist
 */
export type Options = {
  _: string[];
  [key: string]: unknown;
};

/**
 * Flags extracted for scope resolution
 */
export type ScopeFlags = {
  tabIds: number[];
  groupTitle: string;
  groupId: number | null;
  windowId: number | null;
  hasScope: boolean;
  ungrouped: boolean;
};

/**
 * Parameters extracted for API calls
 */
export type ScopeParams = {
  tabIds?: number[];
  groupTitle?: string;
  groupId?: number;
  windowId?: number;
  all?: boolean;
};

/**
 * Pagination information returned with list results
 */
export type PageInfo = {
  offset: number;
  limit: number;
  returned: number;
  total: number;
  hasMore: boolean;
  nextOffset: number | null;
  hint: string | null;
};

/**
 * Result of pagination option parsing
 */
export type PaginationResult = {
  offset: number;
  limit: number;
  page: PageInfo | null;
};

/**
 * Callback for progress updates during long operations
 */
export type ProgressCallback = (data: Record<string, unknown>) => void;

/**
 * Result of tab selection operations
 */
export type SelectionResult = {
  tabs: Array<Record<string, unknown>>;
  error?: {
    message: string;
    hint?: string;
    matches?: Array<{ windowId: number; groupId: number; windowLabel: string | null }>;
    availableGroups?: Array<Record<string, unknown>>;
  };
};

/**
 * Context passed to command handlers
 */
export type CommandContext = {
  command: string;
  options: Options;
  policy: PolicyContext;
  policySummary: Record<string, unknown>;
  prettyOutput: boolean;
};

/**
 * Result returned from command handlers
 */
export type CommandResult = {
  response: Record<string, unknown>;
  exitCode?: number;
};

/**
 * Command handler function signature
 */
export type CommandHandler = (ctx: CommandContext) => Promise<CommandResult>;
