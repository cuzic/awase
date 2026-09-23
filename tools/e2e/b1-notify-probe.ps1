# B-1 real-machine check: WM_IME_NOTIFY must be observed by the learner window (EDIT subclass).
param([string]$Exe = "target\debug\examples\notify_probe.exe")
$ErrorActionPreference = "Continue"
Add-Type @"
using System; using System.Runtime.InteropServices;
public class Ime {
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string t);
  [DllImport("imm32.dll")] public static extern IntPtr ImmGetDefaultIMEWnd(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
}
"@
$out = "$env:TEMP\b1-probe-stdout.log"; Remove-Item $out -ErrorAction SilentlyContinue
$dir = Split-Path -Parent (Resolve-Path $Exe)
Remove-Item (Join-Path $dir "notify-probe.stop") -ErrorAction SilentlyContinue
$p = Start-Process -FilePath (Resolve-Path $Exe) -WorkingDirectory $dir -PassThru -RedirectStandardOutput $out -WindowStyle Normal
$null = $p.Handle
$ready = $false
for ($i = 0; $i -lt 240 -and -not $ready; $i++) {
    Start-Sleep -Milliseconds 500
    $ready = (Get-Content $out -ErrorAction SilentlyContinue) -contains "WAIT_EXTERNAL"
    if ($p.HasExited) { break }
}
"ready=$ready"
$p.Refresh()
$hwnd = $p.MainWindowHandle
if ($hwnd -eq [IntPtr]::Zero) { $hwnd = [Ime]::FindWindow("AwaseKeymapLearnWindow", [NullString]::Value) }
$ime = [Ime]::ImmGetDefaultIMEWnd($hwnd)
"hwnd=$hwnd imeWnd=$ime"
# WM_IME_CONTROL(0x283) IMC_SETOPENSTATUS(6): toggle from a different process
foreach ($v in 1, 0, 1, 0) { [void][Ime]::SendMessage($ime, 0x283, [IntPtr]6, [IntPtr]$v); Start-Sleep -Milliseconds 600 }
# physical-style keys from another process (no SELF_MARKER): informational
Add-Type -Name K -Namespace W -MemberDefinition '[DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);'
foreach ($vk in 0xF3, 0xF4, 0xF3, 0xF4) { [W.K]::keybd_event($vk, 0, 0, [UIntPtr]::Zero); [W.K]::keybd_event($vk, 0, 2, [UIntPtr]::Zero); Start-Sleep -Milliseconds 600 }
Start-Sleep -Seconds 2
New-Item -ItemType File -Force -Path (Join-Path $dir "notify-probe.stop") | Out-Null
[void]$p.WaitForExit(20000)
"--- probe stdout ---"; Get-Content $out
if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
$lines = Get-Content $out
$phase1 = if (($lines -join "`n") -match 'PHASE1 notify_external=(\d+)') { [int]$Matches[1] } else { -1 }
$inproc = ($lines | Where-Object { $_ -like 'INPROC_TOGGLE*' })
$inprocDetected = ($inproc | Where-Object { $_ -match 'delta=([1-9]\d*)' }).Count
$phase2 = if (($lines -join "`n") -match 'PHASE2 notify_external=(\d+)') { [int]$Matches[1] } else { -1 }
"phase1(self-injection false positives)=$phase1 phase2(external detections)=$phase2"
$fails = @()
if (-not $ready) { $fails += 'probe never reached WAIT_EXTERNAL' }
"in-process toggles detected: $inprocDetected / $($inproc.Count) (cross-process phase2=$phase2 is informational)"
if ($inprocDetected -le 0) { $fails += 'in-process ImmSetOpenStatus (outside expect window) was NOT detected via WM_IME_NOTIFY' }
if ($phase1 -gt 0) { $fails += "self injection produced $phase1 false external notifications (expect window too short?)" }
if ($fails.Count -gt 0) { $fails | ForEach-Object { "FAIL: $_" }; exit 1 }
"PASS"
