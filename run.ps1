# web-radar — one-command runner (like `npm start`)
#
# Usage:
#   .\run.ps1              # build + run with config.toml
#   .\run.ps1 -Demo        # tiny fixture, no multi-GB files needed
#   .\run.ps1 -Open        # open results folder in Explorer after run
#   .\run.ps1 -Demo -Open

param(
    [switch]$Demo,
    [switch]$Open,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$config = if ($Demo) { "testdata\config.toml" } else { "config.toml" }
$resultsHint = if ($Demo) { "testdata\results" } else { "results" }

Write-Host ""
Write-Host "=== web-radar ===" -ForegroundColor Cyan
Write-Host "Project:  $PSScriptRoot"
Write-Host "Config:   $config"
Write-Host ""

if (-not $SkipBuild) {
    Write-Host "Building release (first time can take a few minutes)..." -ForegroundColor Yellow
    cargo build --release
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Write-Host ""
}

$exe = Join-Path $PSScriptRoot "target\release\web-radar.exe"
if (-not (Test-Path $exe)) {
    Write-Host "ERROR: binary not found at $exe" -ForegroundColor Red
    Write-Host "Run: cargo build --release"
    exit 1
}

Write-Host "Running..." -ForegroundColor Yellow
& $exe -c $config
$code = $LASTEXITCODE

Write-Host ""
if ($code -eq 0) {
    $absResults = Join-Path $PSScriptRoot $resultsHint
    Write-Host "OK. Results are here:" -ForegroundColor Green
    Write-Host "  $absResults" -ForegroundColor Green
    if (Test-Path $absResults) {
        Get-ChildItem $absResults -Filter *.json | ForEach-Object {
            Write-Host ("  - " + $_.FullName + "  (" + $_.Length + " bytes)")
        }
    }
    if ($Open -and (Test-Path $absResults)) {
        Invoke-Item $absResults
    }
} else {
    Write-Host "FAILED (exit $code)." -ForegroundColor Red
    Write-Host "If files are missing, download domain edges+ranks from:" -ForegroundColor Yellow
    Write-Host "  https://commoncrawl.org/web-graphs"
    Write-Host "Or try the demo:  .\run.ps1 -Demo -Open"
}

exit $code
