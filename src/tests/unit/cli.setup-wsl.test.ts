import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { detectRuntimeEnvironment, resolveManifestDir, resolveSetupWrapperPath } from "../../cli/lib/commands/setup";

test("detectRuntimeEnvironment treats linux with WSL markers as native-linux", () => {
  if (process.platform !== "linux") return;

  const prevInterop = process.env.WSL_INTEROP;
  const prevDistro = process.env.WSL_DISTRO_NAME;
  process.env.WSL_INTEROP = "/tmp/wsl-interop";
  process.env.WSL_DISTRO_NAME = "Ubuntu";
  try {
    assert.equal(detectRuntimeEnvironment(), "native-linux");
  } finally {
    if (prevInterop === undefined) delete process.env.WSL_INTEROP;
    else process.env.WSL_INTEROP = prevInterop;
    if (prevDistro === undefined) delete process.env.WSL_DISTRO_NAME;
    else process.env.WSL_DISTRO_NAME = prevDistro;
  }
});

test("resolveManifestDir resolves native Linux paths", () => {
  const home = os.homedir();
  const expectedChrome = path.join(home, ".config", "google-chrome", "NativeMessagingHosts");
  const expectedEdge = path.join(home, ".config", "microsoft-edge", "NativeMessagingHosts");

  assert.equal(
    resolveManifestDir("chrome", "native-linux"),
    expectedChrome,
  );
  assert.equal(
    resolveManifestDir("edge", "native-linux"),
    expectedEdge,
  );
});

test("resolveSetupWrapperPath keeps unix wrapper for native linux runtime", () => {
  if (process.platform === "win32") return;

  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tabctl-setup-wrapper-"));
  try {
    const result = resolveSetupWrapperPath(
      process.execPath,
      path.join(tmpDir, "host.bundle.js"),
      "edge",
      tmpDir,
    );

    assert.ok(result.wrapperPath.endsWith(".sh"));
    assert.ok(fs.existsSync(result.wrapperPath));
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});
