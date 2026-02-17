import normalizeUrlLib from "normalize-url";

export type RustUrlNormalizer = (url: string) => string | null;

let rustUrlNormalizer: RustUrlNormalizer | null = null;

export function setRustUrlNormalizer(normalizer: RustUrlNormalizer | null) {
  rustUrlNormalizer = typeof normalizer === "function" ? normalizer : null;
}

function normalizeUrlWithTs(url: string): string | null {
  try {
    return normalizeUrlLib(url, {
      stripHash: true,
      removeQueryParameters: [
        /^utm_\w+$/i,
        "fbclid",
        "gclid",
        "igshid",
        "mc_cid",
        "mc_eid",
        "ref",
        "ref_src",
        "ref_url",
        "si",
      ],
    });
  } catch {
    return null;
  }
}

export function normalizeUrlForDedupe(rawUrl: unknown): string | null {
  if (!rawUrl || typeof rawUrl !== "string") {
    return null;
  }

  if (rustUrlNormalizer) {
    try {
      const normalized = rustUrlNormalizer(rawUrl);
      if (typeof normalized === "string" || normalized === null) {
        return normalized;
      }
    } catch {
      // Fall through to TS implementation if Rust/WASM binding fails.
    }
  }

  return normalizeUrlWithTs(rawUrl);
}
