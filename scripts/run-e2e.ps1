<#
.SYNOPSIS
    awase E2E テストをローカル Windows で実行し、ログを保存する。

.DESCRIPTION
    Phase 1: Engine in-process テスト（CI と同じ）
    Phase 2: SendMessage + Edit コントロールテスト（CI と同じ）
    Phase 2i: SendInput インタラクティブテスト（ローカルのみ）
    Phase 3: IME + NICOLA テスト（ローカルのみ、日本語 IME 必要）

.EXAMPLE
    # 全テスト実行（Phase 2i + Phase 3 含む）
    .\scripts\run-e2e.ps1

    # CI と同じテストのみ（インタラクティブなし）
    .\scripts\run-e2e.ps1 -CIOnly

    # 特定のテストだけ実行
    .\scripts\run-e2e.ps1 -Filter "e2e_message"

.NOTES
    ログは logs/e2e_YYYYMMDD_HHmmss.log に保存される。
    デバッグ時はこのログファイルを共有すること。
    PowerShell 5.1 以上で動作。
#>

param(
    [switch]$CIOnly,
    [string]$Filter = ""
)

$ErrorActionPreference = "Continue"

# cargo が使えるか確認
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host "ERROR: cargo が見つかりません。Rust をインストールしてください。" -ForegroundColor Red
    Write-Host "https://rustup.rs/"
    exit 1
}

# プロジェクトルートを基準にする
$projectRoot = Split-Path $PSScriptRoot -Parent

# ログディレクトリ
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

# 元の環境変数を保存
$origRustLog = $env:RUST_LOG
$origInteractive = $env:AWASE_E2E_INTERACTIVE

# 環境情報
$env:RUST_LOG = "debug"

if (-not $CIOnly) {
    $env:AWASE_E2E_INTERACTIVE = "1"
    Write-Host "Mode: Full (interactive tests enabled)" -ForegroundColor Green
} else {
    $env:AWASE_E2E_INTERACTIVE = ""
    Write-Host "Mode: CI-only (interactive tests skipped)" -ForegroundColor Yellow
}

# システム情報を収集（PowerShell 5.1 互換）
$langInfo = "N/A"
$imeInfo = "N/A"
try {
    $langs = Get-WinUserLanguageList -ErrorAction Stop
    $langInfo = ($langs | ForEach-Object { $_.LanguageTag }) -join ", "
    $imeInfo = if ($langs | Where-Object { $_.LanguageTag -eq "ja-JP" }) { "Yes" } else { "No" }
} catch {
    # Get-WinUserLanguageList が利用できない環境
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
IME Installed: $imeInfo
Interactive: $(-not $CIOnly)
Project Root: $projectRoot
===================

"@
$sysInfo | Tee-Object -FilePath $logFile

# ビルド
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

    # テスト実行
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

    # 環境変数を復元
    $env:RUST_LOG = $origRustLog
    $env:AWASE_E2E_INTERACTIVE = $origInteractive
}

# 結果表示
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
