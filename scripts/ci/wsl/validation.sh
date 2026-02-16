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

TIMINGS_FILE="/tmp/tabctl-wsl-timings.txt"
APT_UPDATED=0

rm -f /tmp/tabctl-wsl-diagnostics.txt \
  /tmp/tabctl-wsl-setup.json \
  /tmp/tabctl-wsl-integration.log \
  /tmp/tabctl-wsl-execution-marker.txt \
  /tmp/tabctl-wsl-integration.ps1 \
  "$TIMINGS_FILE"

copy_artifact() {
  local source_path="$1"
  local target_name="$2"
  if [ -f "$source_path" ]; then
    cp "$source_path" "$WSL_WORKSPACE/$target_name" || true
  fi
}

init_timings() {
  {
    echo "distro=${WSL_DISTRO}"
    echo "setup_mode=${TABCTL_WSL_SETUP_MODE:-legacy}"
    echo "started_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  } > "$TIMINGS_FILE"
}

run_timed_phase() {
  local phase="$1"
  shift
  local started_at ended_at status_code
  started_at="$(date +%s)"
  if "$@"; then
    ended_at="$(date +%s)"
    echo "${phase}_seconds=$((ended_at - started_at))" >> "$TIMINGS_FILE"
    echo "${phase}_status=ok" >> "$TIMINGS_FILE"
    return 0
  fi
  status_code="$?"
  ended_at="$(date +%s)"
  echo "${phase}_seconds=$((ended_at - started_at))" >> "$TIMINGS_FILE"
  echo "${phase}_status=failed" >> "$TIMINGS_FILE"
  return "$status_code"
}

copy_timing_artifact() {
  if [ -f "$TIMINGS_FILE" ]; then
    echo "finished_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$TIMINGS_FILE"
  fi
  copy_artifact "$TIMINGS_FILE" "wsl-timings.txt"
}

apt_update_once() {
  if [ "$APT_UPDATED" -eq 0 ]; then
    sudo apt-get update
    APT_UPDATED=1
  fi
}

has_supported_go() {
  if ! command -v go >/dev/null 2>&1; then
    return 1
  fi
  local go_version major minor
  go_version="$(go version 2>/dev/null | awk '{print $3}')"
  if [[ "$go_version" =~ ^go([0-9]+)\.([0-9]+) ]]; then
    major="${BASH_REMATCH[1]}"
    minor="${BASH_REMATCH[2]}"
    if [ "$major" -gt 1 ] || { [ "$major" -eq 1 ] && [ "$minor" -ge 21 ]; }; then
      return 0
    fi
  fi
  return 1
}

install_prerequisites() {
  export DEBIAN_FRONTEND=noninteractive
  local missing_packages=()
  local node_major=""

  for package in ca-certificates curl gnupg; do
    if ! dpkg -s "$package" >/dev/null 2>&1; then
      missing_packages+=("$package")
    fi
  done

  if ! has_supported_go; then
    missing_packages+=("golang-go")
  fi

  if [ "${#missing_packages[@]}" -gt 0 ]; then
    apt_update_once
    sudo apt-get install -y "${missing_packages[@]}"
  fi

  if command -v node >/dev/null 2>&1; then
    node_major="$(node --version | sed -E 's/^v([0-9]+).*/\1/')"
  fi

  if [ "${node_major:-}" != "24" ]; then
    if ! command -v curl >/dev/null 2>&1 || ! command -v gpg >/dev/null 2>&1; then
      apt_update_once
      sudo apt-get install -y curl gnupg
    fi
    curl -fsSL https://deb.nodesource.com/setup_24.x | sudo -E bash -
    sudo apt-get install -y nodejs
  fi
}

capture_diagnostics() {
  local diag_file="/tmp/tabctl-wsl-diagnostics.txt"
  local win_npm_prefix=""
  local win_npm_prefix_status=""
  local win_npm_bin=""
  local win_npm_bin_status=""
  local win_where_tabctl=""
  local win_where_tabctl_status=""
  local win_where_host=""
  local win_where_host_status=""
  local win_tabctl_version=""
  local win_tabctl_version_status=""
  local win_tabctl_path_version="not-run"
  local win_tabctl_path_version_status="n/a"
  local win_host_path_version="not-run"
  local win_host_path_version_status="n/a"
  local first_tabctl_path=""
  local first_host_path=""

  set +e
  win_npm_prefix="$(cmd.exe /d /s /c "npm prefix -g" 2>&1)"
  win_npm_prefix_status="$?"
  win_npm_bin="$(cmd.exe /d /s /c "npm bin -g" 2>&1)"
  win_npm_bin_status="$?"
  win_where_tabctl="$(cmd.exe /d /s /c "where tabctl" 2>&1)"
  win_where_tabctl_status="$?"
  win_where_host="$(cmd.exe /d /s /c "where tabctl-host.exe" 2>&1)"
  win_where_host_status="$?"
  win_tabctl_version="$(timeout 10s cmd.exe /d /s /c "tabctl --version" 2>&1)"
  win_tabctl_version_status="$?"
  first_tabctl_path="$(printf '%s\n' "$win_where_tabctl" | tr -d '\r' | awk 'NF { print; exit }')"
  first_host_path="$(printf '%s\n' "$win_where_host" | tr -d '\r' | awk 'NF { print; exit }')"
  if [ -n "$first_tabctl_path" ]; then
    win_tabctl_path_version="$(timeout 10s cmd.exe /d /s /c "\"$first_tabctl_path\" --version" 2>&1)"
    win_tabctl_path_version_status="$?"
  fi
  if [ -n "$first_host_path" ]; then
    win_host_path_version="$(timeout 10s cmd.exe /d /s /c "\"$first_host_path\" --version" 2>&1)"
    win_host_path_version_status="$?"
  fi
  set -e

  {
    echo "distro: ${WSL_DISTRO}"
    echo "setup_mode: ${TABCTL_WSL_SETUP_MODE:-legacy}"
    echo "uname: $(uname -a)"
    echo "os-release:"
    cat /etc/os-release
    echo "node: $(node --version 2>/dev/null || echo missing)"
    echo "npm: $(npm --version 2>/dev/null || echo missing)"
    echo "go: $(go version 2>/dev/null || echo missing)"
    echo "chrome: $(command -v google-chrome || command -v google-chrome-stable || command -v chromium || command -v chromium-browser || echo missing)"
    echo "windows npm prefix -g (status=${win_npm_prefix_status}):"
    printf '%s\n' "$win_npm_prefix" | tr -d '\r'
    echo "windows npm bin -g (status=${win_npm_bin_status}):"
    printf '%s\n' "$win_npm_bin" | tr -d '\r'
    echo "windows where tabctl (status=${win_where_tabctl_status}):"
    printf '%s\n' "$win_where_tabctl" | tr -d '\r'
    echo "windows where tabctl-host.exe (status=${win_where_host_status}):"
    printf '%s\n' "$win_where_host" | tr -d '\r'
    echo "windows tabctl --version from WSL (status=${win_tabctl_version_status}):"
    printf '%s\n' "$win_tabctl_version" | tr -d '\r'
    echo "windows tabctl path --version from WSL (status=${win_tabctl_path_version_status}):"
    printf '%s\n' "$win_tabctl_path_version" | tr -d '\r'
    echo "windows tabctl-host.exe path --version from WSL (status=${win_host_path_version_status}):"
    printf '%s\n' "$win_host_path_version" | tr -d '\r'
  } > "$diag_file"
  copy_artifact "$diag_file" "wsl-diagnostics.txt"
}

