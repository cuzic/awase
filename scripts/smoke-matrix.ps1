<#
.SYNOPSIS
    Semi-automated smoke-test matrix for awase (App x IME x Scenario).

.DESCRIPTION
    137+ warmup fixes have historically regressed *other* cells of the
    "app x IME x scenario" combination space (fixing Chrome cold-start broke
    WezTerm, fixing MS-IME broke GJI, etc.). This script enumerates that matrix
    so a human operator can sweep every cell before a version bump instead of
    spot-checking whichever app they happen to have open.

    Each cell is classified as:

      * Auto   - the target exposes a readable text field, so the script injects
                 a NICOLA key sequence via SendInput and reads the text back
                 (clipboard round-trip). PASS/FAIL is decided by string compare.
      * Manual - the result is only observable by eye (IME tray indicator,
                 GJI candidate window, katakana mode banner). The script prints
                 an instruction and prompts the operator for PASS/FAIL/SKIP.

    SAFETY: the default run mode is a DRY RUN. It prints the full plan and does
    NOT touch any window, send any key, or read any clipboard. You must pass
    -Execute to actually drive the desktop. Even then, injection only happens
    for cells marked Auto; Manual cells are always operator-driven.

    Apps and IMEs referenced here are the ones awase actually classifies and
    documents (docs/adr/033-app-ime-profile.md, docs/adr/043-app-delivery-profile.md,
    docs/known-bugs.md, crates/awase-windows/src/focus/class_names.rs).

.PARAMETER Execute
    Actually drive the desktop. Without this switch the script is a dry run that
    only prints the planned matrix. Requires awase to be running (it is the hook
    under test) and the target apps to be open.

.PARAMETER Apps
    Restrict to these app keys (chrome, edge, wezterm, notepad, line, vscode).
    Default: all.

.PARAMETER Imes
    Restrict to these IME keys (gji, msime). Default: all.

.PARAMETER Scenarios
    Restrict to these scenario keys (see $Scenarios below). Default: all.

.PARAMETER ResultsDir
    Where to write the Markdown + JSON result files. Default: logs/smoke/.

.EXAMPLE
    # Safe: print the matrix, touch nothing.
    .\scripts\smoke-matrix.ps1

.EXAMPLE
    # Full sweep on real hardware (awase + target apps must be running).
    .\scripts\smoke-matrix.ps1 -Execute

.EXAMPLE
    # Only re-check the Chrome/WezTerm cold-start cells after a warmup change.
    .\scripts\smoke-matrix.ps1 -Execute -Apps chrome,wezterm -Scenarios cold-start

.NOTES
    Intended execution path: this file is delivered to the Windows box and run
    there via the clipwire-exec / awase-build skills. See
    docs/smoke-testing-guide.md for the operating rules.
    Requires PowerShell 5.1+.
#>

param(
    [switch]$Execute,
    [string[]]$Apps      = @(),
    [string[]]$Imes      = @(),
    [string[]]$Scenarios = @(),
    [string]$ResultsDir  = ""
)

$ErrorActionPreference = "Stop"
$projectRoot = Split-Path $PSScriptRoot -Parent
if (-not $ResultsDir) { $ResultsDir = Join-Path $projectRoot "logs\smoke" }

# ─────────────────────────────────────────────────────────────
# Matrix definitions
#   Grounded in awase's own app classification. `Readable` = the target has a
#   text field whose contents can be read back (clipboard round-trip). WezTerm
#   is a terminal: literal-vs-composed is only reliably judged by eye, so it is
#   not Readable and its cells stay Manual.
# ─────────────────────────────────────────────────────────────
$Targets = @(
    [pscustomobject]@{ Key="chrome";  Name="Google Chrome";  Class="Chrome_WidgetWin_1";      Delivery="VkBatched (Imm32Unavailable)"; Readable=$true;  Notes="BUG-02 cold-start literal (toいう)" }
    [pscustomobject]@{ Key="edge";    Name="Microsoft Edge"; Class="Chrome_WidgetWin_1";      Delivery="VkBatched (Imm32Unavailable)"; Readable=$true;  Notes="Chromium, same delivery as Chrome" }
    [pscustomobject]@{ Key="wezterm"; Name="WezTerm";        Class="org.wezfurlong.wezterm";  Delivery="TSF native (himc_null)";       Readable=$false; Notes="BUG-01 cold-start literal (kあ…)" }
    [pscustomobject]@{ Key="notepad"; Name="メモ帳 (Notepad)"; Class="Notepad";               Delivery="IMM32 classic";                Readable=$true;  Notes="Win32 baseline, IMM32 cross-process OK" }
    [pscustomobject]@{ Key="line";    Name="LINE";           Class="Qt* (ImmCross)";          Delivery="ImmCross (VK_KANJI)";          Readable=$true;  Notes="ImmCross app; physical IME keys hidden" }
    [pscustomobject]@{ Key="vscode";  Name="Visual Studio Code"; Class="Chrome_WidgetWin_1";  Delivery="VkBatched (Imm32Unavailable)"; Readable=$true;  Notes="Electron/Chromium" }
)

