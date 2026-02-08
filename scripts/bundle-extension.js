/**
 * Bundle the extension background script into a single IIFE file.
 *
 * Chrome service workers cannot use require() or ES module imports (without
 * "type": "module" in manifest.json). After tsc compiles the split TypeScript
 * sources into CommonJS modules under dist/extension/, this script bundles
 * them into a single self-contained background.js.
 */

const { buildSync } = require("esbuild");
const path = require("path");

const dist = path.resolve(__dirname, "..", "dist");

buildSync({
  entryPoints: [path.join(dist, "extension", "background.js")],
  bundle: true,
  outfile: path.join(dist, "extension", "background.js"),
  allowOverwrite: true,
  format: "iife",
  platform: "browser",
  target: "chrome120",
  // Chrome extension APIs are globals, not npm packages
  external: [],
});

console.log("Extension bundled: dist/extension/background.js");
