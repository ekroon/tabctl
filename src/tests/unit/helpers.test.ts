import { test, describe } from "node:test";
import assert from "node:assert/strict";
import {
  parseVersionName,
  requireFiniteId,
  inferIncognitoEvent,
  normalizeEventPayload,
  updateIncognitoState,
  type IncognitoState,
} from "../../extension/helpers.ts";

function makeState(): IncognitoState {
  return {
    incognitoWindowIds: new Set(),
    incognitoTabIds: new Set(),
    incognitoGroupIds: new Set(),
  };
}

// ---------------------------------------------------------------------------
// parseVersionName
// ---------------------------------------------------------------------------
describe("parseVersionName", () => {
  test("returns null gitSha and dirty=false for a stable version", () => {
    assert.deepEqual(parseVersionName("1.0.0"), { gitSha: null, dirty: false });
  });

  test("returns null gitSha for a pre-release without dev suffix", () => {
    assert.deepEqual(parseVersionName("1.0.0-rc.1"), { gitSha: null, dirty: false });
  });

  test("extracts gitSha from dev version", () => {
    const result = parseVersionName("1.0.0-dev.abc123");
    assert.equal(result.gitSha, "abc123");
    assert.equal(result.dirty, false);
  });

  test("extracts gitSha and dirty flag from dirty dev version", () => {
    const result = parseVersionName("1.0.0-dev.abc123.dirty");
    assert.equal(result.gitSha, "abc123");
    assert.equal(result.dirty, true);
  });

  test("returns null for an empty string", () => {
    assert.deepEqual(parseVersionName(""), { gitSha: null, dirty: false });
  });

  test("is case-insensitive for hex sha", () => {
    const lower = parseVersionName("2.0.0-dev.deadbeef");
    const upper = parseVersionName("2.0.0-dev.DEADBEEF");
    assert.equal(lower.gitSha, "deadbeef");
    assert.equal(upper.gitSha, "DEADBEEF");
  });
});

// ---------------------------------------------------------------------------
// requireFiniteId
// ---------------------------------------------------------------------------
describe("requireFiniteId", () => {
  test("returns the number for a valid finite integer", () => {
    assert.equal(requireFiniteId(42, "tabId"), 42);
  });

  test("returns the number for a numeric string", () => {
    assert.equal(requireFiniteId("7", "tabId"), 7);
  });

  test("throws for Infinity", () => {
    assert.throws(() => requireFiniteId(Infinity, "tabId"), /tabId must be a finite number/);
  });

  test("throws for NaN", () => {
    assert.throws(() => requireFiniteId(NaN, "tabId"), /tabId must be a finite number/);
  });

  test("throws for a non-numeric string", () => {
    assert.throws(() => requireFiniteId("abc", "groupId"), /groupId must be a finite number/);
  });

  test("throws for undefined", () => {
    assert.throws(() => requireFiniteId(undefined, "windowId"), /windowId must be a finite number/);
  });

  test("includes the field name and the bad value in the error", () => {
    assert.throws(
      () => requireFiniteId("bad", "myField"),
      (err: unknown) => {
        assert.ok(err instanceof Error);
        assert.ok(err.message.includes("myField"));
        assert.ok(err.message.includes("bad"));
        return true;
      },
    );
  });
});

