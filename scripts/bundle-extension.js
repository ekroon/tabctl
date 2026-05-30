/**
 * Bundle the extension background script into a single file.
 *
 * Chrome service workers cannot use require() or ES module imports.
 * We bundle directly from the TypeScript entrypoint so esbuild can resolve
 * ESM-only runtime dependencies before writing dist/extension/background.js.
 */

const { buildSync } = require("esbuild");
const path = require("path");

const root = path.resolve(__dirname, "..");
const dist = path.join(root, "dist");

// Extension bundle (IIFE for service worker)
buildSync({
  entryPoints: [path.join(root, "src", "extension", "background.ts")],
  bundle: true,
  outfile: path.join(dist, "extension", "background.js"),
  allowOverwrite: true,
  format: "iife",
  platform: "browser",
  target: "chrome120",
  external: [],
});
console.log("Extension bundled: dist/extension/background.js");
