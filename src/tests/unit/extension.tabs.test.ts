import assert from "node:assert/strict";
import test from "node:test";
import {
  normalizeUrl,
  normalizeTabIndex,
  getMostRecentFocusedWindowId,
  resolveOpenWindow,
  openTabs,
} from "../../extension/lib/tabs";

// ============================================================================
// normalizeUrl — used for duplicate URL detection in group reuse
// ============================================================================

test("normalizeUrl returns normalized http URL", () => {
  assert.equal(normalizeUrl("https://example.com/page"), "https://example.com/page");
});

test("normalizeUrl strips hash fragments", () => {
  assert.equal(normalizeUrl("https://example.com/page#section"), "https://example.com/page");
});

test("normalizeUrl strips utm params", () => {
  assert.equal(
    normalizeUrl("https://example.com/page?utm_source=twitter&utm_medium=social"),
    "https://example.com/page",
  );
});

test("normalizeUrl strips tracking params but keeps others", () => {
  assert.equal(
    normalizeUrl("https://example.com/search?q=hello&utm_source=twitter&page=2"),
    "https://example.com/search?q=hello&page=2",
  );
});

test("normalizeUrl strips fbclid and gclid", () => {
  assert.equal(
    normalizeUrl("https://example.com/?fbclid=abc123&gclid=def456"),
    "https://example.com/",
  );
});

test("normalizeUrl returns null for non-http protocols", () => {
  assert.equal(normalizeUrl("file:///path/to/file"), null);
  assert.equal(normalizeUrl("data:text/html,hello"), null);
  assert.equal(normalizeUrl("chrome://extensions"), null);
  assert.equal(normalizeUrl("about:blank"), null);
});

test("normalizeUrl returns null for invalid input", () => {
  assert.equal(normalizeUrl(null), null);
  assert.equal(normalizeUrl(undefined), null);
  assert.equal(normalizeUrl(""), null);
  assert.equal(normalizeUrl(42), null);
  assert.equal(normalizeUrl("not a url"), null);
});

test("normalizeUrl treats same origin+pathname as equal after normalization", () => {
  const base = normalizeUrl("https://github.com/ekroon/tabctl");
  const withHash = normalizeUrl("https://github.com/ekroon/tabctl#readme");
  const withUtm = normalizeUrl("https://github.com/ekroon/tabctl?utm_source=docs");
  assert.equal(base, withHash);
  assert.equal(base, withUtm);
});

test("normalizeUrl preserves meaningful query params", () => {
  const url = normalizeUrl("https://example.com/search?q=tabctl&lang=en");
  assert.equal(url, "https://example.com/search?q=tabctl&lang=en");
});

test("normalizeUrl strips custom utm_ prefixed params", () => {
  assert.equal(
    normalizeUrl("https://example.com/?utm_custom=value"),
    "https://example.com/",
  );
});

// ============================================================================
// normalizeTabIndex — used for insertion index calculation
// ============================================================================

test("normalizeTabIndex returns number for valid input", () => {
  assert.equal(normalizeTabIndex(0), 0);
  assert.equal(normalizeTabIndex(5), 5);
  assert.equal(normalizeTabIndex("3"), 3);
});

test("normalizeTabIndex returns null for invalid input", () => {
  assert.equal(normalizeTabIndex(NaN), null);
  assert.equal(normalizeTabIndex(Infinity), null);
  assert.equal(normalizeTabIndex("not-a-number"), null);
  assert.equal(normalizeTabIndex(undefined), null);
  // Note: Number(null) === 0, so normalizeTabIndex(null) returns 0
  assert.equal(normalizeTabIndex(null), 0);
});

// ============================================================================
// getMostRecentFocusedWindowId
// ============================================================================