$ImeDefs = @{
    gji   = [pscustomobject]@{ Key="gji";   Name="Google 日本語入力 (GJI)"; Notes="candidate window I/O monitored by GjiMonitor" }
    msime = [pscustomobject]@{ Key="msime"; Name="Microsoft IME";           Notes="TSF; MsImeDirectStrategy (ADR-063)" }
}

# AutoCapable = the scenario's outcome is expressible as read-back text.
# ExpectPhrase = canonical NICOLA input -> expected composed output.
$ScenarioDefs = @(
    [pscustomobject]@{ Key="cold-start";   Name="Cold start, 1st char"; AutoCapable=$true;
        Input="という"; Expect="という";
        Setup="Ensure the target has had NO input for >10s (long idle). Focus it, then type once." }
    [pscustomobject]@{ Key="alt-tab";      Name="Alt+Tab return";       AutoCapable=$true;
        Input="かんきょう"; Expect="かんきょう";
        Setup="Alt+Tab away and back to the target, then type immediately on return." }
    [pscustomobject]@{ Key="idle-30s";     Name="After 30s idle";       AutoCapable=$true;
        Input="にゅうりょく"; Expect="にゅうりょく";
        Setup="Leave the target focused and idle 30s (past SessionExpired 2000ms + long_idle), then type." }
    [pscustomobject]@{ Key="ctrl-muhenkan";Name="Ctrl+Muhenkan IME-OFF"; AutoCapable=$true;
        Input="abc"; Expect="abc";
        Setup="Press Ctrl+Muhenkan to force IME-OFF, then type ASCII; it must pass through literally." }
    [pscustomobject]@{ Key="katakana";     Name="Katakana toggle";      AutoCapable=$true;
        Input="カタカナ"; Expect="カタカナ";
        Setup="Switch to katakana mode, type, and confirm katakana output." }
    [pscustomobject]@{ Key="tray-toggle";  Name="Tray IME switch";      AutoCapable=$false;
        Input=""; Expect="";
        Setup="Switch IME on/off from the language-bar / tray by mouse, then confirm the mode indicator and next keystroke behave." }
)

# ─────────────────────────────────────────────────────────────
# Win32 interop (only used under -Execute)
#   Physical scancode injection so keys traverse the awase low-level hook,
#   mirroring the SendInput helper in crates/awase-windows/tests/e2e_windows.rs.
# ─────────────────────────────────────────────────────────────
function Initialize-Interop {
    if ("AwaseSmoke.Native" -as [type]) { return }
    Add-Type -Namespace AwaseSmoke -Name Native -MemberDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Text;

[StructLayout(LayoutKind.Sequential)]
public struct INPUT { public uint type; public KEYBDINPUT ki; }

[StructLayout(LayoutKind.Sequential)]
public struct KEYBDINPUT {
    public ushort wVk; public ushort wScan; public uint dwFlags;
    public uint time; public IntPtr dwExtraInfo;
    // pad so managed struct >= native INPUT (mouse variant)
    public int pad0; public int pad1;
}

[DllImport("user32.dll", SetLastError=true)]
public static extern uint SendInput(uint nInputs, INPUT[] pInputs, int cbSize);

[DllImport("user32.dll")]
public static extern IntPtr GetForegroundWindow();

[DllImport("user32.dll", CharSet=CharSet.Unicode)]
public static extern int GetClassName(IntPtr hWnd, StringBuilder buf, int max);

[DllImport("user32.dll")]
public static extern bool SetForegroundWindow(IntPtr hWnd);
"@
}

