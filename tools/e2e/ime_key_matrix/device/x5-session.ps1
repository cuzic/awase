# Waits until the machine has been idle for IdleMs (polling up to WaitMin minutes), then runs the X5 scenarios back to back.
# Progress goes to Out/session.status. ASCII only.
param([int]$IdleMs = 60000, [int]$WaitMin = 600, [string]$Out = 'C:/Users/cuzic/dv-out')
Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public class Idle3{[StructLayout(LayoutKind.Sequential)]public struct LII{public uint cb;public uint t;}[DllImport("user32.dll")]public static extern bool GetLastInputInfo(ref LII p);public static uint Ms(){LII l=new LII();l.cb=8;GetLastInputInfo(ref l);return (uint)Environment.TickCount-l.t;}}'
Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public class Es{[DllImport("kernel32.dll")]public static extern uint SetThreadExecutionState(uint f);}public class Lk3{[DllImport("user32.dll")]public static extern IntPtr GetForegroundWindow();[DllImport("user32.dll")]public static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);public static uint FgPid(){uint p;GetWindowThreadProcessId(GetForegroundWindow(),out p);return p;}}'
# keep the display awake while this process lives (not a persistent setting; released on exit)
[void][Es]::SetThreadExecutionState(0x80000000 -bor 0x00000002 -bor 0x00000001)
function Test-Locked { $p = [Lk3]::FgPid(); if ($p -eq 0) { return $true }; $n = (Get-Process -Id $p -ErrorAction SilentlyContinue).ProcessName; return ($n -eq 'LockApp' -or $n -eq 'LogonUI') }
New-Item -ItemType Directory -Force $Out | Out-Null
$st = "$Out/session.status"
$env:X5_SESSION = '1'
"waiting since $(Get-Date -Format o)" | Set-Content $st
$deadline = (Get-Date).AddMinutes($WaitMin)
while ((Test-Locked) -or ([Idle3]::Ms() -lt $IdleMs)) {
  if ((Get-Date) -gt $deadline) { "gave up (owner never idle) $(Get-Date -Format o)" | Add-Content $st; exit 0 }
  Start-Sleep 10
}
"start $(Get-Date -Format o) idle_ms=$([Idle3]::Ms()) locked=$(Test-Locked)" | Add-Content $st
$runs = @(
  @('x5-1a', '--close-ime=10 --close-key=1A --then-chord=A2,1C --settle=800'),
  @('x5-f3', '--close-ime=10 --close-key=F3 --then-chord=A2,1C --settle=800'),
  @('x5-wm', '--close-ime=10 --settle=800')
)
foreach ($r in $runs) {
  if (Test-Locked) { "aborted before $($r[0]): screen locked $(Get-Date -Format o)" | Add-Content $st; break }
  "run $($r[0]) $(Get-Date -Format o)" | Add-Content $st
  & powershell -NonInteractive -ExecutionPolicy Bypass -File C:/Users/cuzic/dv-x5.ps1 -Name $r[0] -ProbeArgs $r[1] -MinIdleMs 0 *> "$Out/$($r[0])-runner.txt"
}
"done $(Get-Date -Format o)" | Add-Content $st