// ---------------------------------------------------------------------------
// updateIncognitoState
// ---------------------------------------------------------------------------
describe("updateIncognitoState", () => {
  test("populates incognito IDs from a snapshot", () => {
    const state = makeState();
    updateIncognitoState(
      {
        windows: [
          {
            windowId: 10,
            incognito: true,
            tabs: [{ tabId: 101 }, { tabId: 102 }],
            groups: [{ groupId: 1 }],
          },
        ],
      },
      state,
    );
    assert.ok(state.incognitoWindowIds.has(10));
    assert.ok(state.incognitoTabIds.has(101));
    assert.ok(state.incognitoTabIds.has(102));
    assert.ok(state.incognitoGroupIds.has(1));
  });

  test("does not add IDs from non-incognito windows", () => {
    const state = makeState();
    updateIncognitoState(
      {
        windows: [
          { windowId: 20, incognito: false, tabs: [{ tabId: 201 }], groups: [] },
        ],
      },
      state,
    );
    assert.equal(state.incognitoWindowIds.size, 0);
    assert.equal(state.incognitoTabIds.size, 0);
  });

  test("clears previous incognito state on each call", () => {
    const state = makeState();
    state.incognitoWindowIds.add(99);
    state.incognitoTabIds.add(999);
    updateIncognitoState({ windows: [] }, state);
    assert.equal(state.incognitoWindowIds.size, 0);
    assert.equal(state.incognitoTabIds.size, 0);
  });

  test("handles empty snapshot gracefully", () => {
    const state = makeState();
    updateIncognitoState({}, state);
    assert.equal(state.incognitoWindowIds.size, 0);
  });

  test("ignores windows without a numeric windowId", () => {
    const state = makeState();
    updateIncognitoState(
      {
        windows: [
          { windowId: undefined, incognito: true, tabs: [{ tabId: 1 }], groups: [] },
        ],
      },
      state,
    );
    assert.equal(state.incognitoWindowIds.size, 0);
    assert.equal(state.incognitoTabIds.size, 0);
  });
});

// ---------------------------------------------------------------------------
// inferIncognitoEvent
// ---------------------------------------------------------------------------
describe("inferIncognitoEvent", () => {
  test("returns true when tabId is in incognito set", () => {
    const state = makeState();
    state.incognitoTabIds.add(5);
    assert.equal(inferIncognitoEvent({ tabId: 5 }, state), true);
  });

  test("returns true when groupId is in incognito set", () => {
    const state = makeState();
    state.incognitoGroupIds.add(3);
    assert.equal(inferIncognitoEvent({ groupId: 3 }, state), true);
  });

  test("returns true when windowId is in incognito set", () => {
    const state = makeState();
    state.incognitoWindowIds.add(7);
    assert.equal(inferIncognitoEvent({ windowId: 7 }, state), true);
  });

  test("returns false when none of the IDs are incognito", () => {
    const state = makeState();
    assert.equal(inferIncognitoEvent({ tabId: 1, windowId: 2 }, state), false);
  });

  test("returns false for an empty payload", () => {
    assert.equal(inferIncognitoEvent({}, makeState()), false);
  });
});

// ---------------------------------------------------------------------------
// normalizeEventPayload
// ---------------------------------------------------------------------------
describe("normalizeEventPayload", () => {
  test("always includes kind and occurredAt", () => {
    const before = Date.now();
    const event = normalizeEventPayload("tabs.onCreated", { tabId: 1 }, makeState());
    const after = Date.now();
    assert.equal(event.kind, "tabs.onCreated");
    assert.ok(typeof event.occurredAt === "number");
    assert.ok((event.occurredAt as number) >= before && (event.occurredAt as number) <= after);
  });

  test("copies defined payload fields", () => {
    const event = normalizeEventPayload("tabs.onCreated", { tabId: 42, windowId: 10 }, makeState());
    assert.equal(event.tabId, 42);
    assert.equal(event.windowId, 10);
  });

  test("omits undefined payload fields", () => {
    const event = normalizeEventPayload("tabs.onCreated", { tabId: 1, url: undefined }, makeState());
    assert.equal("url" in event, false);
  });

  test("adds incognito=true when tabId is in incognito state", () => {
    const state = makeState();
    state.incognitoTabIds.add(99);
    const event = normalizeEventPayload("tabs.onCreated", { tabId: 99 }, state);
    assert.equal(event.incognito, true);
  });

  test("does not override explicit incognito=true from payload", () => {
    const state = makeState(); // no incognito IDs
    const event = normalizeEventPayload("tabs.onCreated", { tabId: 1, incognito: true }, state);
    assert.equal(event.incognito, true);
  });

  test("does not add incognito when payload and state both say non-incognito", () => {
    const event = normalizeEventPayload("tabs.onCreated", { tabId: 1 }, makeState());
    assert.equal(event.incognito, undefined);
  });
});
