import { describe, it, afterEach } from "node:test";
import * as assert from "node:assert/strict";
import {
  normalizeUrlForDedupe,
  setRustUrlNormalizer,
} from "../../extension/lib/url-normalizer";

describe("url normalizer rust boundary", () => {
  afterEach(() => {
    setRustUrlNormalizer(null);
  });

  it("matches existing normalization behavior", () => {
    const withTracking = normalizeUrlForDedupe(
      "https://example.com/page/?utm_source=test#section",
    );
    const canonical = normalizeUrlForDedupe("https://example.com/page");
    assert.equal(withTracking, canonical);
  });

  it("uses rust normalizer when provided", () => {
    setRustUrlNormalizer((url) => `rust:${url}`);
    assert.equal(
      normalizeUrlForDedupe("https://example.com"),
      "rust:https://example.com",
    );
  });

  it("falls back to ts normalizer when rust normalizer fails", () => {
    setRustUrlNormalizer(() => {
      throw new Error("wasm unavailable");
    });
    const normalized = normalizeUrlForDedupe(
      "https://example.com/page?utm_medium=email",
    );
    assert.equal(normalized, "https://example.com/page");
  });
});
