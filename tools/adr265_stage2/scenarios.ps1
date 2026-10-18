# ADR-265 stage 2 additional measurement: focus switching and input-method switch-away (new processes only).
$ErrorActionPreference = 'Continue'
$out = 'C:\Users\cuzic\adr265-measure-out'
$py = Join-Path $out 'drive_scenarios.py'
$rounds = 25
New-Item -ItemType Directory -Force $out | Out-Null
$runlog = Join-Path $out 'run.log'
function Log($m) { Add-Content $runlog "$((Get-Date).ToUniversalTime().ToString('o')) $m" }
function OurChrome { @(Get-CimInstance Win32_Process -Filter "Name='chrome.exe'" -ErrorAction SilentlyContinue | Where-Object { $_.CommandLine -like '*adr265-measure-out*' } | ForEach-Object { $_.ProcessId }) }
Log 'scenarios.ps1 start'
# Notepad: only a NEW process counts
$before = @(Get-Process -Name Notepad -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
Start-Process notepad.exe
Start-Sleep 8
$np = @(Get-Process -Name Notepad -ErrorAction SilentlyContinue | Where-Object { $_.Id -notin $before } | ForEach-Object { $_.Id })
Log "notepad new pids: $($np -join ',')"
# Chrome with its own profile directory
$page = Join-Path $out 'input.html'
Set-Content $page '<!doctype html><title>t</title><input id=a autofocus style="width:90%;font-size:30px">' -Encoding UTF8
$chrome = 'C:\Users\cuzic\scoop\apps\googlechrome\current\chrome.exe'
$udd = Join-Path $out 'chrome-profile'
Start-Process $chrome -ArgumentList "--user-data-dir=`"$udd`" --no-first-run --no-default-browser-check --new-window `"file:///$($page -replace '\\','/')`""
Start-Sleep 10
$cp = OurChrome
Log "chrome (our profile) pids: $($cp -join ',')"
if ($np.Count -gt 0 -and $cp.Count -gt 0) {
  & python $py --scenario focus --notepad-pids ($np -join ',') --chrome-pids ($cp -join ',') --rounds $rounds --out (Join-Path $out 'scen_focus.jsonl') 2>&1 | Out-File (Join-Path $out 'scen_focus.stdout.txt') -Encoding utf8
  Log "focus scenario exit=$LASTEXITCODE"
}
if ($np.Count -gt 0) {
  & python $py --scenario away --notepad-pids ($np -join ',') --rounds $rounds --out (Join-Path $out 'scen_away.jsonl') 2>&1 | Out-File (Join-Path $out 'scen_away.stdout.txt') -Encoding utf8
  Log "away scenario exit=$LASTEXITCODE"
}
Stop-Process -Id $np -Force -ErrorAction SilentlyContinue
foreach ($p in (OurChrome)) { Stop-Process -Id $p -Force -ErrorAction SilentlyContinue }
Log 'scenarios.ps1 done'