test("getMostRecentFocusedWindowId returns window with latest focused tab", () => {
  const windows = [
    { windowId: 1, focused: false, tabs: [{ lastFocusedAt: "1000" }], groups: [] },
    { windowId: 2, focused: false, tabs: [{ lastFocusedAt: "2000" }], groups: [] },
    { windowId: 3, focused: false, tabs: [{ lastFocusedAt: "1500" }], groups: [] },
  ];
  assert.equal(getMostRecentFocusedWindowId(windows), 2);
});

test("getMostRecentFocusedWindowId returns null when no tabs have focus time", () => {
  const windows = [
    { windowId: 1, focused: false, tabs: [{ lastFocusedAt: undefined }], groups: [] },
  ];
  assert.equal(getMostRecentFocusedWindowId(windows), null);
});

test("getMostRecentFocusedWindowId returns null for empty windows", () => {
  assert.equal(getMostRecentFocusedWindowId([]), null);
});

// ============================================================================
// resolveOpenWindow — window resolution logic used by openTabs
// ============================================================================

test("resolveOpenWindow returns focused window by default", () => {
  const snapshot = {
    windows: [
      { windowId: 1, focused: false, tabs: [], groups: [] },
      { windowId: 2, focused: true, tabs: [], groups: [] },
    ],
  };
  const result = resolveOpenWindow(snapshot, {});
  assert.deepEqual(result, { windowId: 2 });
});

test("resolveOpenWindow returns error when no windows", () => {
  const result = resolveOpenWindow({ windows: [] }, {});
  assert.ok((result as { error: { message: string } }).error);
  assert.match((result as { error: { message: string } }).error.message, /No windows available/);
});

test("resolveOpenWindow resolves by windowId", () => {
  const snapshot = {
    windows: [
      { windowId: 1, focused: false, tabs: [], groups: [] },
      { windowId: 2, focused: false, tabs: [], groups: [] },
    ],
  };
  const result = resolveOpenWindow(snapshot, { windowId: 2 });
  assert.deepEqual(result, { windowId: 2 });
});

test("resolveOpenWindow resolves active window", () => {
  const snapshot = {
    windows: [
      { windowId: 1, focused: true, tabs: [], groups: [] },
      { windowId: 2, focused: false, tabs: [], groups: [] },
    ],
  };
  const result = resolveOpenWindow(snapshot, { windowId: "active" });
  assert.deepEqual(result, { windowId: 1 });
});

test("resolveOpenWindow resolves by windowGroupTitle", () => {
  const snapshot = {
    windows: [
      { windowId: 1, focused: false, tabs: [], groups: [{ groupId: 10, title: "Work" }] },
      { windowId: 2, focused: false, tabs: [], groups: [{ groupId: 20, title: "Home" }] },
    ],
  };
  const result = resolveOpenWindow(snapshot, { windowGroupTitle: "Work" });
  assert.deepEqual(result, { windowId: 1 });
});

test("resolveOpenWindow errors when windowGroupTitle matches multiple windows", () => {
  const snapshot = {
    windows: [
      { windowId: 1, focused: false, tabs: [], groups: [{ groupId: 10, title: "Work" }] },
      { windowId: 2, focused: false, tabs: [], groups: [{ groupId: 20, title: "Work" }] },
    ],
  };
  const result = resolveOpenWindow(snapshot, { windowGroupTitle: "Work" });
  assert.ok((result as { error: { message: string } }).error);
  assert.match((result as { error: { message: string } }).error.message, /Multiple windows/);
});

test("resolveOpenWindow resolves by windowUrl", () => {
  const snapshot = {
    windows: [
      { windowId: 1, focused: false, tabs: [{ url: "https://github.com/ekroon/tabctl" }], groups: [] },
      { windowId: 2, focused: false, tabs: [{ url: "https://example.com" }], groups: [] },
    ],
  };
  const result = resolveOpenWindow(snapshot, { windowUrl: "github.com" });
  assert.deepEqual(result, { windowId: 1 });
});

