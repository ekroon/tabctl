import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { detectRuntimeEnvironment, isWslEnvironment, resolveManifestDir, resolveSetupWrapperPath, type WslWindowsPaths } from "../../cli/lib/commands/setup";

test("detectRuntimeEnvironment treats linux with WSL markers as wsl", () => {
  if (process.platform !== "linux") return;

  const prevInterop = process.env.WSL_INTEROP;
  const prevDistro = process.env.WSL_DISTRO_NAME;
  process.env.WSL_INTEROP = "/tmp/wsl-interop";
  process.env.WSL_DISTRO_NAME = "Ubuntu";
  try {
    assert.equal(isWslEnvironment(), true);
    assert.equal(detectRuntimeEnvironment(), "wsl");
  } finally {
    if (prevInterop === undefined) delete process.env.WSL_INTEROP;
    else process.env.WSL_INTEROP = prevInterop;
    if (prevDistro === undefined) delete process.env.WSL_DISTRO_NAME;
    else process.env.WSL_DISTRO_NAME = prevDistro;
  }
});

test("resolveManifestDir supports explicit wsl Windows appdata paths", () => {
  const wslPaths: WslWindowsPaths = {
    windowsLocalAppData: "C:\\Users\\alice\\AppData\\Local",
    unixLocalAppData: "/mnt/c/Users/alice/AppData/Local",
  };

  assert.equal(
    resolveManifestDir("chrome", "wsl", wslPaths),
    "/mnt/c/Users/alice/AppData/Local/Google/Chrome/User Data/NativeMessagingHosts",
  );
  assert.equal(
    resolveManifestDir("edge", "wsl", wslPaths),
    "/mnt/c/Users/alice/AppData/Local/Microsoft/Edge/User Data/NativeMessagingHosts",
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
      "native-linux",
    );

    assert.ok(result.wrapperPath.endsWith(".sh"));
    assert.equal(result.unixWrapperPath, undefined);
    assert.ok(fs.existsSync(result.wrapperPath));
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});
