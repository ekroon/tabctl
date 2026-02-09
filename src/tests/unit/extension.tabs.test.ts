import assert from "node:assert/strict";
import test from "node:test";
import {
  normalizeUrl,
  normalizeTabIndex,
  getMostRecentFocusedWindowId,
  resolveOpenWindow,
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
