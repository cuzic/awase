# ADR195-T7 real-machine check (round3 R1): focus loss mid-learning must abort with reason=interference.
param(
    [string]$Exe = "target\debug\awase-keymap-learn-win.exe",
    [int]$WarmupSec = 8,
    [int]$WaitExitSec = 90,
    [switch]$Control
)
$ErrorActionPreference = "Continue"
Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text;
public class Fg {
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
}
"@
function Fg-Info { $h=[Fg]::GetForegroundWindow(); $sb=New-Object Text.StringBuilder 256; [void][Fg]::GetWindowText($h,$sb,256); $p=0; [void][Fg]::GetWindowThreadProcessId($h,[ref]$p); "$($sb.ToString()) (pid=$p)" }

$out = "$env:TEMP\t7-learn-stdout.log"; $err = "$env:TEMP\t7-learn-stderr.log"
Remove-Item $out,$err -ErrorAction SilentlyContinue
$tableFile = Join-Path (Split-Path -Parent (Resolve-Path $Exe)) "keymap-learn-table.json"
Remove-Item $tableFile -ErrorAction SilentlyContinue
$learn = Start-Process -FilePath $Exe -PassThru -RedirectStandardOutput $out -RedirectStandardError $err -WindowStyle Normal
$null = $learn.Handle
"learn pid=$($learn.Id) started"
Start-Sleep -Seconds $WarmupSec
"before switch: fg = $(Fg-Info)"
"progress lines so far: $((Get-Content $out -ErrorAction SilentlyContinue | Measure-Object -Line).Lines)"
$switched = $true
if (-not $Control) {
    $t0 = Get-Date
    Start-Process notepad.exe
    # Notepad may not take the foreground by itself in time (focus-stealing prevention): force it deterministically.
    $switched = $false
    for ($i = 0; $i -lt 40 -and -not $switched; $i++) {
        Start-Sleep -Milliseconds 250
        $np = Get-Process notepad -ErrorAction SilentlyContinue | Where-Object { $_.StartTime -gt $t0 -and $_.MainWindowHandle -ne 0 } | Select-Object -First 1
        if ($np) {
            [Fg]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero); [Fg]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero)
            [void][Fg]::SetForegroundWindow($np.MainWindowHandle)
            Start-Sleep -Milliseconds 300
            $switched = (Fg-Info) -like "*(pid=$($np.Id))"
        }
    }
}
Start-Sleep -Seconds 1
"after notepad: fg = $(Fg-Info)"
$sw = [Diagnostics.Stopwatch]::StartNew()
$exited = $learn.WaitForExit($WaitExitSec * 1000)
"learn exited=$exited after $([int]$sw.Elapsed.TotalSeconds)s exitcode=$(if($exited){$learn.ExitCode}else{'n/a'})"
"after exit: fg = $(Fg-Info)"
"--- stdout tail ---"; Get-Content $out -Tail 8 -ErrorAction SilentlyContinue
"--- stderr tail ---"; Get-Content $err -Tail 5 -ErrorAction SilentlyContinue
if (-not $exited) { Stop-Process -Id $learn.Id -Force; "learn force-killed (did NOT self-terminate)" }

$result = (Get-Content $out -ErrorAction SilentlyContinue | Where-Object { $_ -like 'result status=*' } | Select-Object -Last 1)
$presses = if ($result -match 'presses=(\d+)') { [int]$Matches[1] } else { -1 }
$fails = @()
if (-not $switched) { $fails += 'could not bring notepad to the foreground (inconclusive)' }
if (-not $exited) { $fails += 'learner did not exit' }
if (-not $result) { $fails += 'no result line' }
if ($Control) {
    if ($exited -and $learn.ExitCode -ne 0) { $fails += "control exit code $($learn.ExitCode) != 0" }
    if ($result -notlike '*status=success*') { $fails += 'control: status is not success' }
} else {
    if ($exited -and $learn.ExitCode -ne 1) { $fails += "exit code $($learn.ExitCode) != 1" }
    if ($result -notlike '*reason=interference*') { $fails += 'reason is not interference' }
    if ($presses -le 0) { $fails += 'presses=0: aborted before the focus switch (not a valid test)' }
    if ((Fg-Info) -like "*(pid=$($learn.Id))") { $fails += 'learner reclaimed the foreground' }
    if (Test-Path $tableFile) { $fails += 'table file written on failure' }
}
if ($fails.Count -gt 0) { $fails | ForEach-Object { "FAIL: $_" }; exit 1 }
"PASS"
# notepad is left open on purpose: never kill by name (the user may have unsaved notepad windows)