run_build_and_unit_tests() {
  cd "$WSL_WORKSPACE"
  if [ -f src/tests/unit/fixtures/npx ]; then
    sed -i 's/\r$//' src/tests/unit/fixtures/npx
  fi
  npm ci
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
  local ps_runner="/tmp/tabctl-wsl-integration.ps1"
  cat > "$ps_runner" <<'POWERSHELL'
param(
  [Parameter(Mandatory=$true)][string]$Workspace,
  [Parameter(Mandatory=$true)][string]$ChromePath
)
$ErrorActionPreference = "Stop"
Set-Location -LiteralPath $Workspace
$env:CHROME_PATH = $ChromePath
node dist/scripts/integration-test.js
POWERSHELL
  local win_ps_runner
  win_ps_runner="$(wslpath -w "$ps_runner")"
  set +e
  powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$win_ps_runner" "$win_workspace" "$win_chrome" 2>&1 | tee -a "$log_file"
  local status="${PIPESTATUS[0]}"
  set -e
  rm -f "$ps_runner"
  if [ "$status" -eq 0 ]; then
    echo "execution=executed" > "$marker_file"
  else
    echo "execution=failed" > "$marker_file"
  fi
  copy_artifact "$log_file" "wsl-integration.log"
  copy_artifact "$marker_file" "wsl-execution-marker.txt"
  return "$status"
}

init_timings
trap copy_timing_artifact EXIT
run_timed_phase prerequisites install_prerequisites
run_timed_phase diagnostics capture_diagnostics
run_timed_phase build_and_unit run_build_and_unit_tests
run_timed_phase setup_validation run_setup_validation
run_timed_phase integration run_integration_tests
