// Shared dependency interface for extension modules.
// Each module uses Pick<ExtensionDeps, ...> to declare its actual requirements.

type GroupMatch = import("./groups").GroupMatch;

export interface ExtensionDeps {
  getTabSnapshot: () => Promise<{ generatedAt: number; windows: Array<Record<string, unknown>> }>;
  selectTabsByScope: (
    snapshot: { windows: Array<Record<string, unknown>> },
    params: Record<string, unknown>,
  ) => { tabs: Array<Record<string, unknown>>; error?: Record<string, unknown> };
  sendProgress: (id: string, payload: Record<string, unknown>) => void;
  log: (...args: Array<unknown>) => void;
  resolveWindowIdFromParams: (snapshot: { windows: Array<Record<string, unknown>> }, value: unknown) => number | null;
  resolveGroupByTitle: (snapshot: { windows: Array<Record<string, unknown>> }, groupTitle: string, windowId?: number) => { match?: GroupMatch; error?: Record<string, unknown> };
  resolveGroupById: (snapshot: { windows: Array<Record<string, unknown>> }, groupId: number) => { match?: GroupMatch; error?: Record<string, unknown> };
  buildWindowLabels: (snapshot: { windows: Array<{ windowId: number }> }) => Map<number, string>;
  getArchiveWindowId: () => Promise<number | null>;
  setArchiveWindowId: (id: number | null) => Promise<void>;
  delay: (ms: number) => Promise<unknown>;
  executeWithTimeout: <T>(
    tabId: number,
    timeoutMs: number,
    func: (...args: Array<any>) => T,
    args?: Array<unknown>,
  ) => Promise<T | null>;
  isScriptableUrl: (url: unknown) => boolean;
  waitForTabReady: (tabId: number, params: Record<string, unknown>, fallbackTimeoutMs: number) => Promise<void>;
}
