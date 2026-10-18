# ADR-265 stage 2, physical-key measurement: a NEW Notepad; a person presses IME-mode keys while drive_physical.py records.
$ErrorActionPreference = 'Continue'
$out = 'C:\Users\cuzic\adr265-measure-out'
$seconds = 420
New-Item -ItemType Directory -Force $out | Out-Null
$runlog = Join-Path $out 'run.log'
function Log($m) { Add-Content $runlog "$((Get-Date).ToUniversalTime().ToString('o')) $m" }
Log 'physical.ps1 start'
$before = @(Get-Process -Name Notepad -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
Start-Process notepad.exe
Start-Sleep 8
$np = @(Get-Process -Name Notepad -ErrorAction SilentlyContinue | Where-Object { $_.Id -notin $before } | ForEach-Object { $_.Id })
Log "notepad new pids: $($np -join ',')"
if ($np.Count -gt 0) {
  Log 'recording starts now'
  & python (Join-Path $out 'drive_physical.py') --pids ($np -join ',') --seconds $seconds --out (Join-Path $out 'physical.jsonl') 2>&1 | Out-File (Join-Path $out 'physical.stdout.txt') -Encoding utf8
  Log "physical driver exit=$LASTEXITCODE"
  Stop-Process -Id $np -Force -ErrorAction SilentlyContinue
} else { Log 'no new notepad process (joined an existing instance?) - skipped' }
Log 'physical.ps1 done'
