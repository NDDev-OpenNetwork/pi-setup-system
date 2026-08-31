# Install pi-setup-system for the current user on Windows.
#
# Downloads the release artifact for this platform, checks it against the
# release's own SHA256SUMS, and places it under %LOCALAPPDATA%\Programs.
#
#   powershell -ExecutionPolicy Bypass -File install.ps1
#   powershell -ExecutionPolicy Bypass -File install.ps1 -Version 0.1.0
[CmdletBinding()]
param(
  [string]$Version = "0.0.50",
  [string]$InstallDir = "$env:LOCALAPPDATA\Programs\pi-setup-system"
)
$ErrorActionPreference = "Stop"

$repo   = "NDDev-OpenNetwork/pi-setup-system"
$binary = "pi-setup-system"
$arch   = switch ($env:PROCESSOR_ARCHITECTURE) {
  "AMD64" { "x86_64" }
  "ARM64" { "aarch64" }
  default { throw "unsupported architecture: $env:PROCESSOR_ARCHITECTURE" }
}
$asset = "$binary-$arch-pc-windows-msvc.exe"
$base  = "https://github.com/$repo/releases/download/$Version"
$work  = Join-Path $env:TEMP ([System.Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $work -Force | Out-Null

try {
  Write-Host "fetching $asset $Version"
  Invoke-WebRequest -Uri "$base/$asset"     -OutFile (Join-Path $work $asset) -UseBasicParsing
  Invoke-WebRequest -Uri "$base/SHA256SUMS" -OutFile (Join-Path $work "SHA256SUMS") -UseBasicParsing

  # Checked before anything is placed, against the release's own list rather
  # than a value written into this script.
  $line = Select-String -Path (Join-Path $work "SHA256SUMS") -Pattern ([regex]::Escape($asset)) |
          Select-Object -First 1
  if (-not $line) { throw "$asset is not listed in SHA256SUMS" }
  $want = ($line.Line -split '\s+')[0]
  $got  = (Get-FileHash -Algorithm SHA256 -Path (Join-Path $work $asset)).Hash.ToLower()
  if ($got -ne $want.ToLower()) { throw "digest mismatch for $asset" }

  New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
  Move-Item -Force -Path (Join-Path $work $asset) -Destination (Join-Path $InstallDir "$binary.exe")

  $installed = Join-Path $InstallDir "$binary.exe"
  Write-Host "installed $installed"
  Write-Host ""
  Write-Host "Point ai-stp at it with the full path:"
  Write-Host "  ai-stp provider conformance --harness pi ``"
  Write-Host "    --executable $installed --target <dir> --protocol-version 3 --json"
  if (-not ($env:PATH -split ';' | Where-Object { $_ -eq $InstallDir })) {
    Write-Host ""
    Write-Host "note: $InstallDir is not on your PATH."
  }
} finally {
  Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
