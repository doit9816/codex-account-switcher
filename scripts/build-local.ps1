param(
  [string]$PacketLibPath,
  [string]$PacketDllPath
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$tauriDir = Join-Path $repoRoot "src-tauri"
$bundleRoot = Join-Path $tauriDir "target\release\bundle"
$buildStartedAt = Get-Date

$sevenZipCandidates = @(
  (Join-Path ([System.IO.Path]::GetTempPath()) "codex-easytier-build-tools\7z.exe"),
  "C:\Program Files\7-Zip\7z.exe",
  "C:\Program Files (x86)\7-Zip\7z.exe"
)
$sevenZip = $sevenZipCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
if ($sevenZip) {
  $sevenZipDir = Split-Path -Parent (Resolve-Path -LiteralPath $sevenZip).Path
  $env:Path = $sevenZipDir + ";" + $env:Path
}

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

function Find-PacketRuntime {
  param([string]$ExplicitPath)

  $candidates = @()
  if ($ExplicitPath) {
    $candidates += $ExplicitPath
  }
  $candidates += @(
    (Join-Path $repoRoot "src-tauri\easytier\third_party\x86_64\Packet.dll"),
    (Join-Path $env:WINDIR "System32\Npcap\Packet.dll"),
    (Join-Path $env:WINDIR "System32\Packet.dll"),
    "C:\Program Files\Npcap\Packet.dll",
    "C:\Program Files\Npcap\NPF\Packet.dll"
  )

  foreach ($candidate in $candidates) {
    if (Test-Path -LiteralPath $candidate -PathType Leaf) {
      return (Resolve-Path -LiteralPath $candidate).Path
    }
  }

  throw "Packet.dll was not found. Install Npcap, or pass -PacketDllPath <x64 Packet.dll>."
}

Set-Location $repoRoot
Require-Command "npm"
Require-Command "cargo"
Require-Command "7z"

$package = Get-Content (Join-Path $repoRoot "package.json") -Raw | ConvertFrom-Json
$version = $package.version
$packetLib = Find-PacketLib -ExplicitPath $PacketLibPath
$packetDll = Find-PacketRuntime -ExplicitPath $PacketDllPath
$packetLibDir = Split-Path -Parent $packetLib
$env:LIB = $packetLibDir + ";" + $env:LIB
$packetRuntimeDir = Join-Path $repoRoot "src-tauri\easytier\third_party\x86_64"
New-Item -ItemType Directory -Path $packetRuntimeDir -Force | Out-Null
$packetRuntimeTarget = Join-Path $packetRuntimeDir "Packet.dll"
if ([System.IO.Path]::GetFullPath($packetDll) -ne [System.IO.Path]::GetFullPath($packetRuntimeTarget)) {
  Copy-Item -LiteralPath $packetDll -Destination $packetRuntimeTarget -Force
}

Write-Host "Building CodexSwitcher v$version..." -ForegroundColor Cyan
Write-Host "Packet.lib: $packetLib"
Write-Host "Packet.dll: $packetDll"

& npm run tauri build -- --bundles nsis,msi
$buildExitCode = $LASTEXITCODE

$artifacts = @(
  (Join-Path $bundleRoot "nsis\CodexSwitcher_${version}_x64-setup.exe"),
  (Join-Path $bundleRoot "msi\CodexSwitcher_${version}_x64_en-US.msi")
)
$missing = $artifacts | Where-Object {
  if (-not (Test-Path -LiteralPath $_ -PathType Leaf)) {
    return $true
  }
  (Get-Item -LiteralPath $_).LastWriteTime -lt $buildStartedAt
}
if ($missing) {
  throw "Build failed. Missing artifacts: $($missing -join ', ') (exit code $buildExitCode)"
}

if ($buildExitCode -ne 0) {
  Write-Warning "Tauri returned exit code $buildExitCode, but both installers were generated. This is usually caused by a missing TAURI_SIGNING_PRIVATE_KEY."
}

$bundledRuntime = Join-Path $tauriDir "target\release\Packet.dll"
if (-not (Test-Path -LiteralPath $bundledRuntime -PathType Leaf)) {
  throw "Build completed without bundling Packet.dll: $bundledRuntime"
}

Write-Host "Build complete:" -ForegroundColor Green
foreach ($artifact in $artifacts) {
  $file = Get-Item -LiteralPath $artifact
  Write-Host ("- {0} ({1:N0} bytes)" -f $file.FullName, $file.Length)
}
exit 0
