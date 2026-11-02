$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$tauriRoot = Join-Path $repoRoot "src-tauri"

Write-Host "Running isolated two-node EasyTier file sharing and MCP overlay test..."
Write-Host "The test creates temporary nodes, a temporary share, and test-only credentials."
Write-Host "No installed app settings or user credentials are used."

Push-Location $tauriRoot
try {
    cargo test two_embedded_easytier_nodes_transfer_a_file_over_the_overlay -- --nocapture
    if ($LASTEXITCODE -ne 0) {
        throw "The EasyTier file sharing and MCP overlay test failed (exit $LASTEXITCODE)."
    }
} finally {
    Pop-Location
}

Write-Host "PASS: HTTP file listing/download and MCP initialize/tools/list/read_shared_file worked between separate EasyTier nodes."
