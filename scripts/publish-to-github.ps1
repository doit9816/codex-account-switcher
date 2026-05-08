param(
  [Parameter(Mandatory = $true)]
  [string]$RemoteUrl,

  [string]$Branch = "main",
  [string]$CommitMessage = "Initial Codex Account Switcher release"
)

$ErrorActionPreference = "Stop"

$projectRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$publishRoot = Join-Path ([System.IO.Path]::GetTempPath()) "codex-account-switcher-publish"
$resolvedTemp = Resolve-Path ([System.IO.Path]::GetTempPath())

if (Test-Path -LiteralPath $publishRoot) {
  $resolvedPublish = Resolve-Path $publishRoot
  if (-not $resolvedPublish.Path.StartsWith($resolvedTemp.Path, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to remove unexpected publish directory: $resolvedPublish"
  }
  Remove-Item -LiteralPath $publishRoot -Recurse -Force
}

New-Item -ItemType Directory -Path $publishRoot | Out-Null

$excludedDirs = @(
  ".git",
  "node_modules",
  "dist",
  "src-tauri\target",
  "src-tauri\gen"
)

$excludedExtensions = @(".log", ".tmp")
$excludedNames = @(".env")

Get-ChildItem -LiteralPath $projectRoot -Recurse -Force -File | ForEach-Object {
  $relative = $_.FullName.Substring($projectRoot.Path.Length).TrimStart("\", "/")
  $normalized = $relative -replace "/", "\"
  $isExcludedDir = $excludedDirs | Where-Object {
    $normalized.Equals($_, [System.StringComparison]::OrdinalIgnoreCase) -or
    $normalized.StartsWith("$($_)\", [System.StringComparison]::OrdinalIgnoreCase)
  }
  if ($isExcludedDir) {
    return
  }
  if ($excludedExtensions -contains $_.Extension.ToLowerInvariant() -or $excludedNames -contains $_.Name) {
    return
  }

  $destination = Join-Path $publishRoot $relative
  $destinationDir = Split-Path -Parent $destination
  if (-not (Test-Path -LiteralPath $destinationDir)) {
    New-Item -ItemType Directory -Path $destinationDir -Force | Out-Null
  }
  Copy-Item -LiteralPath $_.FullName -Destination $destination -Force
}

Push-Location $publishRoot
try {
  git init
  git add .
  git commit -m $CommitMessage
  git branch -M $Branch
  git remote add origin $RemoteUrl
  git push -u origin $Branch
} finally {
  Pop-Location
}

Write-Host "Published standalone Codex Account Switcher source from $publishRoot to $RemoteUrl"