test("resolveOpenWindow resolves single window without selectors", () => {
  const snapshot = {
    windows: [
      { windowId: 1, focused: false, tabs: [], groups: [] },
    ],
  };
  const result = resolveOpenWindow(snapshot, {});
  assert.deepEqual(result, { windowId: 1 });
});

test("resolveOpenWindow resolves by windowTabId", () => {
  const snapshot = {
    windows: [
      { windowId: 1, focused: false, tabs: [{ tabId: 10 }], groups: [] },
      { windowId: 2, focused: false, tabs: [{ tabId: 20 }], groups: [] },
    ],
  };
  const result = resolveOpenWindow(snapshot, { windowTabId: 20 });
  assert.deepEqual(result, { windowId: 2 });
});

// ============================================================================
// Duplicate URL detection scenarios — validates the dedup logic
// ============================================================================

test("normalizeUrl matches URLs that differ only in tracking params", () => {
  const urls = [
    "https://github.com/ekroon/tabctl",
    "https://github.com/ekroon/tabctl?utm_source=docs",
    "https://github.com/ekroon/tabctl?fbclid=abc123",
    "https://github.com/ekroon/tabctl#readme",
    "https://github.com/ekroon/tabctl?utm_medium=email&utm_source=newsletter#top",
  ];
  const normalized = urls.map((u) => normalizeUrl(u));
  const unique = new Set(normalized);
  assert.equal(unique.size, 1, "All URLs should normalize to the same value");
});

test("normalizeUrl distinguishes different paths", () => {
  const a = normalizeUrl("https://github.com/ekroon/tabctl");
  const b = normalizeUrl("https://github.com/ekroon/tabctl/pulls");
  assert.notEqual(a, b);
});

test("normalizeUrl distinguishes different query params", () => {
  const a = normalizeUrl("https://example.com/search?q=hello");
  const b = normalizeUrl("https://example.com/search?q=world");
  assert.notEqual(a, b);
});

test("duplicate detection via Set matches group reuse logic", () => {
  // Simulates the dedup logic in openTabs: build a Set from existing
  // group tab URLs, then check new URLs against it.
  const existingTabs = [
    { url: "https://github.com/ekroon/tabctl" },
    { url: "https://github.com/ekroon/tabctl/pulls" },
  ];
  const existingUrls = new Set<string>();
  for (const tab of existingTabs) {
    const normalized = normalizeUrl(tab.url);
    if (normalized) existingUrls.add(normalized);
  }

  const newUrls = [
    "https://github.com/ekroon/tabctl",            // duplicate
    "https://github.com/ekroon/tabctl?utm_source=x", // duplicate (after normalization)
    "https://www.npmjs.com/package/tabctl",          // new
  ];

  const created: string[] = [];
  const skipped: string[] = [];
  for (const url of newUrls) {
    const normalized = normalizeUrl(url);
    if (normalized && existingUrls.has(normalized)) {
      skipped.push(url);
    } else {
      created.push(url);
    }
  }

  assert.equal(skipped.length, 2);
  assert.equal(created.length, 1);
  assert.equal(created[0], "https://www.npmjs.com/package/tabctl");
});

// ============================================================================
// openTabs — full function tests with lightweight chrome stubs
// ============================================================================

