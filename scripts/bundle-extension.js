/**
 * Bundle the extension background script and host into single files.
 *
 * Extension: Chrome service workers cannot use require() or ES module imports.
 * After tsc compiles the split TypeScript sources into CommonJS modules under
 * dist/extension/, this script bundles them into a single self-contained
 * background.js.
 *
 * Host: The native messaging host is bundled into a single file so it can be
 * synced to a stable path (~/.local/state/tabctl/host.bundle.js) that survives npm
 * upgrades without re-running `tabctl setup`.
 */

const { buildSync } = require("esbuild");
const path = require("path");

const dist = path.resolve(__dirname, "..", "dist");

// Extension bundle (IIFE for service worker)
buildSync({
  entryPoints: [path.join(dist, "extension", "background.js")],
  bundle: true,
  outfile: path.join(dist, "extension", "background.js"),
  allowOverwrite: true,
  format: "iife",
  platform: "browser",
  target: "chrome120",
  external: [],
});
console.log("Extension bundled: dist/extension/background.js");

// Host bundle (CJS for Node, externalize builtins)
buildSync({
  entryPoints: [path.join(dist, "host", "host.js")],
  bundle: true,
  outfile: path.join(dist, "host", "host.bundle.js"),
  format: "cjs",
  platform: "node",
  target: "node20",
});
console.log("Host bundled: dist/host/host.bundle.js");
