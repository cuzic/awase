# X5 / D1 real-device runner (clipwire exec). ASCII only (PowerShell 5 reads BOM-less files as ANSI).
# Stops the owner's awase, runs a test awase (debug log + test injection) from a separate worktree,
# runs chrome_probe with the given args, then ALWAYS restores the owner's awase.
# Refuses to start unless the keyboard/mouse has been idle for MinIdleMs.
param(
  [string]$Name = 'run',
  [string]$ProbeArgs = '--close-ime=10 --close-key=1A --settle=800',
  [int]$MinIdleMs = 300000,
  [string]$OwnerDir = 'C:/Users/cuzic/scoop/persist/msys2/home/cuzic/awase',
  [string]$TestDir = 'C:/Users/cuzic/awase-dv',
  [string]$Out = 'C:/Users/cuzic/dv-out',
  [switch]$NoAwase
)
Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public class Idle2{[StructLayout(LayoutKind.Sequential)]public struct LII{public uint cb;public uint t;}[DllImport("user32.dll")]public static extern bool GetLastInputInfo(ref LII p);public static uint Ms(){LII l=new LII();l.cb=8;GetLastInputInfo(ref l);return (uint)Environment.TickCount-l.t;}}'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
New-Item -ItemType Directory -Force $Out | Out-Null
Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public class Lk{[DllImport("user32.dll")]public static extern IntPtr GetForegroundWindow();[DllImport("user32.dll")]public static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);public static uint FgPid(){uint p;GetWindowThreadProcessId(GetForegroundWindow(),out p);return p;}}'
function Test-Locked { $p = [Lk]::FgPid(); if ($p -eq 0) { return $true }; $n = (Get-Process -Id $p -ErrorAction SilentlyContinue).ProcessName; return ($n -eq 'LockApp' -or $n -eq 'LogonUI') }
if (Test-Locked) { "SKIP screen is locked (LockApp/LogonUI is foreground): injected keys cannot reach apps"; exit 0 }
$idle = [Idle2]::Ms()
if ($idle -lt $MinIdleMs) { "SKIP idle_ms=$idle < $MinIdleMs (owner is using the machine)"; exit 0 }
$owner = Get-CimInstance Win32_Process -Filter "Name='awase.exe'" | Where-Object { $_.ExecutablePath -notlike '*awase-dv*' }
"start idle_ms=$idle owner_pids=$(($owner | % ProcessId) -join ',')"
$job = $null
try {
  $owner | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
  Start-Sleep 1
  $aw = $null
  if (-not $NoAwase) {
    Remove-Item "$TestDir/target/debug/awase.log" -ErrorAction SilentlyContinue
    $env:RUST_LOG = 'debug'; $env:AWASE_TEST_INJECTION = '1'
    $aw = Start-Process -FilePath "$TestDir/target/debug/awase.exe" -WorkingDirectory $TestDir -PassThru
    Remove-Item Env:RUST_LOG; Remove-Item Env:AWASE_TEST_INJECTION
    Start-Sleep 4
    "test awase pid=$($aw.Id)"
  }
  $pargs = $ProbeArgs -split ' '
  $probeLog = "$Out/$Name-probe.log"
  $pr = Start-Process -FilePath "$TestDir/target/debug/examples/chrome_probe.exe" -ArgumentList ($pargs + "--log=$probeLog") -PassThru -WindowStyle Hidden -RedirectStandardOutput "$Out/$Name-probe.stdout"
  $job = Start-Job -ScriptBlock { param($ppid) $w = New-Object -ComObject WScript.Shell; while (Get-Process -Id $ppid -ErrorAction SilentlyContinue) { $null = $w.AppActivate('IMEPROBE'); Start-Sleep -Milliseconds 600 } } -ArgumentList $pr.Id
  $pr.WaitForExit(240000) | Out-Null
  if (-not $pr.HasExited) { Stop-Process -Id $pr.Id -Force; "probe timeout" }
}
finally {
  if ($job) { Stop-Job $job -ErrorAction SilentlyContinue; Remove-Job $job -Force -ErrorAction SilentlyContinue }
  Get-Process chrome_probe -ErrorAction SilentlyContinue | Stop-Process -Force
  Get-CimInstance Win32_Process -Filter "Name='chrome.exe'" | Where-Object { $_.CommandLine -like '*chrome_probe_profile*' } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
  Start-Sleep 1
  if ($aw) {
    Stop-Process -Id $aw.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep 1
    Copy-Item "$TestDir/target/debug/awase.log" "$Out/$Name-awase.log" -Force -ErrorAction SilentlyContinue
  }
  Start-Process -FilePath "$OwnerDir/target/debug/awase.exe" -WorkingDirectory $OwnerDir
  Start-Sleep 3
  "restored owner awase: " + ((Get-CimInstance Win32_Process -Filter "Name='awase.exe'" | % { "$($_.ProcessId) $($_.ExecutablePath)" }) -join ' ; ')
}
Get-Content "$Out/$Name-probe.log" -ErrorAction SilentlyContinue | Select-String 'RESULT|SUMMARY|CLOSE_KEY|PROBE|THEN' | ForEach-Object { $_.Line }