// Minimal chrome stub that records calls and returns predictable results.
function installChromeStub() {
  let tabIdCounter = 100;
  let groupIdCounter = 200;
  let windowIdCounter = 300;
  const calls: Array<{ method: string; args: unknown[] }> = [];

  const chromeStub = {
    windows: {
      create: async (opts: Record<string, unknown>) => {
        calls.push({ method: "windows.create", args: [opts] });
        const windowId = windowIdCounter++;
        return {
          id: windowId,
          tabs: [{ id: tabIdCounter++, windowId, index: 0, url: "chrome://newtab", title: "New Tab" }],
        };
      },
    },
    tabs: {
      create: async (opts: Record<string, unknown>) => {
        calls.push({ method: "tabs.create", args: [opts] });
        const id = tabIdCounter++;
        return {
          id,
          windowId: opts.windowId ?? 1,
          index: (opts.index as number) ?? 0,
          url: opts.url ?? "",
          title: "",
        };
      },
      group: async (opts: Record<string, unknown>) => {
        calls.push({ method: "tabs.group", args: [opts] });
        if (opts.groupId != null) {
          return opts.groupId as number;
        }
        return groupIdCounter++;
      },
      remove: async (tabId: number) => {
        calls.push({ method: "tabs.remove", args: [tabId] });
      },
      query: async () => [],
    },
    tabGroups: {
      update: async (groupId: number, props: Record<string, unknown>) => {
        calls.push({ method: "tabGroups.update", args: [groupId, props] });
        return { id: groupId, ...props };
      },
    },
  };

  (globalThis as Record<string, unknown>).chrome = chromeStub;
  return { calls, getTabIdCounter: () => tabIdCounter, getGroupIdCounter: () => groupIdCounter };
}

function makeDeps(snapshot: Record<string, unknown>) {
  const logs: unknown[][] = [];
  return {
    getTabSnapshot: async () => snapshot as { generatedAt: number; windows: Array<Record<string, unknown>> },
    log: (...args: unknown[]) => { logs.push(args); },
    _logs: logs,
  };
}

function makeSnapshot(windows: Array<{
  windowId: number;
  focused?: boolean;
  tabs?: Array<Record<string, unknown>>;
  groups?: Array<Record<string, unknown>>;
}>) {
  return {
    generatedAt: Date.now(),
    windows: windows.map((w) => ({
      windowId: w.windowId,
      focused: w.focused ?? false,
      tabs: w.tabs ?? [],
      groups: w.groups ?? [],
    })),
  };
}

test("openTabs creates tabs in focused window", async () => {
  const stub = installChromeStub();
  const snapshot = makeSnapshot([
    { windowId: 1, focused: true, tabs: [{ tabId: 1, index: 0 }] },
  ]);
  const deps = makeDeps(snapshot);

  const result = await openTabs({
    urls: ["https://example.com", "https://example.org"],
  }, deps);

  assert.equal(result.windowId, 1);
  assert.equal(result.created.length, 2);
  assert.equal(result.skipped.length, 0);
  assert.equal(result.summary.createdTabs, 2);
  assert.equal(result.summary.skippedUrls, 0);
  assert.equal(result.summary.grouped, false);
  assert.equal(result.groupId, null);
});

test("openTabs creates group for new tabs", async () => {
  const stub = installChromeStub();
  const snapshot = makeSnapshot([
    { windowId: 1, focused: true, tabs: [{ tabId: 1, index: 0 }] },
  ]);
  const deps = makeDeps(snapshot);

  const result = await openTabs({
    urls: ["https://example.com"],
    groupTitle: "Test",
    color: "blue",
  }, deps);

  assert.equal(result.groupTitle, "Test");
  assert.ok(result.groupId != null);
  assert.equal(result.summary.grouped, true);

  // Verify chrome.tabs.group was called to create a new group
  const groupCall = stub.calls.find((c) => c.method === "tabs.group");
  assert.ok(groupCall);
  const groupArgs = groupCall!.args[0] as Record<string, unknown>;
  assert.ok(groupArgs.createProperties);
  assert.equal((groupArgs.createProperties as Record<string, unknown>).windowId, 1);

  // Verify chrome.tabGroups.update was called with title and color
  const updateCall = stub.calls.find((c) => c.method === "tabGroups.update");
  assert.ok(updateCall);
  const updateArgs = updateCall!.args[1] as Record<string, unknown>;
  assert.equal(updateArgs.title, "Test");
  assert.equal(updateArgs.color, "blue");
});

