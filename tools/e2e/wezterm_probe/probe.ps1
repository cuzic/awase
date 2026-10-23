# Verification-only (ci/ branch): can WezTerm portable start on a GitHub Windows runner, and what is its window class?
param([string]$FrontEnd = '')
$ErrorActionPreference = 'Continue'
& "$PSScriptRoot/../input_forms/setup/wezterm.ps1"
$exe = 'C:\wezterm\WezTerm-windows-20240203-110809-5046fc22\wezterm-gui.exe'
$env:WEZTERM_LOG = 'info'
$a = @()
if ($FrontEnd -ne '') { $a += @('--config', "front_end=`"$FrontEnd`"") }
$a += @('start', '--always-new-process', '--', 'powershell.exe', '-NoProfile')
Write-Host "args: $($a -join ' ')"
$p = Start-Process -FilePath $exe -ArgumentList $a -PassThru -RedirectStandardError "$PWD\wz-err.txt" -RedirectStandardOutput "$PWD\wz-out.txt"
Start-Sleep -Seconds 20
Write-Host "exited=$($p.HasExited) code=$(if ($p.HasExited) { $p.ExitCode })"
Add-Type @"
using System; using System.Text; using System.Runtime.InteropServices; using System.Collections.Generic;
public static class W {
  public delegate bool P(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(P f, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  public static List<string> All() { var r = new List<string>(); EnumWindows((h, l) => { if (IsWindowVisible(h)) { var c = new StringBuilder(256); var t = new StringBuilder(256); GetClassName(h, c, 256); GetWindowText(h, t, 256); uint pid; GetWindowThreadProcessId(h, out pid); r.Add(pid + " | " + c + " | " + t); } return true; }, IntPtr.Zero); return r; }
}
"@
[W]::All() | ForEach-Object { Write-Host "win: $_" }
Get-Process wezterm* -ErrorAction SilentlyContinue | Format-Table Name, Id, MainWindowTitle | Out-String | Write-Host
Write-Host '--- stderr ---'; Get-Content wz-err.txt -ErrorAction SilentlyContinue | Select-Object -First 60
Write-Host '--- stdout ---'; Get-Content wz-out.txt -ErrorAction SilentlyContinue | Select-Object -First 20
Get-ChildItem "$env:LOCALAPPDATA\wezterm" -Recurse -ErrorAction SilentlyContinue | ForEach-Object { Write-Host "log: $($_.FullName)"; Get-Content $_.FullName -ErrorAction SilentlyContinue | Select-Object -Last 30 }