# ─────────────────────────────────────────────────────────────
# Matrix builder
# ─────────────────────────────────────────────────────────────
function Build-Matrix {
    $appFilter = if ($Apps.Count)      { $Apps }      else { $Targets.Key }
    $imeKeys   = if ($Imes.Count)      { $Imes }      else { @("gji","msime") }
    $scnFilter = if ($Scenarios.Count) { $Scenarios } else { $ScenarioDefs.Key }

    $cells = New-Object System.Collections.Generic.List[object]
    foreach ($app in ($Targets | Where-Object { $appFilter -contains $_.Key })) {
        foreach ($imeKey in $imeKeys) {
            if (-not $script:ImeTable.ContainsKey($imeKey)) { continue }
            $ime = $script:ImeTable[$imeKey]
            foreach ($scn in ($ScenarioDefs | Where-Object { $scnFilter -contains $_.Key })) {
                $auto = $scn.AutoCapable -and $app.Readable
                $cells.Add([pscustomobject]@{
                    App        = $app
                    Ime        = $ime
                    Scenario   = $scn
                    VerifyMode = if ($auto) { "Auto" } else { "Manual" }
                    Result     = "PENDING"
                    Actual     = ""
                    Note       = ""
                })
            }
        }
    }
    return $cells
}

# ─────────────────────────────────────────────────────────────
# Cell execution (only reached under -Execute)
# ─────────────────────────────────────────────────────────────
function Get-ForegroundClass {
    $sb = New-Object System.Text.StringBuilder 256
    $hwnd = [AwaseSmoke.Native]::GetForegroundWindow()
    [void][AwaseSmoke.Native]::GetClassName($hwnd, $sb, $sb.Capacity)
    return $sb.ToString()
}

function Invoke-AutoCell {
    param($Cell)

    Write-Host ("  [Auto] focus {0} and inject: {1}" -f $Cell.App.Name, $Cell.Scenario.Input) -ForegroundColor DarkGray
    Write-Host ("         setup: {0}" -f $Cell.Scenario.Setup) -ForegroundColor DarkGray

    # Confirm the operator has the correct window focused and IME selected.
    $fg = Get-ForegroundClass
    Write-Host ("         foreground class = {0} (expected {1})" -f $fg, $Cell.App.Class) -ForegroundColor DarkGray

    # NOTE: the physical NICOLA key sequence for `$Cell.Scenario.Input` is
    # produced by Send-NicolaSequence, which maps the target phrase to scancode
    # up/down pairs (with thumb-shift simultaneity) and injects them via
    # SendInput so they pass through the awase hook. Read-back then compares.
    #
    # This scaffold deliberately leaves the per-phrase scancode tables and the
    # clipboard round-trip to be filled in against layout/nicola.yab on the
    # Windows box, because they cannot be validated off-hardware. Until then an
    # Auto cell falls through to an operator confirmation so no result is faked.
    Write-Host "         (auto injection scaffold not yet wired for this phrase)" -ForegroundColor Yellow
    return (Invoke-ManualCell -Cell $Cell -AutoFallback)
}

function Invoke-ManualCell {
    param($Cell, [switch]$AutoFallback)

    Write-Host ""
    Write-Host ("  MANUAL CHECK - {0} / {1} / {2}" -f $Cell.App.Name, $Cell.Ime.Name, $Cell.Scenario.Name) -ForegroundColor Cyan
    Write-Host ("    Steps : {0}" -f $Cell.Scenario.Setup)
    if ($Cell.Scenario.Input) {
        Write-Host ("    Type  : {0}" -f $Cell.Scenario.Input)
        Write-Host ("    Expect: {0}" -f $Cell.Scenario.Expect)
    }
    if ($AutoFallback) {
        Write-Host "    (this cell is Auto-capable but injection is not wired; judge by eye)" -ForegroundColor Yellow
    }
    do {
        $ans = Read-Host "    Result [p]ass / [f]ail / [s]kip"
    } while ($ans -notmatch '^[pfsPFS]')

    switch ($ans.ToLower()[0]) {
        'p' { $Cell.Result = "PASS" }
        'f' { $Cell.Result = "FAIL"; $Cell.Note = Read-Host "    Note (what went wrong)" }
        's' { $Cell.Result = "SKIP"; $Cell.Note = Read-Host "    Reason skipped (optional)" }
    }
    return $Cell
}