test("openTabs reuses existing group by default", async () => {
  const stub = installChromeStub();
  const snapshot = makeSnapshot([
    {
      windowId: 1,
      focused: true,
      tabs: [
        { tabId: 1, index: 0, groupId: 50, url: "https://existing.com" },
      ],
      groups: [
        { groupId: 50, title: "MyGroup" },
      ],
    },
  ]);
  const deps = makeDeps(snapshot);

  const result = await openTabs({
    urls: ["https://newsite.com"],
    groupTitle: "MyGroup",
  }, deps);

  assert.equal(result.groupId, 50);
  assert.equal(result.summary.grouped, true);
  assert.equal(result.created.length, 1);

  // Verify chrome.tabs.group was called with existing groupId (reuse)
  const groupCall = stub.calls.find((c) => c.method === "tabs.group");
  assert.ok(groupCall);
  const groupArgs = groupCall!.args[0] as Record<string, unknown>;
  assert.equal(groupArgs.groupId, 50, "Should reuse existing groupId");
  assert.equal(groupArgs.createProperties, undefined, "Should not create new group");
});

test("openTabs forces new group with newGroup flag", async () => {
  const stub = installChromeStub();
  const snapshot = makeSnapshot([
    {
      windowId: 1,
      focused: true,
      tabs: [
        { tabId: 1, index: 0, groupId: 50, url: "https://existing.com" },
      ],
      groups: [
        { groupId: 50, title: "MyGroup" },
      ],
    },
  ]);
  const deps = makeDeps(snapshot);

  const result = await openTabs({
    urls: ["https://newsite.com"],
    groupTitle: "MyGroup",
    newGroup: true,
  }, deps);

  // Should create a new group, not reuse groupId 50
  assert.ok(result.groupId != null);
  assert.notEqual(result.groupId, 50, "Should not reuse existing group");

  const groupCall = stub.calls.find((c) => c.method === "tabs.group");
  assert.ok(groupCall);
  const groupArgs = groupCall!.args[0] as Record<string, unknown>;
  assert.ok(groupArgs.createProperties, "Should create a new group");
});

test("openTabs skips duplicate URLs in existing group", async () => {
  installChromeStub();
  const snapshot = makeSnapshot([
    {
      windowId: 1,
      focused: true,
      tabs: [
        { tabId: 1, index: 0, groupId: 50, url: "https://github.com/ekroon/tabctl" },
        { tabId: 2, index: 1, groupId: 50, url: "https://github.com/ekroon/tabctl/pulls" },
      ],
      groups: [
        { groupId: 50, title: "tabctl" },
      ],
    },
  ]);
  const deps = makeDeps(snapshot);

  const result = await openTabs({
    urls: [
      "https://github.com/ekroon/tabctl",       // duplicate
      "https://github.com/ekroon/tabctl/pulls",  // duplicate
      "https://www.npmjs.com/package/tabctl",     // new
    ],
    groupTitle: "tabctl",
  }, deps);

  assert.equal(result.created.length, 1);
  assert.equal(result.skipped.length, 2);
  assert.equal(result.skipped[0].reason, "duplicate");
  assert.equal(result.skipped[1].reason, "duplicate");
  assert.equal(result.summary.createdTabs, 1);
  assert.equal(result.summary.skippedUrls, 2);
  assert.equal(result.groupId, 50);
});

test("openTabs allows duplicates with allowDuplicates flag", async () => {
  installChromeStub();
  const snapshot = makeSnapshot([
    {
      windowId: 1,
      focused: true,
      tabs: [
        { tabId: 1, index: 0, groupId: 50, url: "https://github.com/ekroon/tabctl" },
      ],
      groups: [
        { groupId: 50, title: "tabctl" },
      ],
    },
  ]);
  const deps = makeDeps(snapshot);

  const result = await openTabs({
    urls: ["https://github.com/ekroon/tabctl"],
    groupTitle: "tabctl",
    allowDuplicates: true,
  }, deps);

  assert.equal(result.created.length, 1, "Duplicate should be opened when allowDuplicates is true");
  assert.equal(result.skipped.length, 0);
});

