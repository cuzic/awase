<#
.SYNOPSIS
    Run awase E2E tests on local Windows and save logs.

.DESCRIPTION
    Phase 1: Engine in-process tests (same as CI)
    Phase 2: SendMessage + Edit control tests (same as CI)
    Phase 2i: SendInput interactive tests (local only)
    Phase 3: IME + NICOLA tests (local only, requires Japanese IME)

.EXAMPLE
    # Run all tests (including interactive)
    .\scripts\run-e2e.ps1

    # CI-compatible tests only (skip interactive)
    .\scripts\run-e2e.ps1 -CIOnly

    # Run specific tests
    .\scripts\run-e2e.ps1 -Filter "e2e_message"

.NOTES
    Logs are saved to logs/e2e_YYYYMMDD_HHmmss.log.
    Share the log file for remote debugging.
    Requires PowerShell 5.1+.
#>

param(
    [switch]$CIOnly,
    [string]$Filter = ""
)

$ErrorActionPreference = "Continue"

# Check cargo is available
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host "ERROR: cargo not found. Please install Rust: https://rustup.rs/" -ForegroundColor Red
    exit 1
}

# Project root (parent of scripts/)
$projectRoot = Split-Path $PSScriptRoot -Parent

# Log directory
$logDir = Join-Path $projectRoot "logs"
if (-not (Test-Path $logDir)) {
    New-Item -ItemType Directory -Path $logDir | Out-Null
}

$timestamp = Get-Date -Format "yyyyMMdd_HHmmss"
$logFile = Join-Path $logDir "e2e_${timestamp}.log"

Write-Host "=== awase E2E Test Runner ===" -ForegroundColor Cyan
Write-Host "Project: $projectRoot"
Write-Host "Log file: $logFile"
Write-Host ""

# Save original env vars
$origRustLog = $env:RUST_LOG
$origInteractive = $env:AWASE_E2E_INTERACTIVE

# Set test environment
$env:RUST_LOG = "debug"

if (-not $CIOnly) {
    $env:AWASE_E2E_INTERACTIVE = "1"
    Write-Host "Mode: Full (interactive tests enabled)" -ForegroundColor Green
} else {
    $env:AWASE_E2E_INTERACTIVE = ""
    Write-Host "Mode: CI-only (interactive tests skipped)" -ForegroundColor Yellow
}

# Collect system info (PS 5.1 compatible)
$langInfo = "N/A"
$imeInfo = "N/A"
try {
    $langs = Get-WinUserLanguageList -ErrorAction Stop
    $langInfo = ($langs | ForEach-Object { $_.LanguageTag }) -join ", "
    $imeInfo = if ($langs | Where-Object { $_.LanguageTag -eq "ja-JP" }) { "Yes" } else { "No" }
} catch {
    # Get-WinUserLanguageList may not be available
}

$sysInfo = @"
=== System Info ===
Date: $(Get-Date -Format "yyyy-MM-dd HH:mm:ss")
OS: $([System.Environment]::OSVersion.VersionString)
User: $env:USERNAME
PowerShell: $($PSVersionTable.PSVersion)
Rust: $(rustc --version 2>&1)
Cargo: $(cargo --version 2>&1)
Keyboard Layouts: $langInfo
Japanese IME: $imeInfo
Interactive: $(-not $CIOnly)
Project Root: $projectRoot
===================

"@
$sysInfo | Tee-Object -FilePath $logFile

# Build
Write-Host "Building tests..." -ForegroundColor Cyan
Push-Location $projectRoot
try {
    $buildOutput = cargo test --test e2e_windows --no-run 2>&1
    $buildOutput | Out-File -Append -FilePath $logFile -Encoding utf8
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Build FAILED!" -ForegroundColor Red
        Write-Host ($buildOutput -join "`n")
        Write-Host ""
        Write-Host "Log saved to: $logFile" -ForegroundColor Yellow
        exit 1
    }
    Write-Host "Build OK" -ForegroundColor Green

    # Run tests
    Write-Host ""
    Write-Host "Running E2E tests..." -ForegroundColor Cyan

    if ($Filter) {
        Write-Host "Filter: $Filter" -ForegroundColor Yellow
        $testArgs = @("test", "--test", "e2e_windows", "--", "--nocapture", $Filter)
    } else {
        $testArgs = @("test", "--test", "e2e_windows", "--", "--nocapture")
    }

    & cargo @testArgs 2>&1 | Tee-Object -Append -FilePath $logFile

    $exitCode = $LASTEXITCODE
} finally {
    Pop-Location

    # Restore env vars
    $env:RUST_LOG = $origRustLog
    $env:AWASE_E2E_INTERACTIVE = $origInteractive
}

# Results
Write-Host ""
Write-Host "=== Results ===" -ForegroundColor Cyan
if ($exitCode -eq 0) {
    Write-Host "ALL TESTS PASSED" -ForegroundColor Green
} else {
    Write-Host "SOME TESTS FAILED (exit code: $exitCode)" -ForegroundColor Red
}

Write-Host ""
Write-Host "Log saved to: $logFile" -ForegroundColor Yellow
Write-Host "Share this file for remote debugging." -ForegroundColor Yellow

exit $exitCode
