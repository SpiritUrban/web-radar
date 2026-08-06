# Web Radar — one-command runner for Windows.
#
#   .\run.ps1                     index status (what exists, what it would cost)
#   .\run.ps1 -Build              build every index tier
#   .\run.ps1 -Build lookup       build one tier: lookup | ranks | inbound
#   .\run.ps1 -Query example.com  ask about one domain
#   .\run.ps1 -Scan               full streaming scan of config.toml targets
#   .\run.ps1 -Demo               tiny fixture, no multi-GB downloads
#   .\run.ps1 -Open               open the results folder afterwards

param(
    [string[]]$Build,
    [string]$Query,
    [switch]$Scan,
    [switch]$Demo,
    [switch]$Open,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$config = if ($Demo) { "testdata\config.toml" } else { "config.toml" }

Write-Host ""
Write-Host "=== Web Radar ===" -ForegroundColor Cyan
Write-Host "Project: $PSScriptRoot"
Write-Host "Config:  $config"
Write-Host ""

if (-not $SkipBuild) {
    Write-Host "Building release binary (first time takes a few minutes)..." -ForegroundColor Yellow
    cargo build --release -p web-radar
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Write-Host ""
}

$exe = Join-Path $PSScriptRoot "target\release\web-radar.exe"
if (-not (Test-Path $exe)) {
    Write-Host "ERROR: binary not found at $exe" -ForegroundColor Red
    Write-Host "Run: cargo build --release -p web-radar"
    exit 1
}

$arguments = @("-c", $config)
if ($Query) {
    $arguments += @("query", $Query)
} elseif ($Scan) {
    $arguments += "run"
} elseif ($PSBoundParameters.ContainsKey("Build")) {
    $arguments += @("index", "build")
    if ($Build) { $arguments += $Build }
} else {
    $arguments += @("index", "status")
}

& $exe @arguments
$code = $LASTEXITCODE

Write-Host ""
if ($code -ne 0) {
    Write-Host "FAILED (exit $code)." -ForegroundColor Red
    Write-Host "No graph files yet? Download them from https://commoncrawl.org/web-graphs" -ForegroundColor Yellow
    Write-Host "Or try the demo fixture:  .\run.ps1 -Demo -Build" -ForegroundColor Yellow
    exit $code
}

if ($Open) {
    $results = Join-Path $PSScriptRoot (if ($Demo) { "testdata\results" } else { "results" })
    if (Test-Path $results) { Invoke-Item $results }
}

exit 0