test("openTabs skips duplicates with normalized tracking params", async () => {
  installChromeStub();
  const snapshot = makeSnapshot([
    {
      windowId: 1,
      focused: true,
      tabs: [
        { tabId: 1, index: 0, groupId: 50, url: "https://example.com/page" },
      ],
      groups: [
        { groupId: 50, title: "Work" },
      ],
    },
  ]);
  const deps = makeDeps(snapshot);

  const result = await openTabs({
    urls: [
      "https://example.com/page?utm_source=twitter",  // same after normalization
      "https://example.com/page#section",              // same after normalization
    ],
    groupTitle: "Work",
  }, deps);

  assert.equal(result.created.length, 0);
  assert.equal(result.skipped.length, 2);
  assert.equal(result.skipped[0].reason, "duplicate");
  assert.equal(result.skipped[1].reason, "duplicate");
});

test("openTabs reports existing group when all URLs are duplicates", async () => {
  installChromeStub();
  const snapshot = makeSnapshot([
    {
      windowId: 1,
      focused: true,
      tabs: [
        { tabId: 1, index: 0, groupId: 50, url: "https://example.com" },
      ],
      groups: [
        { groupId: 50, title: "MyGroup" },
      ],
    },
  ]);
  const deps = makeDeps(snapshot);

  const result = await openTabs({
    urls: ["https://example.com"],
    groupTitle: "MyGroup",
  }, deps);

  assert.equal(result.created.length, 0);
  assert.equal(result.skipped.length, 1);
  assert.equal(result.groupId, 50, "Should still report existing groupId");
  assert.equal(result.summary.grouped, true);
});

test("openTabs inserts after last tab in existing group", async () => {
  const stub = installChromeStub();
  const snapshot = makeSnapshot([
    {
      windowId: 1,
      focused: true,
      tabs: [
        { tabId: 1, index: 0, groupId: 50, url: "https://a.com" },
        { tabId: 2, index: 1, groupId: 50, url: "https://b.com" },
        { tabId: 3, index: 2, groupId: -1, url: "https://other.com" },
      ],
      groups: [
        { groupId: 50, title: "Work" },
      ],
    },
  ]);
  const deps = makeDeps(snapshot);

  const result = await openTabs({
    urls: ["https://c.com"],
    groupTitle: "Work",
  }, deps);

  assert.equal(result.insertIndex, 2, "Should insert after index 1 (last group tab)");
  // Verify chrome.tabs.create was called with index 2
  const createCall = stub.calls.find((c) => c.method === "tabs.create");
  assert.ok(createCall);
  const createArgs = createCall!.args[0] as Record<string, unknown>;
  assert.equal(createArgs.index, 2);
});

test("openTabs auto-resolves window by group title", async () => {
  installChromeStub();
  const snapshot = makeSnapshot([
    {
      windowId: 1,
      focused: false,
      tabs: [{ tabId: 1, index: 0 }],
      groups: [],
    },
    {
      windowId: 2,
      focused: false,
      tabs: [
        { tabId: 2, index: 0, groupId: 50, url: "https://a.com" },
      ],
      groups: [
        { groupId: 50, title: "tabctl" },
      ],
    },
  ]);
  const deps = makeDeps(snapshot);

  const result = await openTabs({
    urls: ["https://b.com"],
    groupTitle: "tabctl",
  }, deps);

  assert.equal(result.windowId, 2, "Should auto-resolve to window containing the group");
});

