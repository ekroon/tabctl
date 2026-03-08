// Lightweight Chrome API stub for unit tests.
// Records calls and returns predictable results without a real browser.

export type PortStub = {
  postMessage: (msg: unknown) => void;
  onMessage: { addListener: (fn: (msg: unknown) => void) => void };
  onDisconnect: { addListener: (fn: () => void) => void };
  _sentMessages: Array<unknown>;
  _messageListeners: Array<(msg: unknown) => void>;
  _disconnectListeners: Array<() => void>;
  _simulateMessage: (msg: unknown) => void;
  _simulateDisconnect: () => void;
};

export type ChromeStub = {
  runtime: {
    getManifest: () => { version: string; version_name?: string };
    connectNative: (name: string) => PortStub;
    onInstalled: { addListener: (fn: () => void) => void };
    onStartup: { addListener: (fn: () => void) => void };
    lastError: { message: string } | null;
    id: string;
    reload: () => void;
  };
  tabs: {
    onCreated: { addListener: (fn: (tab: object) => void) => void };
    onUpdated: { addListener: (fn: (tabId: number, changeInfo: object, tab: object) => void) => void };
    onMoved: { addListener: (fn: (tabId: number, moveInfo: object) => void) => void };
    onAttached: { addListener: (fn: (tabId: number, attachInfo: object) => void) => void };
    onDetached: { addListener: (fn: (tabId: number, detachInfo: object) => void) => void };
    onRemoved: { addListener: (fn: (tabId: number, removeInfo: object) => void) => void };
    onActivated: { addListener: (fn: (activeInfo: object) => void) => void };
    get: (tabId: number) => Promise<object>;
    query: (queryInfo: object) => Promise<Array<object>>;
    create: (createProperties: object) => Promise<object>;
    update: (tabId: number, updateProperties: object) => Promise<object>;
    move: (tabIds: number | number[], moveProperties: object) => Promise<object>;
    remove: (tabIds: number | number[]) => Promise<void>;
    reload: (tabId: number) => Promise<void>;
    group: (options: object) => Promise<number>;
    ungroup: (tabIds: number[]) => Promise<void>;
  };
  tabGroups: {
    onCreated: { addListener: (fn: (group: object) => void) => void };
    onUpdated: { addListener: (fn: (group: object) => void) => void };
    onMoved: { addListener: (fn: (group: object) => void) => void };
    onRemoved: { addListener: (fn: (groupId: number, removeInfo: object) => void) => void };
    query: (queryInfo: object) => Promise<Array<object>>;
    update: (groupId: number, updateProperties: object) => Promise<object>;
  };
  windows: {
    onCreated: { addListener: (fn: (win: object) => void) => void };
    onRemoved: { addListener: (fn: (windowId: number) => void) => void };
    onFocusChanged: { addListener: (fn: (windowId: number) => void) => void };
    getAll: (getInfo: object) => Promise<Array<object>>;
    create: (createData: object) => Promise<object>;
    remove: (windowId: number) => Promise<void>;
    update: (windowId: number, updateInfo: object) => Promise<object>;
  };
  alarms: {
    create: (name: string, alarmInfo: object) => void;
    onAlarm: { addListener: (fn: (alarm: { name: string }) => void) => void };
  };
  scripting: {
    executeScript: (injection: object) => Promise<Array<{ result?: unknown }>>;
  };
};

export function makePort(): PortStub {
  const stub: PortStub = {
    _sentMessages: [],
    _messageListeners: [],
    _disconnectListeners: [],
    postMessage(msg) { this._sentMessages.push(msg); },
    onMessage: { addListener(fn) { stub._messageListeners.push(fn); } },
    onDisconnect: { addListener(fn) { stub._disconnectListeners.push(fn); } },
    _simulateMessage(msg) { stub._messageListeners.forEach((fn) => fn(msg)); },
    _simulateDisconnect() { stub._disconnectListeners.forEach((fn) => fn()); },
  };
  return stub;
}

export function createChromeStub(): ChromeStub {
  const noop = () => {};
  const noopListener = { addListener: noop };
  const ports: PortStub[] = [];

  return {
    runtime: {
      getManifest: () => ({ version: "1.0.0", version_name: "1.0.0" }),
      connectNative: (_name: string) => {
        const port = makePort();
        ports.push(port);
        return port;
      },
      onInstalled: noopListener,
      onStartup: noopListener,
      lastError: null,
      id: "test-extension-id",
      reload: noop,
    },
    tabs: {
      onCreated: noopListener,
      onUpdated: noopListener,
      onMoved: noopListener,
      onAttached: noopListener,
      onDetached: noopListener,
      onRemoved: noopListener,
      onActivated: noopListener,
      get: async () => ({}),
      query: async () => [],
      create: async (p) => p,
      update: async (_id, p) => p,
      move: async (_ids, p) => p,
      remove: async () => {},
      reload: async () => {},
      group: async () => 1,
      ungroup: async () => {},
    },
    tabGroups: {
      onCreated: noopListener,
      onUpdated: noopListener,
      onMoved: noopListener,
      onRemoved: noopListener,
      query: async () => [],
      update: async (_id, p) => p,
    },
    windows: {
      onCreated: noopListener,
      onRemoved: noopListener,
      onFocusChanged: noopListener,
      getAll: async () => [],
      create: async (p) => p,
      remove: async () => {},
      update: async (_id, p) => p,
    },
    alarms: {
      create: noop,
      onAlarm: noopListener,
    },
    scripting: {
      executeScript: async () => [],
    },
  };
}
