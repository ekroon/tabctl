import assert from "node:assert/strict";
import test from "node:test";
import { MAX_NATIVE_MESSAGE_BYTES, prepareNativeMessage, serializedBytes } from "./native-message";

for (const [name, html] of [
  ["ASCII", "a".repeat(MAX_NATIVE_MESSAGE_BYTES)],
  ["Unicode", "😀漢".repeat(2_000_000)],
  ["JSON escaping", '"\\\n'.repeat(2_000_000)],
] as const) {
  test(`native page budget includes envelope and ${name} encoding`, () => {
    const original = { id: "page-read", ok: true, data: { status: "READ", html, truncatedHtml: false } };
    const result = prepareNativeMessage(original);
    const extraction = result.data as typeof original.data & { transportTruncated: boolean };
    assert.equal(result.ok, true);
    assert.ok(serializedBytes(result) <= MAX_NATIVE_MESSAGE_BYTES);
    assert.ok(extraction.html.length > 0 && extraction.html.length < html.length);
    assert.ok(html.startsWith(extraction.html));
    assert.equal(extraction.truncatedHtml, true);
    assert.equal(extraction.transportTruncated, true);
    assert.equal(original.data.html, html);
  });
}

test("active cache budget includes open-tab metadata and preserves extraction diagnostics", () => {
  const html = "😀".repeat(2_000);
  const result = prepareNativeMessage({
    id: "cache",
    action: "page-cache-capture",
    ok: true,
    data: {
      openTabs: [{ tabId: 3, title: "x".repeat(200) }],
      extraction: { status: "READ", html, sourceHtmlChars: html.length, truncatedHtml: false },
    },
  }, 2_000);
  const data = result.data as { openTabs: unknown[]; extraction: { html: string; sourceHtmlChars: number; truncatedHtml: boolean } };
  assert.ok(serializedBytes(result) <= 2_000);
  assert.equal(data.openTabs.length, 1);
  assert.equal(data.extraction.sourceHtmlChars, html.length);
  assert.equal(data.extraction.truncatedHtml, true);
  assert.equal(data.extraction.html.length % 2, 0);
});

test("oversized screenshots and snapshots are explicit errors, never partial success", () => {
  for (const data of [{ tiles: [{ dataUrl: "a".repeat(3_000) }] }, { windows: [{ title: "a".repeat(3_000) }] }]) {
    const result = prepareNativeMessage({ id: "oversize", ok: true, data }, 1_000);
    assert.equal(result.ok, false);
    assert.equal(result.id, "oversize");
    assert.equal(result.data, undefined);
    assert.match((result.error as { message: string }).message, /native message.*budget/i);
    assert.ok(serializedBytes(result) <= 1_000);
  }
});

test("cache metadata that cannot fit is rejected rather than falsely cached", () => {
  const result = prepareNativeMessage({
    id: "cache",
    action: "page-cache-capture",
    ok: true,
    data: { openTabs: ["x".repeat(3_000)], extraction: { status: "READ", html: "abc" } },
  }, 1_000);
  assert.equal(result.ok, false);
  assert.equal(result.action, "page-cache-capture");
  assert.equal(result.data, undefined);
});

test("messages fitting the byte budget are unchanged, including exact boundary", () => {
  const message = { id: "small", ok: true, data: { html: "hi", truncatedHtml: false } };
  assert.equal(prepareNativeMessage(message, serializedBytes(message)), message);
});

test("JSON control escapes and unpaired surrogates obey the exact byte budget", () => {
  const html = '\u0000\b\t\n\f\r"\\\ud800x\udfff漢😀'.repeat(200);
  const result = prepareNativeMessage({ id: "escapes", ok: true, data: { html } }, 1_000);
  assert.equal(result.ok, true);
  assert.ok(serializedBytes(result) <= 1_000);
  assert.ok(html.startsWith((result.data as { html: string }).html));
});
