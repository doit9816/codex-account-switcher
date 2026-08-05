param(
  [string]$PacketLibPath
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$tauriDir = Join-Path $repoRoot "src-tauri"
$bundleRoot = Join-Path $tauriDir "target\release\bundle"

function Require-Command([string]$Name) {
  if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
    throw "Missing command: $Name. Install it and add it to PATH."
  }
}

function Find-PacketLib {
  param([string]$ExplicitPath)

  $candidates = @()
  if ($ExplicitPath) {
    $candidates += $ExplicitPath
  }
  $candidates += @(
    (Join-Path $repoRoot "src-tauri\easytier\third_party\x86_64\Packet.lib"),
    "C:\WpdPack\Lib\x64\Packet.lib",
    "C:\npcap-sdk\Lib\x64\Packet.lib",
    "C:\Npcap\Lib\x64\Packet.lib",
    "C:\Program Files\Npcap\Lib\x64\Packet.lib",
    "C:\Program Files (x86)\Npcap\Lib\x64\Packet.lib"
  )

  foreach ($candidate in $candidates) {
    if (Test-Path -LiteralPath $candidate -PathType Leaf) {
      return (Resolve-Path -LiteralPath $candidate).Path
    }
  }

  $cacheRoot = Join-Path $env:LOCALAPPDATA "CodexSwitcher\build-deps\WpdPack_4_1_2"
  $cached = Join-Path $cacheRoot "WpdPack\Lib\x64\Packet.lib"
  if (Test-Path -LiteralPath $cached -PathType Leaf) {
    return (Resolve-Path -LiteralPath $cached).Path
  }

  $downloadRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("codex-wpdpack-" + [guid]::NewGuid().ToString("N"))
  New-Item -ItemType Directory -Path $downloadRoot | Out-Null
  $archive = Join-Path $downloadRoot "WpdPack_4_1_2.zip"
  Write-Host "Packet.lib not found. Downloading the official WinPcap developer pack..." -ForegroundColor Yellow
  Invoke-WebRequest -Uri "https://www.winpcap.org/install/bin/WpdPack_4_1_2.zip" -OutFile $archive
  Expand-Archive -LiteralPath $archive -DestinationPath $downloadRoot -Force

  $downloaded = Join-Path $downloadRoot "WpdPack\Lib\x64\Packet.lib"
  if (-not (Test-Path -LiteralPath $downloaded -PathType Leaf)) {
    throw "Packet.lib was not found in the downloaded developer pack: $downloaded"
  }
  New-Item -ItemType Directory -Path $cacheRoot -Force | Out-Null
  Copy-Item -LiteralPath (Join-Path $downloadRoot "WpdPack") -Destination $cacheRoot -Recurse -Force
  return (Resolve-Path -LiteralPath $cached).Path
}

Set-Location $repoRoot
Require-Command "npm"
Require-Command "cargo"

$package = Get-Content (Join-Path $repoRoot "package.json") -Raw | ConvertFrom-Json
$version = $package.version
$packetLib = Find-PacketLib -ExplicitPath $PacketLibPath
$packetLibDir = Split-Path -Parent $packetLib
$env:LIB = $packetLibDir + ";" + $env:LIB

Write-Host "Building CodexSwitcher v$version..." -ForegroundColor Cyan
Write-Host "Packet.lib: $packetLib"

& npm run tauri build -- --bundles nsis,msi
$buildExitCode = $LASTEXITCODE

$artifacts = @(
  (Join-Path $bundleRoot "nsis\CodexSwitcher_${version}_x64-setup.exe"),
  (Join-Path $bundleRoot "msi\CodexSwitcher_${version}_x64_en-US.msi")
)
$missing = $artifacts | Where-Object { -not (Test-Path -LiteralPath $_ -PathType Leaf) }
if ($missing) {
  throw "Build failed. Missing artifacts: $($missing -join ', ') (exit code $buildExitCode)"
}

if ($buildExitCode -ne 0) {
  Write-Warning "Tauri returned exit code $buildExitCode, but both installers were generated. This is usually caused by a missing TAURI_SIGNING_PRIVATE_KEY."
}

Write-Host "Build complete:" -ForegroundColor Green
foreach ($artifact in $artifacts) {
  $file = Get-Item -LiteralPath $artifact
  Write-Host ("- {0} ({1:N0} bytes)" -f $file.FullName, $file.Length)
}
exit 0
