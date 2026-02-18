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
  /tmp/tabctl-wsl-invocation.ps1 \
  /tmp/tabctl-wsl-build.ps1 \
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
    echo "setup_target=windows-cli"
    echo "started_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  } > "$TIMINGS_FILE"
}

run_timed_phase() {
  local phase="$1"
  shift
  local started_at ended_at status_code
  started_at="$(date +%s)"
  set +e
  (
    set -euo pipefail
    "$@"
  )
  status_code="$?"
  set -e
  ended_at="$(date +%s)"
  echo "${phase}_seconds=$((ended_at - started_at))" >> "$TIMINGS_FILE"
  if [ "$status_code" -eq 0 ]; then
    echo "${phase}_status=ok" >> "$TIMINGS_FILE"
  else
    echo "${phase}_status=failed" >> "$TIMINGS_FILE"
  fi
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
  first_tabctl_path="$(printf '%s\n' "$win_where_tabctl" | tr -d '\r' | grep -E '^[A-Za-z]:\\\\' | head -n1 || true)"
  first_host_path="$(printf '%s\n' "$win_where_host" | tr -d '\r' | grep -E '^[A-Za-z]:\\\\' | head -n1 || true)"
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
    echo "setup_target: windows-cli"
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
  local ps_runner win_ps_runner win_workspace ps_status
  ps_runner="/tmp/tabctl-wsl-build.ps1"
  win_ps_runner="$(wslpath -w "$ps_runner")"
  win_workspace="$(wslpath -w "$WSL_WORKSPACE")"
  cat > "$ps_runner" <<'POWERSHELL'
param(
  [Parameter(Mandatory=$true)][string]$Workspace
)
$ErrorActionPreference = "Stop"
Set-Location -LiteralPath $Workspace
& npm ci
if ($LASTEXITCODE -ne 0) {
  exit $LASTEXITCODE
}
& npm run build
if ($LASTEXITCODE -ne 0) {
  exit $LASTEXITCODE
}
POWERSHELL
  set +e
  powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$win_ps_runner" "$win_workspace"
  ps_status="$?"
  set -e
  rm -f "$ps_runner"
  return "$ps_status"
}

