#!/usr/bin/env bash
set -euo pipefail

if [ -z "${WSL_WORKSPACE:-}" ]; then
  echo "WSL_WORKSPACE is required." >&2
  exit 1
fi

if [ -z "${WSL_DISTRO:-}" ]; then
  echo "WSL_DISTRO is required." >&2
  exit 1
fi

copy_artifact() {
  local source_path="$1"
  local target_name="$2"
  if [ -f "$source_path" ]; then
    cp "$source_path" "$WSL_WORKSPACE/$target_name" || true
  fi
}

install_prerequisites() {
  export DEBIAN_FRONTEND=noninteractive
  sudo apt-get update
  sudo apt-get install -y ca-certificates curl gnupg golang-go
  curl -fsSL https://deb.nodesource.com/setup_24.x | sudo -E bash -
  sudo apt-get install -y nodejs
  if ! command -v google-chrome >/dev/null 2>&1 \
    && ! command -v google-chrome-stable >/dev/null 2>&1 \
    && ! command -v chromium >/dev/null 2>&1 \
    && ! command -v chromium-browser >/dev/null 2>&1; then
    curl -fsSL https://dl.google.com/linux/linux_signing_key.pub | sudo gpg --dearmor -o /usr/share/keyrings/google-linux.gpg
    echo "deb [arch=amd64 signed-by=/usr/share/keyrings/google-linux.gpg] http://dl.google.com/linux/chrome/deb/ stable main" | sudo tee /etc/apt/sources.list.d/google-chrome.list >/dev/null
    sudo apt-get update
    sudo apt-get install -y google-chrome-stable
  fi
}

capture_diagnostics() {
  local diag_file="/tmp/tabctl-wsl-diagnostics.txt"
  {
    echo "distro: ${WSL_DISTRO}"
    echo "uname: $(uname -a)"
    echo "os-release:"
    cat /etc/os-release
    echo "node: $(node --version 2>/dev/null || echo missing)"
    echo "npm: $(npm --version 2>/dev/null || echo missing)"
    echo "go: $(go version 2>/dev/null || echo missing)"
    echo "chrome: $(command -v google-chrome || command -v google-chrome-stable || command -v chromium || command -v chromium-browser || echo missing)"
  } > "$diag_file"
  copy_artifact "$diag_file" "wsl-diagnostics.txt"
}

run_build_and_unit_tests() {
  cd "$WSL_WORKSPACE"
  if [ -f src/tests/unit/fixtures/npx ]; then
    sed -i 's/\r$//' src/tests/unit/fixtures/npx
  fi
  npm ci
  npm run build
  TABCTL_TEST_CLI_TIMEOUT_MS=5000 npm run test:unit
}

run_setup_validation() {
  local setup_output="/tmp/tabctl-wsl-setup.json"
  cd "$WSL_WORKSPACE"
  local extension_id
  extension_id="$(node <<'NODE'
const crypto = require("node:crypto");
const path = require("node:path");
const extensionDir = path.resolve("dist/extension");
const hash = crypto.createHash("sha256").update(extensionDir).digest("hex").slice(0, 32);
let id = "";
for (const c of hash) {
  id += String.fromCharCode("a".charCodeAt(0) + parseInt(c, 16));
}
console.log(id);
NODE
)"

  node dist/cli/tabctl.js setup --browser chrome --extension-id "$extension_id" --json > "$setup_output"
  SETUP_OUTPUT_PATH="$setup_output" node <<'NODE'
const fs = require("node:fs");
const outputPath = process.env.SETUP_OUTPUT_PATH;
const output = JSON.parse(fs.readFileSync(outputPath, "utf8"));
if (!output.ok) throw new Error("setup command failed");
const data = output.data || {};
if (data.runtimeEnv !== "wsl") throw new Error(`expected runtimeEnv=wsl, got ${data.runtimeEnv}`);
if (typeof data.wrapperPath !== "string" || !/^[A-Za-z]:\\/.test(data.wrapperPath)) {
  throw new Error(`expected Windows wrapperPath, got ${data.wrapperPath}`);
}
if (typeof data.windowsManifestPath !== "string" || !/^[A-Za-z]:\\/.test(data.windowsManifestPath)) {
  throw new Error(`expected windowsManifestPath, got ${data.windowsManifestPath}`);
}
if (typeof data.unixWrapperPath !== "string" || !data.unixWrapperPath.startsWith("/")) {
  throw new Error(`expected unixWrapperPath, got ${data.unixWrapperPath}`);
}
console.log("WSL setup output validated");
NODE
  copy_artifact "$setup_output" "wsl-setup.json"
}

find_windows_browser() {
  if [ -f /mnt/c/Program\ Files/Google/Chrome/Application/chrome.exe ]; then
    wslpath -w /mnt/c/Program\ Files/Google/Chrome/Application/chrome.exe
    return
  fi
  if [ -f /mnt/c/Program\ Files/Microsoft/Edge/Application/msedge.exe ]; then
    wslpath -w /mnt/c/Program\ Files/Microsoft/Edge/Application/msedge.exe
    return
  fi
  if [ -f /mnt/c/Program\ Files\ \(x86\)/Microsoft/Edge/Application/msedge.exe ]; then
    wslpath -w /mnt/c/Program\ Files\ \(x86\)/Microsoft/Edge/Application/msedge.exe
    return
  fi
}

run_integration_tests() {
  local log_file="/tmp/tabctl-wsl-integration.log"
  local marker_file="/tmp/tabctl-wsl-execution-marker.txt"
  local allow_skip="${TABCTL_WSL_ALLOW_SKIP:-0}"
  cd "$WSL_WORKSPACE"

  local win_workspace
  win_workspace="$(wslpath -w "$WSL_WORKSPACE")"
  local win_chrome
  win_chrome="$(find_windows_browser || true)"
  if [ -z "$win_chrome" ]; then
    echo "execution=skipped_no_windows_browser" > "$marker_file"
    echo "No Windows Chrome/Edge binary found; skipping integration test." > "$log_file"
    copy_artifact "$log_file" "wsl-integration.log"
    copy_artifact "$marker_file" "wsl-execution-marker.txt"
    if [ "$allow_skip" = "1" ]; then
      exit 0
    fi
    echo "Failing because TABCTL_WSL_ALLOW_SKIP is not enabled." >> "$log_file"
    exit 1
  fi

  echo "Using Windows browser: $win_chrome" > "$log_file"
  echo "execution=running" > "$marker_file"
  set +e
  WIN_WORKSPACE="$win_workspace" WIN_CHROME="$win_chrome" \
    powershell.exe -NoProfile -Command '$ErrorActionPreference="Stop"; Set-Location $env:WIN_WORKSPACE; $env:CHROME_PATH=$env:WIN_CHROME; node dist/scripts/integration-test.js' 2>&1 | tee -a "$log_file"
  local status="${PIPESTATUS[0]}"
  set -e
  if [ "$status" -eq 0 ]; then
    echo "execution=executed" > "$marker_file"
  else
    echo "execution=failed" > "$marker_file"
  fi
  copy_artifact "$log_file" "wsl-integration.log"
  copy_artifact "$marker_file" "wsl-execution-marker.txt"
  return "$status"
}

install_prerequisites
capture_diagnostics
run_build_and_unit_tests
run_setup_validation
run_integration_tests
