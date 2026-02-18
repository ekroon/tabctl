/**
 * Bundle the extension background script into a single file.
 *
 * Chrome service workers cannot use require() or ES module imports.
 * After tsc compiles the split TypeScript sources into CommonJS modules under
 * dist/extension/, this script bundles them into a self-contained background.js.
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