run_setup_validation() {
  local setup_output="/tmp/tabctl-wsl-setup.json"
  cd "$WSL_WORKSPACE"
  local win_workspace win_launcher_version
  win_workspace="$(wslpath -m "$WSL_WORKSPACE")"
  win_launcher_version="$(node -e 'const pkg = require("./package.json"); process.stdout.write((pkg.optionalDependencies && pkg.optionalDependencies["tabctl-win32-x64"]) || "")')"
  if [ -n "$win_launcher_version" ]; then
    cmd.exe /d /c npm install -g "$win_workspace" "tabctl-win32-x64@$win_launcher_version" --no-fund --no-audit
  else
    cmd.exe /d /c npm install -g "$win_workspace" --no-fund --no-audit
  fi
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

  local ps_runner win_ps_runner win_setup_output setup_status
  ps_runner="/tmp/tabctl-wsl-setup.ps1"
  win_ps_runner="$(wslpath -w "$ps_runner")"
  win_setup_output="$(wslpath -w "$setup_output")"
  cat > "$ps_runner" <<'POWERSHELL'
param(
  [Parameter(Mandatory=$true)][string]$Workspace,
  [Parameter(Mandatory=$true)][string]$ExtensionId,
  [Parameter(Mandatory=$true)][string]$OutputPath
)
$ErrorActionPreference = "Stop"
Set-Location -LiteralPath $Workspace
$tabctlCommand = @()
if (Test-Path -LiteralPath "dist\\cli\\tabctl.js") {
  $workspaceScript = (Resolve-Path -LiteralPath "dist\\cli\\tabctl.js").Path
  $tabctlCommand = @("node", $workspaceScript)
}
$prefix = (& npm prefix -g).Trim()
if ($tabctlCommand.Count -eq 0 -and $prefix) {
  $cmdCandidate = Join-Path $prefix "tabctl.cmd"
  if (Test-Path -LiteralPath $cmdCandidate) {
    $tabctlCommand = @($cmdCandidate)
  } else {
    $scriptCandidate = Join-Path $prefix "node_modules\\tabctl\\dist\\cli\\tabctl.js"
    if (Test-Path -LiteralPath $scriptCandidate) {
      $tabctlCommand = @("node", $scriptCandidate)
    }
  }
}
if ($tabctlCommand.Count -eq 0) {
  $cmd = Get-Command tabctl -CommandType Application -ErrorAction SilentlyContinue
  if ($cmd) {
    $tabctlCommand = @($cmd.Source)
  }
}
if ($tabctlCommand.Count -eq 0) {
  $workspaceScriptExists = Test-Path -LiteralPath "dist\\cli\\tabctl.js"
  throw "Failed to resolve Windows tabctl executable (npm prefix -g: '$prefix', workspace script exists: '$workspaceScriptExists')."
}
$command = $tabctlCommand[0]
$commandArgs = @()
if ($tabctlCommand.Count -gt 1) {
  $commandArgs += $tabctlCommand[1..($tabctlCommand.Count - 1)]
}
$commandArgs += @("setup", "--browser", "chrome", "--extension-id", $ExtensionId, "--json")
$json = & $command @commandArgs
$exitCode = $LASTEXITCODE
$json | Out-File -LiteralPath $OutputPath -Encoding utf8
if ($exitCode -ne 0) {
  exit $exitCode
}
POWERSHELL
  set +e
  powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$win_ps_runner" "$win_workspace" "$extension_id" "$win_setup_output"
  setup_status="$?"
  set -e
  rm -f "$ps_runner"
  if [ "$setup_status" -ne 0 ]; then
    cat "$setup_output" >&2 || true
    return "$setup_status"
  fi
  SETUP_OUTPUT_PATH="$setup_output" node <<'NODE'
const fs = require("node:fs");
const outputPath = process.env.SETUP_OUTPUT_PATH;
const output = JSON.parse(fs.readFileSync(outputPath, "utf8").replace(/^\uFEFF/, ""));
if (!output.ok) throw new Error("setup command failed");
const data = output.data || {};
if (data.runtimeEnv !== "native-win32") throw new Error(`expected runtimeEnv=native-win32, got ${data.runtimeEnv}`);
if (typeof data.wrapperPath !== "string" || !/^[A-Za-z]:\\/.test(data.wrapperPath)) {
  throw new Error(`expected Windows wrapperPath, got ${data.wrapperPath}`);
}
if (typeof data.manifestPath !== "string" || !/^[A-Za-z]:\\/.test(data.manifestPath)) {
  throw new Error(`expected Windows manifestPath, got ${data.manifestPath}`);
}
if ("unixWrapperPath" in data) {
  throw new Error("did not expect unixWrapperPath in setup output");
}
if ("windowsManifestPath" in data) {
  throw new Error("did not expect windowsManifestPath in setup output");
}
console.log("WSL setup output validated");
NODE
  copy_artifact "$setup_output" "wsl-setup.json"
}