test("openTabs in new window creates window and tabs", async () => {
  const stub = installChromeStub();
  const deps = makeDeps(makeSnapshot([]));

  const result = await openTabs({
    urls: ["https://example.com", "https://example.org"],
    newWindow: true,
  }, deps);

  assert.ok(result.windowId != null);
  assert.equal(result.created.length, 2);
  assert.equal(result.summary.createdTabs, 2);

  // Verify window was created
  const windowCreate = stub.calls.find((c) => c.method === "windows.create");
  assert.ok(windowCreate);

  // Verify seed tab was removed
  const removeCall = stub.calls.find((c) => c.method === "tabs.remove");
  assert.ok(removeCall);
});

test("openTabs in new window with group creates group", async () => {
  const stub = installChromeStub();
  const deps = makeDeps(makeSnapshot([]));

  const result = await openTabs({
    urls: ["https://example.com"],
    newWindow: true,
    groupTitle: "NewGroup",
    color: "red",
  }, deps);

  assert.ok(result.groupId != null);
  assert.equal(result.groupTitle, "NewGroup");
  assert.equal(result.summary.grouped, true);

  const updateCall = stub.calls.find((c) => c.method === "tabGroups.update");
  assert.ok(updateCall);
  const updateArgs = updateCall!.args[1] as Record<string, unknown>;
  assert.equal(updateArgs.title, "NewGroup");
  assert.equal(updateArgs.color, "red");
});

test("openTabs throws when no URLs and no newWindow", async () => {
  installChromeStub();
  const deps = makeDeps(makeSnapshot([]));

  await assert.rejects(
    () => openTabs({ urls: [] }, deps),
    { message: "No URLs provided" },
  );
});

test("openTabs throws when before and after tab both specified", async () => {
  installChromeStub();
  const deps = makeDeps(makeSnapshot([]));

  await assert.rejects(
    () => openTabs({ urls: ["https://example.com"], beforeTabId: 1, afterTabId: 2 }, deps),
    { message: "Only one target position is allowed" },
  );
});

test("openTabs throws when new window combined with before-tab", async () => {
  installChromeStub();
  const deps = makeDeps(makeSnapshot([]));

  await assert.rejects(
    () => openTabs({ urls: ["https://example.com"], newWindow: true, beforeTabId: 1 }, deps),
    { message: "Cannot use --before/--after with --new-window" },
  );
});

test("openTabs throws when new window combined with window selectors", async () => {
  installChromeStub();
  const deps = makeDeps(makeSnapshot([]));

  await assert.rejects(
    () => openTabs({ urls: ["https://example.com"], newWindow: true, windowId: 1 }, deps),
    { message: "Cannot combine --new-window with window selectors" },
  );
});

test("openTabs without group does not skip duplicates", async () => {
  installChromeStub();
  const snapshot = makeSnapshot([
    {
      windowId: 1,
      focused: true,
      tabs: [
        { tabId: 1, index: 0, groupId: -1, url: "https://example.com" },
      ],
    },
  ]);
  const deps = makeDeps(snapshot);

  // Opening same URL but without a group should NOT deduplicate
  const result = await openTabs({
    urls: ["https://example.com"],
  }, deps);

  assert.equal(result.created.length, 1, "Without group, duplicates are not skipped");
  assert.equal(result.skipped.length, 0);
});

test("openTabs with afterGroupTitle inserts after target group", async () => {
  const stub = installChromeStub();
  const snapshot = makeSnapshot([
    {
      windowId: 1,
      focused: true,
      tabs: [
        { tabId: 1, index: 0, groupId: 50, url: "https://a.com" },
        { tabId: 2, index: 1, groupId: 50, url: "https://b.com" },
        { tabId: 3, index: 2, groupId: -1, url: "https://other.com" },
      ],
      groups: [
        { groupId: 50, title: "Work" },
      ],
    },
  ]);
  const deps = makeDeps(snapshot);

  const result = await openTabs({
    urls: ["https://new.com"],
    afterGroupTitle: "Work",
  }, deps);

  assert.equal(result.insertIndex, 2);
  assert.equal(result.afterGroupTitle, "Work");
});

