# ADR-265 stage 2: run the drive for Notepad and Chrome (each a NEW process; the user's own windows are never touched).
$ErrorActionPreference = 'Continue'
$out = 'C:\Users\cuzic\adr265-measure-out'
$drv = Join-Path $out 'drive_measure.py'
$cycles = 25
New-Item -ItemType Directory -Force $out | Out-Null
$runlog = Join-Path $out 'run.log'
function Log($m) { Add-Content $runlog "$((Get-Date).ToUniversalTime().ToString('o')) $m" }
Log 'measure.ps1 start'
# --- Notepad (new process only; if it joined an existing instance there is no new pid and we skip)
$before = @(Get-Process -Name Notepad -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
Log "notepad pids before: $($before -join ',')"
Start-Process notepad.exe
Start-Sleep 8
$np = @(Get-Process -Name Notepad -ErrorAction SilentlyContinue | Where-Object { $_.Id -notin $before } | ForEach-Object { $_.Id })
Log "notepad new pids: $($np -join ',')"
if ($np.Count -gt 0) {
  & python $drv --pids ($np -join ',') --label notepad --cycles $cycles --out (Join-Path $out 'drive_notepad.jsonl') 2>&1 | Out-File (Join-Path $out 'drive_notepad.stdout.txt') -Encoding utf8
  Log "notepad driver exit=$LASTEXITCODE"
  Stop-Process -Id $np -Force -ErrorAction SilentlyContinue
} else { Log 'notepad: no new process (joined an existing instance?) - skipped' }
# --- Chrome with its own profile directory; only processes whose command line carries that directory are ours
$page = Join-Path $out 'input.html'
Set-Content $page '<!doctype html><title>t</title><input id=a autofocus style="width:90%;font-size:30px">' -Encoding UTF8
$chrome = 'C:\Users\cuzic\scoop\apps\googlechrome\current\chrome.exe'
$udd = Join-Path $out 'chrome-profile'
Start-Process $chrome -ArgumentList "--user-data-dir=`"$udd`" --no-first-run --no-default-browser-check --new-window `"file:///$($page -replace '\\','/')`""
Start-Sleep 10
function OurChrome { @(Get-CimInstance Win32_Process -Filter "Name='chrome.exe'" -ErrorAction SilentlyContinue | Where-Object { $_.CommandLine -like '*adr265-measure-out*' } | ForEach-Object { $_.ProcessId }) }
$cp = OurChrome
Log "chrome (our profile) pids: $($cp -join ',')"
if ($cp.Count -gt 0) {
  & python $drv --pids ($cp -join ',') --label chrome --cycles $cycles --out (Join-Path $out 'drive_chrome.jsonl') 2>&1 | Out-File (Join-Path $out 'drive_chrome.stdout.txt') -Encoding utf8
  Log "chrome driver exit=$LASTEXITCODE"
  foreach ($p in (OurChrome)) { Stop-Process -Id $p -Force -ErrorAction SilentlyContinue }
} else { Log 'chrome: no process with our profile - skipped' }
Log 'measure.ps1 done'
