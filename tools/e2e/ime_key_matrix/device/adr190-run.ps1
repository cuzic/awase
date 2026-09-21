# ADR-190 real-device verification: run scenarios (ASCII only). Hands-off: injects keys into the foreground.
# Results: C:\Users\cuzic\adr190-out\<name>\{awase.log,spike.log,config.toml,rc.txt}
$ErrorActionPreference = 'Continue'
$main = 'C:\Users\cuzic\scoop\persist\msys2\home\cuzic\awase'
$wt   = 'C:\Users\cuzic\awase-adr190'
$out  = 'C:\Users\cuzic\adr190-out'
$log  = 'C:\Users\cuzic\adr190-run.log'
$done = 'C:\Users\cuzic\adr190-run.done'
Remove-Item $done -ErrorAction SilentlyContinue
Remove-Item $out -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $out | Out-Null
'start ' + (Get-Date -Format o) | Out-File $log -Encoding ascii
function Log($m) { $m | Out-File $log -Append -Encoding ascii }
$seqDbe   = '--seq=F2,F0,F2,F1,F2,F0,F0,F1,F2'
$seqKanji = '--seq=F2,19,19,16,1A,16,1A,19,F2,1A'
$seqShift = '--seq=F2,A0,A0,A0,A1,F2'
$doneMark = [regex]::Unescape('\u5168\u624b\u9806\u5b8c\u4e86')  # all steps done marker
$seqIme   = '--seq=1A,16,1A,16,1A,16,1A'
# name, variant(bin dir), spike args, config extra lines (;-separated)
$plan = @(
  # ADR-191 で設定 dbe_mode_key_policy(suppress/passthrough)は撤去した。以前の gji-dbe-suppress と gji-dbe-passthrough は
  # 同じ構成になるので1つにした（旧キーは config.toml に残っても無視される。round2 C-N4）。
  @('gji-dbe',              'baseline', "$seqDbe",   ''),
  @('gji-kanji',            'baseline', "$seqKanji", ''),
  @('gji-shift',            'baseline', "$seqShift", 'half_width_alnum_toggle = "all"'),
  @('gji-hz',               'baseline', "--hz",      ''),
  @('msime-dbe',            'baseline', "$seqDbe --msime",          ''),
  @('msime-kanji',          'baseline', "$seqKanji --msime",        ''),
  @('msime-shift',          'baseline', "$seqShift --msime",        'half_width_alnum_toggle = "all"'),
  @('msime-ime-a10',        'a10',      "$seqIme --msime",          ''),
  @('msime-kanji-a10',      'a10',      "$seqKanji --msime",        ''),
  @('msime-dbe-a10',        'a10',      "$seqDbe --msime",          ''),
  @('gji-restore',          'baseline', "--seq=1A",  '')
)
foreach ($p in $plan) {
  $name = $p[0]; $variant = $p[1]; $sargs = $p[2]; $extra = $p[3]
  $dir = "$out\$name"; New-Item -ItemType Directory -Force -Path $dir | Out-Null
  $bin = "$wt\target\bin-$variant"
  Log ("=== $name ($variant) " + (Get-Date -Format o))
  Get-Process awase, ime_key_matrix_spike -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 2
  # config: user's real config.toml + the scenario lines (toggle left as the user has it)
  $cfg = Get-Content "$main\config.toml" -Raw -Encoding utf8
  foreach ($line in ($extra -split ';' | ForEach-Object { $_.Trim() } | Where-Object { $_ })) {
    $key = ($line -split '=')[0].Trim()
    if ($cfg -match "(?m)^$key *=") { $cfg = $cfg -replace "(?m)^$key *=.*$", $line }
    else { $cfg = $cfg -replace '(?m)^\[general\]\s*$', "[general]`r`n$line" }
  }
  $cfgPath = "$dir\config.toml"
  [IO.File]::WriteAllText($cfgPath, $cfg, (New-Object Text.UTF8Encoding($false)))
  [IO.File]::WriteAllText("$bin\cache.toml", "[imm_capability.`"ime_key_matrix_spike.exe`"]`r`nEdit = `"works`"`r`nRICHEDIT50W = `"works`"`r`n", (New-Object Text.UTF8Encoding($false)))
  Remove-Item "$bin\awase.log", "$bin\ime_key_matrix_spike.log" -ErrorAction SilentlyContinue
  $env:RUST_LOG = 'debug'; $env:AWASE_TEST_INJECTION = '1'
  # order as in CI: spike first (init), awase after 'ROUND 2/2' appears, then wait for completion
  Push-Location $bin
  Start-Process -FilePath .\ime_key_matrix_spike.exe -ArgumentList ("--auto --hold=180 --activate-gji " + $sargs) -WorkingDirectory $bin
  for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 500
    if ((Test-Path ime_key_matrix_spike.log) -and (Select-String -Path ime_key_matrix_spike.log -Pattern 'ROUND 2/2' -Quiet)) { break }
  }
  Start-Process -FilePath "$bin\awase.exe" -ArgumentList "`"$cfgPath`"" -WorkingDirectory $wt
  $deadline = (Get-Date).AddSeconds(200)
  while ((Get-Date) -lt $deadline) {
    Start-Sleep -Seconds 2
    if ((Test-Path ime_key_matrix_spike.log) -and (Select-String -Path ime_key_matrix_spike.log -Pattern $doneMark -Encoding utf8 -Quiet)) { break }
  }
  Pop-Location
  Copy-Item "$bin\awase.log" "$dir\awase.log" -ErrorAction SilentlyContinue
  Copy-Item "$bin\ime_key_matrix_spike.log" "$dir\spike.log" -ErrorAction SilentlyContinue
  Get-Process awase, ime_key_matrix_spike -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  Log ("finished $name")
}
'end ' + (Get-Date -Format o) | Out-File $log -Append -Encoding ascii
'ok' | Out-File $done -Encoding ascii
