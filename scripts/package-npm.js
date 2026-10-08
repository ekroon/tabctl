#!/usr/bin/env node
"use strict";

const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

function requireFile(file, label, executable = false) {
  let stat;
  try {
    stat = fs.statSync(file);
  } catch {
    throw new Error(`${label} must be a regular${executable ? " executable" : ""} file: ${file}`);
  }
  if (!stat.isFile() || stat.size === 0 || (executable && !(stat.mode & 0o111))) {
    throw new Error(`${label} must be a regular${executable ? " executable" : ""} file: ${file}`);
  }
  return stat;
}

function requireArchitectures(file, expected, label) {
  let actual;
  try {
    actual = execFileSync("lipo", ["-archs", file], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] })
      .trim().split(/\s+/).sort();
  } catch {
    throw new Error(`Cannot inspect ${label} as a Mach-O executable: ${file}`);
  }
  const wanted = [...expected].sort();
  if (actual.join(" ") !== wanted.join(" ")) {
    throw new Error(`${label}: expected ${wanted.join(" ")}, found ${actual.join(" ")}: ${file}`);
  }
  for (const arch of wanted) {
    execFileSync("lipo", [file, "-verify_arch", arch], { stdio: "pipe" });
  }
}

function buildUniversalBinary(arm64, x86, output) {
  if (process.platform !== "darwin") throw new Error("macOS npm packaging requires macOS and lipo");
  const mode = requireFile(arm64, "arm64 input", true).mode & 0o777;
  requireFile(x86, "x86_64 input", true);
  requireArchitectures(arm64, ["arm64"], "arm64 input");
  requireArchitectures(x86, ["x86_64"], "x86_64 input");
  if ([arm64, x86].some((input) => path.resolve(input) === path.resolve(output))) {
    throw new Error("Universal output must not overwrite either input");
  }
  fs.mkdirSync(path.dirname(output), { recursive: true });
  const temporary = `${output}.tmp`;
  try {
    execFileSync("lipo", ["-create", arm64, x86, "-output", temporary], { stdio: "pipe" });
    fs.chmodSync(temporary, mode);
    requireArchitectures(temporary, ["arm64", "x86_64"], "universal output");
    fs.renameSync(temporary, output);
  } finally {
    fs.rmSync(temporary, { force: true });
  }
  return output;
}

function verifyDistribution(root) {
  const binary = path.join(root, "dist", "npm", "tabctl");
  requireFile(binary, "npm executable", true);
  requireArchitectures(binary, ["arm64", "x86_64"], "npm executable");
  for (const name of ["manifest.json", "background.js"]) {
    requireFile(path.join(root, "dist", "extension", name), `extension ${name}`);
  }
  return binary;
}

if (require.main === module) {
  try {
    const root = path.resolve(__dirname, "..");
    const args = process.argv.slice(2);
    if (args.length === 2) {
      buildUniversalBinary(path.resolve(args[0]), path.resolve(args[1]), path.join(root, "dist", "npm", "tabctl"));
    } else if (args.length !== 1 || args[0] !== "--check") {
      throw new Error("Usage: node scripts/package-npm.js <arm64-binary> <x86_64-binary> | --check");
    }
    console.log(`Verified macOS universal npm distribution: ${verifyDistribution(root)}`);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}

module.exports = { buildUniversalBinary, verifyDistribution };
