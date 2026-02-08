/**
 * Unit tests for WSL utilities.
 */

import { describe, it, mock, beforeEach, afterEach } from "node:test";
import assert from "node:assert";
import fs from "fs";
import { execSync } from "child_process";
import { isWSL, getWindowsUsername, convertToWindowsPath, getWindowsLocalAppData, getWSLDistroName } from "../../shared/wsl";

describe("isWSL", () => {
  const originalPlatform = process.platform;
  const originalReadFileSync = fs.readFileSync;

  afterEach(() => {
    Object.defineProperty(process, "platform", { value: originalPlatform });
    fs.readFileSync = originalReadFileSync;
  });

  it("returns false on non-Linux platforms", () => {
    Object.defineProperty(process, "platform", { value: "win32" });
    assert.strictEqual(isWSL(), false);

    Object.defineProperty(process, "platform", { value: "darwin" });
    assert.strictEqual(isWSL(), false);
  });

  it("returns true when /proc/version contains microsoft", () => {
    Object.defineProperty(process, "platform", { value: "linux" });
    const mockReadFile = mock.fn(() => "Linux version 5.10.16.3-microsoft-standard-WSL2");
    fs.readFileSync = mockReadFile as unknown as typeof fs.readFileSync;
    assert.strictEqual(isWSL(), true);
  });

  it("returns true when /proc/version contains WSL", () => {
    Object.defineProperty(process, "platform", { value: "linux" });
    const mockReadFile = mock.fn(() => "Linux version 5.15.90.1-WSL");
    fs.readFileSync = mockReadFile as unknown as typeof fs.readFileSync;
    assert.strictEqual(isWSL(), true);
  });

  it("returns false when /proc/version does not contain microsoft or WSL", () => {
    Object.defineProperty(process, "platform", { value: "linux" });
    const mockReadFile = mock.fn(() => "Linux version 6.1.0-generic");
    fs.readFileSync = mockReadFile as unknown as typeof fs.readFileSync;
    assert.strictEqual(isWSL(), false);
  });

  it("returns false when /proc/version cannot be read", () => {
    Object.defineProperty(process, "platform", { value: "linux" });
    const mockReadFile = mock.fn(() => {
      throw new Error("ENOENT");
    });
    fs.readFileSync = mockReadFile as unknown as typeof fs.readFileSync;
    assert.strictEqual(isWSL(), false);
  });
});

describe("getWSLDistroName", () => {
  const originalEnv = process.env.WSL_DISTRO_NAME;

  afterEach(() => {
    if (originalEnv !== undefined) {
      process.env.WSL_DISTRO_NAME = originalEnv;
    } else {
      delete process.env.WSL_DISTRO_NAME;
    }
  });

  it("returns the WSL_DISTRO_NAME environment variable", () => {
    process.env.WSL_DISTRO_NAME = "Ubuntu-22.04";
    assert.strictEqual(getWSLDistroName(), "Ubuntu-22.04");
  });

  it("throws when WSL_DISTRO_NAME is not set", () => {
    delete process.env.WSL_DISTRO_NAME;
    assert.throws(() => getWSLDistroName(), /WSL_DISTRO_NAME environment variable not set/);
  });
});

// Note: getWindowsUsername, convertToWindowsPath, and getWindowsLocalAppData
// require actual WSL environment or complex mocking of execSync.
// These are better tested via integration tests in a real WSL environment.
// Basic smoke tests can be added here if needed.
