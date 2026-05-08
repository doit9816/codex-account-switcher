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

git clone --depth 1 --branch $Branch $RemoteUrl $publishRoot
if ($LASTEXITCODE -ne 0) {
  New-Item -ItemType Directory -Path $publishRoot | Out-Null
  git -C $publishRoot init
  git -C $publishRoot branch -M $Branch
  git -C $publishRoot remote add origin $RemoteUrl
}

Get-ChildItem -LiteralPath $publishRoot -Force | Where-Object { $_.Name -ne ".git" } | ForEach-Object {
  Remove-Item -LiteralPath $_.FullName -Recurse -Force
}

$excludedDirs = @(
  ".git",
  "node_modules",
  "dist",
  "src-tauri\target",
  "src-tauri\gen",
  ".cache",
  ".vite",
  "coverage",
  ".idea",
  ".vscode"
)

$excludedExtensions = @(".log", ".tmp", ".temp", ".pem", ".key", ".p12", ".pfx")
$excludedNames = @(".env", ".DS_Store", "Thumbs.db", "Desktop.ini")
$excludedPatterns = @(
  "npm-debug.log",
  "yarn-debug.log",
  "yarn-error.log",
  "pnpm-debug.log"
)

Get-ChildItem -LiteralPath $projectRoot -Recurse -Force -File | ForEach-Object {
  $file = $_
  $relative = $file.FullName.Substring($projectRoot.Path.Length).TrimStart("\", "/")
  $normalized = $relative -replace "/", "\"
  $isExcludedDir = $excludedDirs | Where-Object {
    $normalized.Equals($_, [System.StringComparison]::OrdinalIgnoreCase) -or
    $normalized.StartsWith("$($_)\", [System.StringComparison]::OrdinalIgnoreCase)
  }
  if ($isExcludedDir) {
    return
  }
  if ($excludedExtensions -contains $file.Extension.ToLowerInvariant() -or $excludedNames -contains $file.Name) {
    return
  }
  if ($file.Name.EndsWith(".zip", [System.StringComparison]::OrdinalIgnoreCase) -or $file.Name.EndsWith(".zip.enc", [System.StringComparison]::OrdinalIgnoreCase)) {
    return
  }
  $isExcludedPattern = $excludedPatterns | Where-Object {
    $file.Name.StartsWith($_, [System.StringComparison]::OrdinalIgnoreCase)
  }
  if ($isExcludedPattern) {
    return
  }

  $destination = Join-Path $publishRoot $relative
  $destinationDir = Split-Path -Parent $destination
  if (-not (Test-Path -LiteralPath $destinationDir)) {
    New-Item -ItemType Directory -Path $destinationDir -Force | Out-Null
  }
  Copy-Item -LiteralPath $file.FullName -Destination $destination -Force
}

Push-Location $publishRoot
try {
  git add .
  $changes = git status --porcelain
  if ($changes) {
    git commit -m $CommitMessage
    git push -u origin $Branch
  } else {
    Write-Host "No changes to publish."
  }
} finally {
  Pop-Location
}

Write-Host "Published standalone Codex Account Switcher source from $publishRoot to $RemoteUrl"