test("openTabs with beforeTabId inserts at anchor index", async () => {
  const stub = installChromeStub();
  const snapshot = makeSnapshot([
    {
      windowId: 1,
      focused: true,
      tabs: [
        { tabId: 10, index: 0, url: "https://a.com" },
        { tabId: 20, index: 1, url: "https://b.com" },
        { tabId: 30, index: 2, url: "https://c.com" },
      ],
    },
  ]);
  const deps = makeDeps(snapshot);

  const result = await openTabs({
    urls: ["https://inserted.com"],
    beforeTabId: 20,
  }, deps);

  assert.equal(result.insertIndex, 1, "Should insert at index of the anchor tab");
});

test("openTabs with afterTabId inserts after anchor index", async () => {
  const stub = installChromeStub();
  const snapshot = makeSnapshot([
    {
      windowId: 1,
      focused: true,
      tabs: [
        { tabId: 10, index: 0, url: "https://a.com" },
        { tabId: 20, index: 1, url: "https://b.com" },
        { tabId: 30, index: 2, url: "https://c.com" },
      ],
    },
  ]);
  const deps = makeDeps(snapshot);

  const result = await openTabs({
    urls: ["https://inserted.com"],
    afterTabId: 20,
  }, deps);

  assert.equal(result.insertIndex, 2, "Should insert after the anchor tab");
});

test("openTabs resolves window from anchor tab", async () => {
  installChromeStub();
  const snapshot = makeSnapshot([
    {
      windowId: 1,
      focused: false,
      tabs: [{ tabId: 10, index: 0 }],
    },
    {
      windowId: 2,
      focused: false,
      tabs: [{ tabId: 20, index: 0 }],
    },
  ]);
  const deps = makeDeps(snapshot);

  const result = await openTabs({
    urls: ["https://example.com"],
    afterTabId: 20,
  }, deps);

  assert.equal(result.windowId, 2, "Should resolve window from anchor tab");
});

test("openTabs creates tabs without group when no groupTitle", async () => {
  const stub = installChromeStub();
  const snapshot = makeSnapshot([
    { windowId: 1, focused: true, tabs: [{ tabId: 1, index: 0 }] },
  ]);
  const deps = makeDeps(snapshot);

  const result = await openTabs({
    urls: ["https://example.com"],
  }, deps);

  assert.equal(result.groupId, null);
  assert.equal(result.groupTitle, null);
  assert.equal(result.summary.grouped, false);
  // No tabs.group call should have been made
  const groupCalls = stub.calls.filter((c) => c.method === "tabs.group");
  assert.equal(groupCalls.length, 0);
});

test("openTabs in new window without URLs returns seed tab", async () => {
  installChromeStub();
  const deps = makeDeps(makeSnapshot([]));

  const result = await openTabs({
    newWindow: true,
  }, deps);

  assert.ok(result.windowId != null);
  assert.equal(result.created.length, 1, "Should include seed tab when no URLs");
});

test("openTabs increments insertion index for multiple tabs", async () => {
  const stub = installChromeStub();
  const snapshot = makeSnapshot([
    {
      windowId: 1,
      focused: true,
      tabs: [
        { tabId: 1, index: 0, groupId: 50, url: "https://a.com" },
      ],
      groups: [
        { groupId: 50, title: "Work" },
      ],
    },
  ]);
  const deps = makeDeps(snapshot);

  const result = await openTabs({
    urls: ["https://b.com", "https://c.com", "https://d.com"],
    groupTitle: "Work",
  }, deps);

  assert.equal(result.created.length, 3);
  // Verify tabs were created with incrementing indices
  const createCalls = stub.calls.filter((c) => c.method === "tabs.create");
  assert.equal(createCalls.length, 3);
  const indices = createCalls.map((c) => (c.args[0] as Record<string, unknown>).index);
  assert.equal(indices[0], 1); // After existing group tab at index 0
  assert.equal(indices[1], 2);
  assert.equal(indices[2], 3);
});
