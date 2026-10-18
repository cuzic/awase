# ADR-265 stage 2 additional measurement: focus switching between Notepad and a plain Win32 test window (ui_host.py).
# Chrome was dropped for this scenario: its first IME-on key is unreliable on this machine (also in CI), which stopped
# the setup three times. Chrome's behaviour is covered by the first measurement.
$ErrorActionPreference = 'Continue'
$out = 'C:\Users\cuzic\adr265-measure-out'
$py = Join-Path $out 'drive_scenarios.py'
$rounds = 25
New-Item -ItemType Directory -Force $out | Out-Null
$runlog = Join-Path $out 'run.log'
function Log($m) { Add-Content $runlog "$((Get-Date).ToUniversalTime().ToString('o')) $m" }
Log 'scenarios2.ps1 start'
$before = @(Get-Process -Name Notepad -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
Start-Process notepad.exe
Start-Sleep 8
$np = @(Get-Process -Name Notepad -ErrorAction SilentlyContinue | Where-Object { $_.Id -notin $before } | ForEach-Object { $_.Id })
Log "notepad new pids: $($np -join ',')"
# The scoop "python" is a shim: its PID is not the PID of the process that owns the window. Start the real python.exe.
# -NoNewWindow + redirected output: no console window that could take the foreground (ui_host.py prints and uses
# faulthandler, so pythonw.exe (no stdout/stderr) cannot run it).
$realpy = (& python -c "import sys; print(sys.executable)" | Select-Object -First 1).Trim()
Log "real python: $realpy"
$ui = Start-Process -FilePath $realpy -ArgumentList "`"$(Join-Path $out 'ui_host.py')`" `"$(Join-Path $out 'ui_host_tid.txt')`" `"`" 900" -PassThru -NoNewWindow -RedirectStandardOutput (Join-Path $out 'ui_host.out') -RedirectStandardError (Join-Path $out 'ui_host.err')
Start-Sleep 10
Log "ui_host (pythonw) pid: $($ui.Id)  alive: $(-not $ui.HasExited)"
if ($np.Count -gt 0 -and -not $ui.HasExited) {
  & python $py --scenario focus --notepad-pids ($np -join ',') --second-pids "$($ui.Id)" --rounds $rounds --out (Join-Path $out 'scen_focus.jsonl') 2>&1 | Out-File (Join-Path $out 'scen_focus.stdout.txt') -Encoding utf8
  Log "focus scenario exit=$LASTEXITCODE"
}
Stop-Process -Id $np -Force -ErrorAction SilentlyContinue
Stop-Process -Id $ui.Id -Force -ErrorAction SilentlyContinue
Log 'scenarios2.ps1 done'
