"use strict";

const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const { after, before, test } = require("node:test");
const { buildUniversalBinary, verifyDistribution } = require("./package-npm");

const root = path.resolve(__dirname, "..");
let fixture;
let arm64;
let x86;

before(() => {
  const scratch = path.join(root, "build", "npm-packaging-tests");
  fs.mkdirSync(scratch, { recursive: true });
  fixture = fs.mkdtempSync(path.join(scratch, "case-"));
  arm64 = path.join(fixture, "arm64");
  x86 = path.join(fixture, "x86_64");
  for (const [arch, output] of [["arm64", arm64], ["x86_64", x86]]) {
    execFileSync("clang", ["-arch", arch, "-x", "c", "-", "-o", output], {
      input: "int main(void) { return 0; }\n",
    });
  }
});

after(() => {
  if (fixture) fs.rmSync(fixture, { recursive: true, force: true });
});

test("combines the two native slices and preserves executable mode", () => {
  fs.chmodSync(arm64, 0o755);
  const output = path.join(fixture, "universal", "tabctl");
  assert.equal(buildUniversalBinary(arm64, x86, output), output);
  for (const arch of ["arm64", "x86_64"]) {
    execFileSync("lipo", [output, "-verify_arch", arch]);
  }
  assert.equal(fs.statSync(output).mode & 0o777, 0o755);
  execFileSync(output);
  const repeated = path.join(fixture, "universal", "tabctl-repeat");
  buildUniversalBinary(arm64, x86, repeated);
  assert.deepEqual(fs.readFileSync(repeated), fs.readFileSync(output));
});

test("missing inputs fail before producing an executable", () => {
  const output = path.join(fixture, "missing-output");
  assert.throws(
    () => buildUniversalBinary(path.join(fixture, "missing"), x86, output),
    /arm64 input.*regular executable file/i,
  );
  assert.equal(fs.existsSync(output), false);
});

test("rejects inputs with swapped or duplicate architectures", () => {
  assert.throws(() => buildUniversalBinary(x86, arm64, path.join(fixture, "swapped")), /expected arm64/i);
  assert.throws(() => buildUniversalBinary(arm64, arm64, path.join(fixture, "duplicate")), /expected x86_64/i);
});

test("never overwrites either input", () => {
  assert.throws(() => buildUniversalBinary(arm64, x86, arm64), /must not overwrite/i);
  assert.throws(() => buildUniversalBinary(arm64, x86, x86), /must not overwrite/i);
});

test("rejects a non-executable input", () => {
  const input = path.join(fixture, "not-executable");
  fs.copyFileSync(arm64, input);
  fs.chmodSync(input, 0o644);
  assert.throws(() => buildUniversalBinary(input, x86, path.join(fixture, "bad-mode")), /regular executable file/i);
});

test("rejects a non-Mach-O input without producing an executable", () => {
  const input = path.join(fixture, "not-mach-o");
  fs.writeFileSync(input, "not a Mach-O executable\n", { mode: 0o755 });
  const output = path.join(fixture, "bad-format");
  assert.throws(() => buildUniversalBinary(input, x86, output), /cannot inspect arm64 input/i);
  assert.equal(fs.existsSync(output), false);
});

test("distribution verification requires both native slices and extension assets", () => {
  const packageRoot = path.join(fixture, "package");
  const binary = path.join(packageRoot, "dist", "npm", "tabctl");
  buildUniversalBinary(arm64, x86, binary);
  assert.throws(() => verifyDistribution(packageRoot), /manifest\.json/);
  const extension = path.join(packageRoot, "dist", "extension");
  fs.mkdirSync(extension, { recursive: true });
  fs.writeFileSync(path.join(extension, "manifest.json"), "{}\n");
  assert.throws(() => verifyDistribution(packageRoot), /background\.js/);
  fs.writeFileSync(path.join(extension, "background.js"), "(() => {})();\n");
  assert.equal(verifyDistribution(packageRoot), binary);
  fs.copyFileSync(arm64, binary);
  assert.throws(() => verifyDistribution(packageRoot), /expected arm64 x86_64/i);
});
