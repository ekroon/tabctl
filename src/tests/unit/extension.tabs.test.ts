import { describe, it, beforeEach } from "node:test";
import * as assert from "node:assert/strict";
import { openTabs, normalizeUrl } from "../../extension/lib/tabs";
import { groupGather, resolveGroupByTitle } from "../../extension/lib/groups";

// ---------------------------------------------------------------------------
// Chrome API stub
// ---------------------------------------------------------------------------

type Call = { api: string; args: unknown[] };

function installChromeStub() {
  const calls: Call[] = [];
  let nextTabId = 1000;
  let nextGroupId = 5000;

  const stub = {
    tabs: {
      create: async (opts: Record<string, unknown>) => {
        const tab = {
          id: nextTabId++,
          windowId: opts.windowId ?? 1,
          index: opts.index ?? 0,
          url: opts.url,
          title: "",
          groupId: -1,
        };
        calls.push({ api: "tabs.create", args: [opts] });
        return tab;
      },
      group: async (opts: Record<string, unknown>) => {
        calls.push({ api: "tabs.group", args: [opts] });
        if (opts.groupId != null) return opts.groupId;
        return nextGroupId++;
      },
      move: async (tabIds: number | number[], opts: Record<string, unknown>) => {
        calls.push({ api: "tabs.move", args: [tabIds, opts] });
        const ids = Array.isArray(tabIds) ? tabIds : [tabIds];
        return ids.map((id, i) => ({ id, index: (opts.index as number) + i, windowId: opts.windowId }));
      },
      query: async (_opts: Record<string, unknown>) => {
        calls.push({ api: "tabs.query", args: [_opts] });
        // Default: return empty, overridden per-test via queryResult
        return stubState.queryResult;
      },
      remove: async (id: number) => {
        calls.push({ api: "tabs.remove", args: [id] });
      },
      ungroup: async (ids: number[]) => {
        calls.push({ api: "tabs.ungroup", args: [ids] });
      },
    },
    tabGroups: {
      update: async (id: number, props: Record<string, unknown>) => {
        calls.push({ api: "tabGroups.update", args: [id, props] });
        return { id, windowId: 1, ...props };
      },
    },
    windows: {
      create: async (opts: Record<string, unknown>) => {
        calls.push({ api: "windows.create", args: [opts] });
        return { id: 99, tabs: [{ id: 999, index: 0 }] };
      },
    },
  };

  const stubState = {
    queryResult: [] as Array<Record<string, unknown>>,
    nextTabId: () => nextTabId,
    nextGroupId: () => nextGroupId,
  };

  (globalThis as any).chrome = stub;
  return { calls, stub, stubState };
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeDeps(snapshot: { windows: Array<Record<string, unknown>> }) {
  return {
    getTabSnapshot: async () => ({ generatedAt: Date.now(), ...snapshot }),
    buildWindowLabels: () => new Map<number, string>(),
    resolveWindowIdFromParams: (_snap: unknown, id: unknown) => Number(id),
    log: () => {},
  };
}

function buildWindowLabels() {
  return new Map<number, string>();
}

// ---------------------------------------------------------------------------
// openTabs — Group reuse
// ---------------------------------------------------------------------------

describe("openTabs — group reuse", () => {
  let calls: Call[];
  let stubState: ReturnType<typeof installChromeStub>["stubState"];

  beforeEach(() => {
    const s = installChromeStub();
    calls = s.calls;
    stubState = s.stubState;
  });

  it("reuses existing group when --group matches", async () => {
    const snapshot = {
      windows: [
        {
          windowId: 1,
          focused: true,
          tabs: [
            { tabId: 1, windowId: 1, index: 0, groupId: 42, url: "https://existing.com", groupTitle: "Work" },
          ],
          groups: [{ groupId: 42, title: "Work", color: "blue" }],
        },
      ],
    };
    // After group assignment, tabs.query returns all tabs ungrouped-free
    stubState.queryResult = [
      { id: 1, index: 0, groupId: 42, windowId: 1 },
      { id: 1000, index: 1, groupId: 42, windowId: 1 },
    ];

    const result = await openTabs(
      { urls: ["https://new.com"], groupTitle: "Work" },
      makeDeps(snapshot),
    );

    assert.equal(result.groupId, 42);
    const groupCall = calls.find((c) => c.api === "tabs.group");
    assert.ok(groupCall, "tabs.group should be called");
    const groupArgs = groupCall!.args[0] as Record<string, unknown>;
    assert.equal(groupArgs.groupId, 42, "should reuse existing groupId");
    assert.equal(groupArgs.createProperties, undefined, "should NOT use createProperties");
  });

  it("inserts new tabs after last tab in existing group", async () => {
    const snapshot = {
      windows: [
        {
          windowId: 1,
          focused: true,
          tabs: [
            { tabId: 1, windowId: 1, index: 0, groupId: 42, url: "https://a.com", groupTitle: "Work" },
            { tabId: 2, windowId: 1, index: 1, groupId: 42, url: "https://b.com", groupTitle: "Work" },
            { tabId: 3, windowId: 1, index: 2, groupId: -1, url: "https://c.com" },
          ],
          groups: [{ groupId: 42, title: "Work" }],
        },
      ],
    };
    stubState.queryResult = [
      { id: 1, index: 0, groupId: 42, windowId: 1 },
      { id: 2, index: 1, groupId: 42, windowId: 1 },
      { id: 1000, index: 2, groupId: 42, windowId: 1 },
      { id: 3, index: 3, groupId: -1, windowId: 1 },
    ];

    const result = await openTabs(
      { urls: ["https://new.com"], groupTitle: "Work" },
      makeDeps(snapshot),
    );

    assert.equal(result.insertIndex, 2, "should insert at index 2 (after last group tab at index 1)");
    const createCall = calls.find((c) => c.api === "tabs.create");
    assert.ok(createCall);
    assert.equal((createCall!.args[0] as Record<string, unknown>).index, 2);
  });

  it("auto-resolves window when group exists in different window", async () => {
    const snapshot = {
      windows: [
        {
          windowId: 1,
          focused: true,
          tabs: [{ tabId: 1, windowId: 1, index: 0, groupId: -1, url: "https://other.com" }],
          groups: [],
        },
        {
          windowId: 2,
          focused: false,
          tabs: [{ tabId: 2, windowId: 2, index: 0, groupId: 50, url: "https://a.com", groupTitle: "Research" }],
          groups: [{ groupId: 50, title: "Research" }],
        },
      ],
    };
    stubState.queryResult = [
      { id: 2, index: 0, groupId: 50, windowId: 2 },
      { id: 1000, index: 1, groupId: 50, windowId: 2 },
    ];

    const result = await openTabs(
      { urls: ["https://new.com"], groupTitle: "Research" },
      makeDeps(snapshot),
    );

    assert.equal(result.windowId, 2, "should auto-resolve to window 2");
    assert.equal(result.groupId, 50, "should reuse existing group");
  });

  it("throws ambiguous error when 2+ groups named X exist", async () => {
    const snapshot = {
      windows: [
        {
          windowId: 1,
          focused: true,
          tabs: [
            { tabId: 1, windowId: 1, index: 0, groupId: 42, url: "https://a.com", groupTitle: "Work" },
            { tabId: 2, windowId: 1, index: 1, groupId: 43, url: "https://b.com", groupTitle: "Work" },
          ],
          groups: [
            { groupId: 42, title: "Work" },
            { groupId: 43, title: "Work" },
          ],
        },
      ],
    };

    await assert.rejects(
      () => openTabs({ urls: ["https://new.com"], groupTitle: "Work" }, makeDeps(snapshot)),
      (err: Error) => {
        assert.ok(err.message.includes("Ambiguous group title"));
        assert.ok(err.message.includes("group-gather"));
        assert.ok(err.message.includes("--group-id"));
        return true;
      },
    );
  });
});

// ---------------------------------------------------------------------------
// openTabs — Duplicate URL skipping
// ---------------------------------------------------------------------------

describe("openTabs — duplicate URL skipping", () => {
  let calls: Call[];
  let stubState: ReturnType<typeof installChromeStub>["stubState"];

  beforeEach(() => {
    const s = installChromeStub();
    calls = s.calls;
    stubState = s.stubState;
  });

  it("skips URLs already present in the group", async () => {
    const snapshot = {
      windows: [
        {
          windowId: 1,
          focused: true,
          tabs: [
            { tabId: 1, windowId: 1, index: 0, groupId: 42, url: "https://example.com/", groupTitle: "Work" },
          ],
          groups: [{ groupId: 42, title: "Work" }],
        },
      ],
    };
    stubState.queryResult = [{ id: 1, index: 0, groupId: 42, windowId: 1 }];

    const result = await openTabs(
      { urls: ["https://example.com/", "https://new.com"], groupTitle: "Work" },
      makeDeps(snapshot),
    );

    assert.equal(result.skipped.length, 1);
    assert.equal((result.skipped[0] as Record<string, unknown>).url, "https://example.com/");
    assert.equal((result.skipped[0] as Record<string, unknown>).reason, "duplicate");
    assert.equal(result.created.length, 1);
  });

  it("uses normalizeUrl for comparison (trailing slash, tracking params)", async () => {
    const snapshot = {
      windows: [
        {
          windowId: 1,
          focused: true,
          tabs: [
            { tabId: 1, windowId: 1, index: 0, groupId: 42, url: "https://example.com", groupTitle: "Work" },
          ],
          groups: [{ groupId: 42, title: "Work" }],
        },
      ],
    };
    stubState.queryResult = [{ id: 1, index: 0, groupId: 42, windowId: 1 }];

    // normalizeUrl strips trailing slash and utm params
    const result = await openTabs(
      { urls: ["https://example.com/?utm_source=test"], groupTitle: "Work" },
      makeDeps(snapshot),
    );

    assert.equal(result.skipped.length, 1);
    assert.equal((result.skipped[0] as Record<string, unknown>).reason, "duplicate");
  });

  it("--allow-duplicates disables skipping", async () => {
    const snapshot = {
      windows: [
        {
          windowId: 1,
          focused: true,
          tabs: [
            { tabId: 1, windowId: 1, index: 0, groupId: 42, url: "https://example.com", groupTitle: "Work" },
          ],
          groups: [{ groupId: 42, title: "Work" }],
        },
      ],
    };
    stubState.queryResult = [
      { id: 1, index: 0, groupId: 42, windowId: 1 },
      { id: 1000, index: 1, groupId: 42, windowId: 1 },
    ];

    const result = await openTabs(
      { urls: ["https://example.com"], groupTitle: "Work", allowDuplicates: true },
      makeDeps(snapshot),
    );

    assert.equal(result.skipped.length, 0);
    assert.equal(result.created.length, 1);
  });

  it("returns existing groupId when ALL urls are duplicates", async () => {
    const snapshot = {
      windows: [
        {
          windowId: 1,
          focused: true,
          tabs: [
            { tabId: 1, windowId: 1, index: 0, groupId: 42, url: "https://example.com", groupTitle: "Work" },
            { tabId: 2, windowId: 1, index: 1, groupId: 42, url: "https://other.com", groupTitle: "Work" },
          ],
          groups: [{ groupId: 42, title: "Work" }],
        },
      ],
    };
    stubState.queryResult = [
      { id: 1, index: 0, groupId: 42, windowId: 1 },
      { id: 2, index: 1, groupId: 42, windowId: 1 },
    ];

    const result = await openTabs(
      { urls: ["https://example.com", "https://other.com"], groupTitle: "Work" },
      makeDeps(snapshot),
    );

    assert.equal(result.created.length, 0);
    assert.equal(result.skipped.length, 2);
    assert.equal(result.groupId, 42, "should still report existing groupId");
  });

  it("does not dedup when no group exists (same URL open elsewhere)", async () => {
    const snapshot = {
      windows: [
        {
          windowId: 1,
          focused: true,
          tabs: [
            { tabId: 1, windowId: 1, index: 0, groupId: -1, url: "https://example.com" },
          ],
          groups: [],
        },
      ],
    };
    stubState.queryResult = [
      { id: 1, index: 0, groupId: -1, windowId: 1 },
      { id: 1000, index: 1, groupId: 5000, windowId: 1 },
    ];

    const result = await openTabs(
      { urls: ["https://example.com"], groupTitle: "New" },
      makeDeps(snapshot),
    );

    assert.equal(result.skipped.length, 0, "should not skip — no existing group named 'New'");
    assert.equal(result.created.length, 1);
  });
});

// ---------------------------------------------------------------------------
// openTabs — --new-group flag
// ---------------------------------------------------------------------------

describe("openTabs — --new-group flag", () => {
  let calls: Call[];
  let stubState: ReturnType<typeof installChromeStub>["stubState"];

  beforeEach(() => {
    const s = installChromeStub();
    calls = s.calls;
    stubState = s.stubState;
  });

  it("forces new group creation even when one with same name exists", async () => {
    const snapshot = {
      windows: [
        {
          windowId: 1,
          focused: true,
          tabs: [
            { tabId: 1, windowId: 1, index: 0, groupId: 42, url: "https://a.com", groupTitle: "Work" },
          ],
          groups: [{ groupId: 42, title: "Work" }],
        },
      ],
    };
    stubState.queryResult = [
      { id: 1, index: 0, groupId: 42, windowId: 1 },
      { id: 1000, index: 1, groupId: 5000, windowId: 1 },
    ];

    const result = await openTabs(
      { urls: ["https://new.com"], groupTitle: "Work", newGroup: true },
      makeDeps(snapshot),
    );

    const groupCall = calls.find((c) => c.api === "tabs.group");
    assert.ok(groupCall);
    const groupArgs = groupCall!.args[0] as Record<string, unknown>;
    assert.ok(groupArgs.createProperties, "should use createProperties for a new group");
    assert.equal(groupArgs.groupId, undefined, "should NOT pass existing groupId");
    assert.ok(result.groupId != null);
  });

  it("does not perform dedup with --new-group", async () => {
    const snapshot = {
      windows: [
        {
          windowId: 1,
          focused: true,
          tabs: [
            { tabId: 1, windowId: 1, index: 0, groupId: 42, url: "https://example.com", groupTitle: "Work" },
          ],
          groups: [{ groupId: 42, title: "Work" }],
        },
      ],
    };
    stubState.queryResult = [
      { id: 1, index: 0, groupId: 42, windowId: 1 },
      { id: 1000, index: 1, groupId: 5000, windowId: 1 },
    ];

    const result = await openTabs(
      { urls: ["https://example.com"], groupTitle: "Work", newGroup: true },
      makeDeps(snapshot),
    );

    assert.equal(result.skipped.length, 0, "should not skip any URLs");
    assert.equal(result.created.length, 1);
  });
});

// ---------------------------------------------------------------------------
// openTabs — groups-before-ungrouped reordering
// ---------------------------------------------------------------------------

describe("openTabs — groups-before-ungrouped reordering", () => {
  let calls: Call[];
  let stubState: ReturnType<typeof installChromeStub>["stubState"];

  beforeEach(() => {
    const s = installChromeStub();
    calls = s.calls;
    stubState = s.stubState;
  });

  it("moves grouped tabs before ungrouped tabs after group assignment", async () => {
    const snapshot = {
      windows: [
        {
          windowId: 1,
          focused: true,
          tabs: [
            { tabId: 1, windowId: 1, index: 0, groupId: -1, url: "https://ungrouped.com" },
          ],
          groups: [],
        },
      ],
    };
    // After creating a tab and grouping it, the query shows ungrouped first, then grouped
    stubState.queryResult = [
      { id: 1, index: 0, groupId: -1, windowId: 1 },
      { id: 1000, index: 1, groupId: 5000, windowId: 1 },
    ];

    await openTabs(
      { urls: ["https://new.com"], groupTitle: "MyGroup" },
      makeDeps(snapshot),
    );

    const moveCall = calls.find((c) => c.api === "tabs.move");
    assert.ok(moveCall, "tabs.move should be called to reorder");
    const movedIds = moveCall!.args[0] as number[];
    assert.deepEqual(movedIds, [1000], "should move the grouped tab");
    assert.equal((moveCall!.args[1] as Record<string, unknown>).index, 0, "should move to before ungrouped");
  });

  it("succeeds even if reordering fails (best-effort)", async () => {
    const snapshot = {
      windows: [
        {
          windowId: 1,
          focused: true,
          tabs: [
            { tabId: 1, windowId: 1, index: 0, groupId: -1, url: "https://ungrouped.com" },
          ],
          groups: [],
        },
      ],
    };
    // Make tabs.query throw to simulate failure
    (globalThis as any).chrome.tabs.query = async () => {
      throw new Error("query failed");
    };

    const result = await openTabs(
      { urls: ["https://new.com"], groupTitle: "MyGroup" },
      makeDeps(snapshot),
    );

    assert.ok(result, "should still return a result");
    assert.equal(result.created.length, 1);
  });
});

// ---------------------------------------------------------------------------
// groupGather
// ---------------------------------------------------------------------------

describe("groupGather", () => {
  let calls: Call[];

  beforeEach(() => {
    const s = installChromeStub();
    calls = s.calls;
  });

  it("merges 2 groups with same title into one", async () => {
    const snapshot = {
      windows: [
        {
          windowId: 1,
          focused: true,
          tabs: [
            { tabId: 1, windowId: 1, index: 1, groupId: 10, url: "https://a.com", groupTitle: "Work" },
            { tabId: 2, windowId: 1, index: 3, groupId: 20, url: "https://b.com", groupTitle: "Work" },
          ],
          groups: [
            { groupId: 10, title: "Work", color: "blue" },
            { groupId: 20, title: "Work", color: "red" },
          ],
        },
      ],
    };

    const result = await groupGather({}, makeDeps(snapshot));

    assert.equal(result.summary.mergedGroups, 1);
    assert.equal(result.summary.movedTabs, 1);
    const groupCall = calls.find((c) => c.api === "tabs.group");
    assert.ok(groupCall);
    const args = groupCall!.args[0] as Record<string, unknown>;
    assert.equal(args.groupId, 10, "should merge into the first (lowest-index) group");
    assert.deepEqual(args.tabIds, [2], "should move tab from second group");
  });

  it("merges 3+ groups with same title", async () => {
    const snapshot = {
      windows: [
        {
          windowId: 1,
          focused: true,
          tabs: [
            { tabId: 1, windowId: 1, index: 1, groupId: 10, url: "https://a.com", groupTitle: "Work" },
            { tabId: 2, windowId: 1, index: 3, groupId: 20, url: "https://b.com", groupTitle: "Work" },
            { tabId: 3, windowId: 1, index: 5, groupId: 30, url: "https://c.com", groupTitle: "Work" },
          ],
          groups: [
            { groupId: 10, title: "Work" },
            { groupId: 20, title: "Work" },
            { groupId: 30, title: "Work" },
          ],
        },
      ],
    };

    const result = await groupGather({}, makeDeps(snapshot));

    assert.equal(result.summary.mergedGroups, 2);
    assert.equal(result.summary.movedTabs, 2);
  });

  it("only merges within the same window", async () => {
    const snapshot = {
      windows: [
        {
          windowId: 1,
          focused: true,
          tabs: [
            { tabId: 1, windowId: 1, index: 1, groupId: 10, url: "https://a.com", groupTitle: "Work" },
          ],
          groups: [{ groupId: 10, title: "Work" }],
        },
        {
          windowId: 2,
          focused: false,
          tabs: [
            { tabId: 2, windowId: 2, index: 1, groupId: 20, url: "https://b.com", groupTitle: "Work" },
          ],
          groups: [{ groupId: 20, title: "Work" }],
        },
      ],
    };

    const result = await groupGather({}, makeDeps(snapshot));

    assert.equal(result.summary.mergedGroups, 0, "should not merge across windows");
    assert.equal(result.summary.movedTabs, 0);
  });

  it("--group filter targets specific group name", async () => {
    const snapshot = {
      windows: [
        {
          windowId: 1,
          focused: true,
          tabs: [
            { tabId: 1, windowId: 1, index: 1, groupId: 10, url: "https://a.com", groupTitle: "Work" },
            { tabId: 2, windowId: 1, index: 3, groupId: 20, url: "https://b.com", groupTitle: "Work" },
            { tabId: 3, windowId: 1, index: 5, groupId: 30, url: "https://c.com", groupTitle: "Play" },
            { tabId: 4, windowId: 1, index: 7, groupId: 40, url: "https://d.com", groupTitle: "Play" },
          ],
          groups: [
            { groupId: 10, title: "Work" },
            { groupId: 20, title: "Work" },
            { groupId: 30, title: "Play" },
            { groupId: 40, title: "Play" },
          ],
        },
      ],
    };

    const result = await groupGather({ groupTitle: "Work" }, makeDeps(snapshot));

    assert.equal(result.merged.length, 1, "should only merge Work groups");
    assert.equal((result.merged[0] as Record<string, unknown>).groupTitle, "Work");
    assert.equal(result.summary.mergedGroups, 1);
    assert.equal(result.summary.movedTabs, 1);
  });

  it("returns proper summary with mergedGroups and movedTabs counts", async () => {
    const snapshot = {
      windows: [
        {
          windowId: 1,
          focused: true,
          tabs: [
            { tabId: 1, windowId: 1, index: 1, groupId: 10, url: "https://a.com", groupTitle: "Work" },
            { tabId: 2, windowId: 1, index: 3, groupId: 20, url: "https://b.com", groupTitle: "Work" },
            { tabId: 3, windowId: 1, index: 4, groupId: 20, url: "https://c.com", groupTitle: "Work" },
          ],
          groups: [
            { groupId: 10, title: "Work" },
            { groupId: 20, title: "Work" },
          ],
        },
      ],
    };

    const result = await groupGather({}, makeDeps(snapshot));

    assert.equal(result.summary.mergedGroups, 1);
    assert.equal(result.summary.movedTabs, 2);
    assert.equal(result.merged.length, 1);
    assert.equal((result.merged[0] as Record<string, unknown>).primaryGroupId, 10);
    assert.equal((result.merged[0] as Record<string, unknown>).mergedGroupCount, 1);
    assert.equal((result.merged[0] as Record<string, unknown>).movedTabs, 2);
  });

  it("returns undo data with original tab assignments", async () => {
    const snapshot = {
      windows: [
        {
          windowId: 1,
          focused: true,
          tabs: [
            { tabId: 1, windowId: 1, index: 1, groupId: 10, url: "https://a.com", groupTitle: "Work", groupColor: "blue" },
            { tabId: 2, windowId: 1, index: 3, groupId: 20, url: "https://b.com", groupTitle: "Work", groupColor: "red" },
          ],
          groups: [
            { groupId: 10, title: "Work", color: "blue", collapsed: false },
            { groupId: 20, title: "Work", color: "red", collapsed: true },
          ],
        },
      ],
    };

    const result = await groupGather({}, makeDeps(snapshot));

    assert.equal(result.undo.action, "group-gather");
    const undoTabs = result.undo.tabs as Array<Record<string, unknown>>;
    assert.equal(undoTabs.length, 1, "should have undo data for moved tab");
    assert.equal(undoTabs[0].tabId, 2);
    assert.equal(undoTabs[0].groupId, 20, "should record original groupId");
    assert.equal(undoTabs[0].windowId, 1);
  });

  it("does not merge untitled groups (empty title)", async () => {
    const snapshot = {
      windows: [
        {
          windowId: 1,
          focused: true,
          tabs: [
            { tabId: 1, windowId: 1, index: 1, groupId: 10, url: "https://a.com" },
            { tabId: 2, windowId: 1, index: 3, groupId: 20, url: "https://b.com" },
          ],
          groups: [
            { groupId: 10, title: "" },
            { groupId: 20, title: "" },
          ],
        },
      ],
    };

    const result = await groupGather({}, makeDeps(snapshot));

    assert.equal(result.summary.mergedGroups, 0, "untitled groups should not be merged");
    assert.equal(result.summary.movedTabs, 0);
  });

  it("leaves single groups (no duplicates) alone", async () => {
    const snapshot = {
      windows: [
        {
          windowId: 1,
          focused: true,
          tabs: [
            { tabId: 1, windowId: 1, index: 1, groupId: 10, url: "https://a.com", groupTitle: "Work" },
            { tabId: 2, windowId: 1, index: 3, groupId: 20, url: "https://b.com", groupTitle: "Play" },
          ],
          groups: [
            { groupId: 10, title: "Work" },
            { groupId: 20, title: "Play" },
          ],
        },
      ],
    };

    const result = await groupGather({}, makeDeps(snapshot));

    assert.equal(result.summary.mergedGroups, 0);
    assert.equal(result.summary.movedTabs, 0);
    assert.equal(result.merged.length, 0);
  });

  it("selects the correct primary group when a tab is at index 0", async () => {
    const snapshot = {
      windows: [
        {
          windowId: 1,
          focused: true,
          tabs: [
            { tabId: 1, windowId: 1, index: 0, groupId: 10, url: "https://a.com", groupTitle: "Work" },
            { tabId: 2, windowId: 1, index: 3, groupId: 20, url: "https://b.com", groupTitle: "Work" },
          ],
          groups: [
            { groupId: 10, title: "Work", color: "blue" },
            { groupId: 20, title: "Work", color: "red" },
          ],
        },
      ],
    };

    const result = await groupGather({}, makeDeps(snapshot));

    assert.equal(result.summary.mergedGroups, 1);
    const groupCall = calls.find((c) => c.api === "tabs.group");
    assert.ok(groupCall);
    const args = groupCall!.args[0] as Record<string, unknown>;
    assert.equal(args.groupId, 10, "should merge into group at index 0, not treat 0 as falsy");
    assert.deepEqual(args.tabIds, [2]);
  });
});

// ---------------------------------------------------------------------------
// Ambiguous group title error (resolveGroupByTitle)
// ---------------------------------------------------------------------------

describe("resolveGroupByTitle — ambiguous error", () => {
  beforeEach(() => {
    installChromeStub();
  });

  it("errors when 2+ groups match with helpful message", () => {
    const snapshot = {
      windows: [
        {
          windowId: 1,
          focused: true,
          tabs: [
            { tabId: 1, windowId: 1, index: 0, groupId: 10, groupTitle: "Work" },
            { tabId: 2, windowId: 1, index: 1, groupId: 20, groupTitle: "Work" },
          ],
          groups: [
            { groupId: 10, title: "Work" },
            { groupId: 20, title: "Work" },
          ],
        },
      ],
    };

    const result = resolveGroupByTitle(snapshot, buildWindowLabels, "Work");

    assert.ok(result.error, "should return an error");
    const msg = result.error!.message as string;
    assert.ok(msg.includes("Ambiguous group title"), "should mention ambiguous");
    assert.ok(msg.includes("group-gather"), "should mention group-gather");
    assert.ok(msg.includes("--group-id"), "should mention --group-id");
    assert.ok(msg.includes("--window"), "should mention --window");
  });

  it("returns matches array for disambiguation", () => {
    const snapshot = {
      windows: [
        {
          windowId: 1,
          focused: true,
          tabs: [
            { tabId: 1, windowId: 1, index: 0, groupId: 10, groupTitle: "Work" },
            { tabId: 2, windowId: 1, index: 1, groupId: 20, groupTitle: "Work" },
          ],
          groups: [
            { groupId: 10, title: "Work" },
            { groupId: 20, title: "Work" },
          ],
        },
      ],
    };

    const result = resolveGroupByTitle(snapshot, buildWindowLabels, "Work");

    assert.ok(result.error);
    const matches = result.error!.matches as Array<Record<string, unknown>>;
    assert.equal(matches.length, 2);
    assert.equal(matches[0].groupId, 10);
    assert.equal(matches[1].groupId, 20);
  });
});

// ---------------------------------------------------------------------------
// normalizeUrl utility
// ---------------------------------------------------------------------------

describe("normalizeUrl", () => {
  it("strips trailing slash", () => {
    const a = normalizeUrl("https://example.com/");
    const b = normalizeUrl("https://example.com");
    assert.equal(a, b);
  });

  it("strips utm parameters", () => {
    const a = normalizeUrl("https://example.com/page?utm_source=test&utm_medium=email");
    const b = normalizeUrl("https://example.com/page");
    assert.equal(a, b);
  });

  it("strips hash", () => {
    const a = normalizeUrl("https://example.com/page#section");
    const b = normalizeUrl("https://example.com/page");
    assert.equal(a, b);
  });

  it("returns null for invalid input", () => {
    assert.equal(normalizeUrl(null), null);
    assert.equal(normalizeUrl(""), null);
    assert.equal(normalizeUrl(123), null);
  });
});
