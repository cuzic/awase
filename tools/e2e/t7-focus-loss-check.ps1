# ADR195-T7 real-machine check (round3 R1): focus loss mid-learning must abort with reason=interference.
param(
    [string]$Exe = "target\debug\awase-keymap-learn-win.exe",
    [int]$WarmupSec = 8,
    [int]$WaitExitSec = 90
)
$ErrorActionPreference = "Continue"
Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text;
public class Fg {
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
}
"@
function Fg-Info { $h=[Fg]::GetForegroundWindow(); $sb=New-Object Text.StringBuilder 256; [void][Fg]::GetWindowText($h,$sb,256); $p=0; [void][Fg]::GetWindowThreadProcessId($h,[ref]$p); "$($sb.ToString()) (pid=$p)" }

$out = "$env:TEMP\t7-learn-stdout.log"; $err = "$env:TEMP\t7-learn-stderr.log"
Remove-Item $out,$err -ErrorAction SilentlyContinue
$learn = Start-Process -FilePath $Exe -PassThru -RedirectStandardOutput $out -RedirectStandardError $err -WindowStyle Normal
"learn pid=$($learn.Id) started"
Start-Sleep -Seconds $WarmupSec
"before switch: fg = $(Fg-Info)"
"progress lines so far: $((Get-Content $out -ErrorAction SilentlyContinue | Measure-Object -Line).Lines)"
$np = Start-Process notepad.exe -PassThru
Start-Sleep -Seconds 2
"after notepad: fg = $(Fg-Info)"
$sw = [Diagnostics.Stopwatch]::StartNew()
$exited = $learn.WaitForExit($WaitExitSec * 1000)
"learn exited=$exited after $([int]$sw.Elapsed.TotalSeconds)s exitcode=$(if($exited){$learn.ExitCode}else{'n/a'})"
"after exit: fg = $(Fg-Info)"
"--- stdout tail ---"; Get-Content $out -Tail 8 -ErrorAction SilentlyContinue
"--- stderr tail ---"; Get-Content $err -Tail 5 -ErrorAction SilentlyContinue
if (-not $exited) { Stop-Process -Id $learn.Id -Force; "learn force-killed (did NOT self-terminate)" }
Stop-Process -Id $np.Id -Force -ErrorAction SilentlyContinue
