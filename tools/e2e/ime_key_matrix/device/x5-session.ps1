# Waits until the machine has been idle for IdleMs (polling up to WaitMin minutes), then runs the X5 scenarios back to back.
# Progress goes to Out/session.status. ASCII only.
param([int]$IdleMs = 10000, [int]$WaitMin = 600, [string]$Out = 'C:/Users/cuzic/dv-out')
Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public class Idle3{[StructLayout(LayoutKind.Sequential)]public struct LII{public uint cb;public uint t;}[DllImport("user32.dll")]public static extern bool GetLastInputInfo(ref LII p);public static uint Ms(){LII l=new LII();l.cb=8;GetLastInputInfo(ref l);return (uint)Environment.TickCount-l.t;}}'
Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public class Es{[DllImport("kernel32.dll")]public static extern uint SetThreadExecutionState(uint f);}public class Lk3{[DllImport("user32.dll")]public static extern IntPtr GetForegroundWindow();[DllImport("user32.dll")]public static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);[DllImport("user32.dll")]public static extern IntPtr OpenInputDesktop(uint f,bool i,uint a);[DllImport("user32.dll")]public static extern bool CloseDesktop(IntPtr h);public static bool InputDesktopOpen(){IntPtr h=OpenInputDesktop(0,false,1);if(h==IntPtr.Zero)return false;CloseDesktop(h);return true;}public static uint FgPid(){uint p;GetWindowThreadProcessId(GetForegroundWindow(),out p);return p;}}'
# keep the display awake while this process lives (not a persistent setting; released on exit)
[void][Es]::SetThreadExecutionState(0x80000000 -bor 0x00000002 -bor 0x00000001)
function Test-Locked { if (-not [Lk3]::InputDesktopOpen()) { return $true }; $p = [Lk3]::FgPid(); if ($p -eq 0) { return $true }; $n = (Get-Process -Id $p -ErrorAction SilentlyContinue).ProcessName; return ($n -eq 'LockApp' -or $n -eq 'LogonUI') }
New-Item -ItemType Directory -Force $Out | Out-Null
$st = "$Out/session.status"
$env:X5_SESSION = '1'
"waiting since $(Get-Date -Format o)" | Set-Content $st
$deadline = (Get-Date).AddMinutes($WaitMin)
$aw = 'C:/Users/cuzic/awase-dv/target/debug/awase.exe'
$cw = 'C:/Users/cuzic/awase-dv'
$runs = @(
  @('x5b-1a', '--close-ime=10 --close-key=1A --then-chord=A2,1C --settle=800', $false),
  @('x5b-f3', '--close-ime=10 --close-key=F3 --then-chord=A2,1C --settle=800', $false),
  @('d1-on', "--d1=6 --d1-state=on --d1-delay=800 --awase-exe=$aw --awase-cwd=$cw", $true),
  @('d1-off', "--d1=6 --d1-state=off --d1-delay=800 --awase-exe=$aw --awase-cwd=$cw", $true)
)
$next = 0
while ($next -lt $runs.Count) {
  # wait until the screen has stayed unlocked for 15 consecutive seconds
  $ok = 0
  while ($ok -lt 15) {
    if ((Get-Date) -gt $deadline) { "gave up (never stably unlocked) $(Get-Date -Format o)" | Add-Content $st; exit 0 }
    if (Test-Locked) { $ok = 0 } else { $ok++ }
    Start-Sleep 1
  }
  "unlocked-stable $(Get-Date -Format o) idle_ms=$([Idle3]::Ms())" | Add-Content $st
  while ($next -lt $runs.Count) {
    $r = $runs[$next]
    if (Test-Locked) { "relocked before $($r[0]) $(Get-Date -Format o)" | Add-Content $st; break }
    "run $($r[0]) $(Get-Date -Format o)" | Add-Content $st
    $extra = @{}
    if ($r[2]) { $extra['NoAwase'] = $true }
    & C:/Users/cuzic/dv-x5.ps1 -Name $r[0] -ProbeArgs $r[1] -MinIdleMs 0 @extra *> "$Out/$($r[0])-runner.txt"
    if (Test-Locked) { "relocked during $($r[0]) (result may be INVALID; will retry) $(Get-Date -Format o)" | Add-Content $st; Move-Item "$Out/$($r[0])-probe.log" "$Out/$($r[0])-probe.invalid-$(Get-Date -Format HHmmss).log" -Force -ErrorAction SilentlyContinue; break }
    $next++
  }
}
"done $(Get-Date -Format o)" | Add-Content $st