run_windows_invocation_checks() {
  cd "$WSL_WORKSPACE"
  local expected_version
  expected_version="$(node -e 'process.stdout.write(require("./package.json").version)')"

  local cmd_tabctl_version cmd_tabctl_version_status
  set +e
  cmd_tabctl_version="$(timeout 10s cmd.exe /d /s /c "tabctl --version" 2>&1)"
  cmd_tabctl_version_status="$?"
  set -e
  if [ "$cmd_tabctl_version_status" -ne 0 ]; then
    echo "Windows invocation check failed: cmd.exe could not run tabctl --version." >&2
    printf '%s\n' "$cmd_tabctl_version" | tr -d '\r' >&2
    return 1
  fi
  local cmd_tabctl_version_clean
  cmd_tabctl_version_clean="$(printf '%s\n' "$cmd_tabctl_version" | tr -d '\r' | head -n1 | tr -d '[:space:]')"
  if [ "$cmd_tabctl_version_clean" != "$expected_version" ]; then
    echo "Windows invocation check failed: expected tabctl --version=$expected_version via cmd.exe, got '$cmd_tabctl_version_clean'." >&2
    return 1
  fi

  local ps_runner win_ps_runner
  ps_runner="/tmp/tabctl-wsl-invocation.ps1"
  win_ps_runner="$(wslpath -w "$ps_runner")"
  cat > "$ps_runner" <<'POWERSHELL'
param(
  [Parameter(Mandatory=$true)][string]$ExpectedVersion
)
$ErrorActionPreference = "Stop"

$tabctlVersion = (& tabctl --version).Trim()
if ($tabctlVersion -ne $ExpectedVersion) {
  throw "expected tabctl --version=$ExpectedVersion via PowerShell, got '$tabctlVersion'"
}

$hostPath = (Get-Command tabctl-host.exe -CommandType Application -ErrorAction Stop).Source
if (-not (Test-Path -LiteralPath $hostPath)) {
  throw "tabctl-host.exe was resolved but path does not exist: $hostPath"
}

$request = @{ id = "wsl-host-version"; action = "version"; params = @{} } | ConvertTo-Json -Compress
$requestBytes = [System.Text.Encoding]::UTF8.GetBytes($request)
$lengthBytes = [System.BitConverter]::GetBytes([int]$requestBytes.Length)

$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = $hostPath
$psi.UseShellExecute = $false
$psi.RedirectStandardInput = $true
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError = $true
$process = [System.Diagnostics.Process]::Start($psi)
if (-not $process) {
  throw "failed to start tabctl-host.exe"
}

$stdin = $process.StandardInput.BaseStream
$stdout = $process.StandardOutput.BaseStream
$stdin.Write($lengthBytes, 0, 4)
$stdin.Write($requestBytes, 0, $requestBytes.Length)
$stdin.Flush()
$process.StandardInput.Close()

$responseLengthBytes = New-Object byte[] 4
$readLength = 0
while ($readLength -lt 4) {
  $count = $stdout.Read($responseLengthBytes, $readLength, 4 - $readLength)
  if ($count -le 0) { break }
  $readLength += $count
}
if ($readLength -ne 4) {
  $stderr = $process.StandardError.ReadToEnd()
  throw "tabctl-host.exe did not return a native response header (stderr: $stderr)"
}

$responseLength = [System.BitConverter]::ToInt32($responseLengthBytes, 0)
if ($responseLength -le 0 -or $responseLength -gt 1048576) {
  throw "invalid native response length from tabctl-host.exe: $responseLength"
}

$responseBytes = New-Object byte[] $responseLength
$readBody = 0
while ($readBody -lt $responseLength) {
  $count = $stdout.Read($responseBytes, $readBody, $responseLength - $readBody)
  if ($count -le 0) { break }
  $readBody += $count
}
if ($readBody -ne $responseLength) {
  $stderr = $process.StandardError.ReadToEnd()
  throw "tabctl-host.exe returned truncated native response (stderr: $stderr)"
}

$responseJson = [System.Text.Encoding]::UTF8.GetString($responseBytes)
$response = $responseJson | ConvertFrom-Json
if (-not $response.ok) {
  throw "tabctl-host.exe version request failed: $responseJson"
}
if ($response.action -ne "version") {
  throw "unexpected action from tabctl-host.exe: $($response.action)"
}
if (-not $response.data -or -not $response.data.version) {
  throw "tabctl-host.exe version response missing data.version: $responseJson"
}

if (-not $process.WaitForExit(10000)) {
  $process.Kill()
  throw "tabctl-host.exe did not exit within timeout after version request"
}
POWERSHELL

  set +e
  powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$win_ps_runner" "$expected_version"
  local ps_status="$?"
  set -e
  rm -f "$ps_runner"
  if [ "$ps_status" -ne 0 ]; then
    echo "Windows invocation check failed: PowerShell bridge validation for tabctl-host.exe failed." >&2
    return "$ps_status"
  fi
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
run_timed_phase windows_invocation run_windows_invocation_checks
run_timed_phase integration run_integration_tests
