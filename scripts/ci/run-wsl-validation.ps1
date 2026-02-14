param(
  [string]$Workspace = $env:GITHUB_WORKSPACE,
  [switch]$AllowIntegrationSkip
)

$ErrorActionPreference = "Stop"

if (-not $Workspace) {
  throw "Workspace is required. Pass -Workspace or set GITHUB_WORKSPACE."
}
$Workspace = (Resolve-Path -Path $Workspace).Path

function Resolve-WslDistro {
  function Get-AvailableDistros {
    return @(wsl --list --quiet |
      ForEach-Object { ($_ -replace "`0", "").Trim() } |
      Where-Object { $_ -and $_ -notlike "docker-desktop*" })
  }

  wsl --status | Out-Host
  $distros = Get-AvailableDistros
  $distro = $distros | Where-Object { $_ -like "Ubuntu*" } | Select-Object -First 1
  if (-not $distro) {
    $distro = $distros | Select-Object -First 1
  }
  if (-not $distro) {
    Write-Host "No WSL distro found; installing Ubuntu..."
    wsl --install -d Ubuntu --no-launch | Out-Host
    for ($i = 0; $i -lt 12 -and -not $distro; $i++) {
      $distros = Get-AvailableDistros
      $distro = $distros | Where-Object { $_ -like "Ubuntu*" } | Select-Object -First 1
      if (-not $distro) {
        $distro = $distros | Select-Object -First 1
      }
      if (-not $distro) {
        Start-Sleep -Seconds 5
      }
    }
  }
  if (-not $distro) {
    throw "No WSL distro available after installation attempt."
  }
  return [string]$distro
}

function Copy-WslFile {
  param(
    [string]$Distro,
    [string]$SourcePath,
    [string]$DestinationPath
  )

  $content = wsl -d $Distro -- bash -lc "cat '$SourcePath' 2>/dev/null"
  if ($LASTEXITCODE -eq 0 -and $content) {
    $content | Out-File -FilePath $DestinationPath -Encoding utf8
  }
}

$distro = Resolve-WslDistro
wsl -d $distro -- uname -a
Write-Host "Using WSL distro: $distro"

$wslWorkspace = (wsl -d $distro -- env WIN_WORKSPACE="$Workspace" bash -lc 'wslpath -a -u "$WIN_WORKSPACE"').Trim()
if (-not $wslWorkspace) {
  throw "Could not resolve WSL workspace path."
}
Write-Host "WSL workspace path: $wslWorkspace"

$allowSkip = if ($AllowIntegrationSkip.IsPresent) { "1" } else { "0" }
$wslScriptPath = "$wslWorkspace/scripts/ci/wsl/validation.sh"
wsl -d $distro -- env WSL_WORKSPACE="$wslWorkspace" WSL_DISTRO="$distro" TABCTL_WSL_ALLOW_SKIP="$allowSkip" bash "$wslScriptPath"
$status = $LASTEXITCODE

$diagPath = Join-Path $Workspace "wsl-diagnostics.txt"
$setupPath = Join-Path $Workspace "wsl-setup.json"
$integrationPath = Join-Path $Workspace "wsl-integration.log"
$markerPath = Join-Path $Workspace "wsl-execution-marker.txt"

Copy-WslFile -Distro $distro -SourcePath "/tmp/tabctl-wsl-diagnostics.txt" -DestinationPath $diagPath
Copy-WslFile -Distro $distro -SourcePath "/tmp/tabctl-wsl-setup.json" -DestinationPath $setupPath
Copy-WslFile -Distro $distro -SourcePath "/tmp/tabctl-wsl-integration.log" -DestinationPath $integrationPath
Copy-WslFile -Distro $distro -SourcePath "/tmp/tabctl-wsl-execution-marker.txt" -DestinationPath $markerPath

if (-not (Test-Path $diagPath)) {
  "WSL diagnostics were not generated." | Out-File -FilePath $diagPath -Encoding utf8
}
if (-not (Test-Path $setupPath)) {
  "{`"ok`":false,`"error`":`"WSL setup output was not generated.`"}" | Out-File -FilePath $setupPath -Encoding utf8
}
if (-not (Test-Path $integrationPath)) {
  "WSL integration log was not generated." | Out-File -FilePath $integrationPath -Encoding utf8
}
if (-not (Test-Path $markerPath)) {
  "execution=unknown" | Out-File -FilePath $markerPath -Encoding utf8
}

if ($status -ne 0) {
  exit $status
}