# ─────────────────────────────────────────────────────────────
# Report writers
# ─────────────────────────────────────────────────────────────
function Write-Report {
    param($Cells, $Stamp)

    if (-not (Test-Path $ResultsDir)) { New-Item -ItemType Directory -Path $ResultsDir | Out-Null }
    $mdPath   = Join-Path $ResultsDir "smoke_${Stamp}.md"
    $jsonPath = Join-Path $ResultsDir "smoke_${Stamp}.json"

    $sb = New-Object System.Text.StringBuilder
    [void]$sb.AppendLine("# awase smoke matrix - $Stamp")
    [void]$sb.AppendLine("")
    [void]$sb.AppendLine("| App | IME | Scenario | Verify | Result | Note |")
    [void]$sb.AppendLine("|---|---|---|---|---|---|")
    foreach ($c in $Cells) {
        [void]$sb.AppendLine(("| {0} | {1} | {2} | {3} | {4} | {5} |" -f `
            $c.App.Name, $c.Ime.Name, $c.Scenario.Name, $c.VerifyMode, $c.Result, ($c.Note -replace '\|','\\|')))
    }
    [void]$sb.AppendLine("")
    $pass = ($Cells | Where-Object { $_.Result -eq "PASS" }).Count
    $fail = ($Cells | Where-Object { $_.Result -eq "FAIL" }).Count
    $skip = ($Cells | Where-Object { $_.Result -eq "SKIP" }).Count
    [void]$sb.AppendLine(("**Totals:** {0} cells - PASS {1} / FAIL {2} / SKIP {3}" -f $Cells.Count, $pass, $fail, $skip))
    $sb.ToString() | Out-File -FilePath $mdPath -Encoding utf8

    $Cells | ForEach-Object {
        [pscustomobject]@{
            app=$_.App.Key; ime=$_.Ime.Key; scenario=$_.Scenario.Key
            verify=$_.VerifyMode; result=$_.Result; note=$_.Note
        }
    } | ConvertTo-Json -Depth 4 | Out-File -FilePath $jsonPath -Encoding utf8

    Write-Host ""
    Write-Host "Report written:" -ForegroundColor Green
    Write-Host "  $mdPath"
    Write-Host "  $jsonPath"
}

# ─────────────────────────────────────────────────────────────
# Main
# ─────────────────────────────────────────────────────────────
$script:ImeTable = @{}
foreach ($k in $ImeDefs.Keys) { $script:ImeTable[$k] = $ImeDefs[$k] }

$cells = Build-Matrix

Write-Host "=== awase smoke matrix ===" -ForegroundColor Cyan
Write-Host ("Project : {0}" -f $projectRoot)
Write-Host ("Cells   : {0}" -f $cells.Count)
$autoN   = ($cells | Where-Object { $_.VerifyMode -eq "Auto" }).Count
$manualN = ($cells | Where-Object { $_.VerifyMode -eq "Manual" }).Count
Write-Host ("Auto    : {0}   Manual: {1}" -f $autoN, $manualN)
Write-Host ("Mode    : {0}" -f $(if ($Execute) { "EXECUTE (drives the desktop)" } else { "DRY RUN (prints plan only)" })) -ForegroundColor $(if ($Execute) { "Yellow" } else { "Green" })
Write-Host ""

if (-not $Execute) {
    Write-Host "Planned matrix (no keys sent, no windows touched):" -ForegroundColor Green
    $cells | Format-Table `
        @{L="App";      E={$_.App.Name}}, `
        @{L="IME";      E={$_.Ime.Name}}, `
        @{L="Scenario"; E={$_.Scenario.Name}}, `
        @{L="Verify";   E={$_.VerifyMode}} -AutoSize
    Write-Host "Re-run with -Execute on the Windows box (awase + target apps running) to sweep." -ForegroundColor Yellow
    return
}

# --- EXECUTE path (real hardware only) ---
Initialize-Interop
Write-Host "Pre-flight: awase must be running (it is the hook under test) and every" -ForegroundColor Yellow
Write-Host "target app open with the intended IME selected. Ctrl+C to abort." -ForegroundColor Yellow
$null = Read-Host "Press Enter when ready"

$i = 0
foreach ($cell in $cells) {
    $i++
    Write-Host ("[{0}/{1}] {2} / {3} / {4}" -f $i, $cells.Count, $cell.App.Name, $cell.Ime.Name, $cell.Scenario.Name) -ForegroundColor White
    if ($cell.VerifyMode -eq "Auto") {
        [void](Invoke-AutoCell -Cell $cell)
    } else {
        [void](Invoke-ManualCell -Cell $cell)
    }
}

$stamp = Get-Date -Format "yyyyMMdd_HHmmss"
Write-Report -Cells $cells -Stamp $stamp
