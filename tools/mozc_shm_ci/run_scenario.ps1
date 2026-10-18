# ADR-265 段階 2: 1 つの場面を、mozc-readable を入れた Windows ランナーで、awase(mozc_shm_shadow = true)を動かして測る。
# 手順: MSI を入れる → 入力方式を Mozc Readable だけにする → awase を起動 → shmci.py drive → shmci.py analyze。
# 窓にキーを送るので、コンソール窓を作るプロセスを並行して起動しない(窓が前面を奪って、キーが別の窓に行く)。
# python のヘルパーは shmci.py の中で CREATE_NO_WINDOW で起動する。ここでは Start-Process -Wait を使わない
# (TIP を読み込んだプロセスが起動する mozc_server が子孫として残り、-Wait が戻らない)。
param([Parameter(Mandatory)][ValidateSet('focus', 'threads', 'appcontainer', 'stale')][string]$Scenario,
      [Parameter(Mandatory)][string]$Msi,
      [string]$Dist = 'dist',
      [string]$MozcCiDir = 'mozc-readable\readable\ci',
      [int]$Rounds = 120,
      [string]$OutDir = 'out',
      [switch]$NoAwase)
$ErrorActionPreference = 'Stop'
$Clsid   = '{AFC94560-63DA-488E-9227-B04F3EF34B5B}'
$Profile = '{7D7B9A38-A0A4-4390-9706-509E98A8C791}'
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

Write-Host '== install Mozc Readable'
$p = Start-Process msiexec.exe -ArgumentList "/i `"$Msi`" /qn /norestart /l*v `"$OutDir\install.log`"" -Wait -PassThru
Write-Host "msiexec exit code: $($p.ExitCode)"
if ($p.ExitCode -notin 0, 3010) { Get-Content "$OutDir\install.log" -Tail 60; exit 1 }
$l = New-WinUserLanguageList ja-JP
$l[0].InputMethodTips.Clear()
$l[0].InputMethodTips.Add("0411:$Clsid$Profile")
Set-WinUserLanguageList $l -Force
Write-Host ((Get-WinUserLanguageList | ForEach-Object { "$($_.LanguageTag): $($_.InputMethodTips -join ',')" }) -join "`n")

$aw = $null
if ($NoAwase) {
  # 対照: awase を起動せずに、同じ手順で窓を IME ON にできるか(awase が干渉しているのか、手順の問題かを切り分ける)
  Write-Host '== control: awase is NOT started'
} else {
  Write-Host '== start awase (mozc_shm_shadow = true)'
  $cfg = Join-Path $Dist 'config.toml'
  if (-not (Test-Path $cfg)) { throw "config.toml not found in $Dist" }
  Add-Content -Path $cfg -Value @('', '[diagnostics]', 'mozc_shm_shadow = true')
  $aw = Start-Process -FilePath (Join-Path $Dist 'awase.exe') -WorkingDirectory $Dist -PassThru
  Start-Sleep -Seconds 8
  if ($aw.HasExited) { Write-Host "awase exited early, code $($aw.ExitCode)"; Get-Content (Join-Path $Dist 'awase.log') -ErrorAction SilentlyContinue | Select-Object -Last 40; exit 1 }
  Write-Host "awase pid $($aw.Id)"
}

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$events = Join-Path $OutDir 'events.jsonl'
$log = Join-Path $Dist 'awase.log'

Write-Host "== drive: $Scenario"
python "$here\shmci.py" drive $Scenario --events $events --rounds $Rounds --mozc-ci-dir $MozcCiDir
$driveExit = $LASTEXITCODE
Write-Host "drive exit code: $driveExit"
Start-Sleep -Seconds 2
if ($NoAwase) {
  # 対照は、設定(窓を ON にできたか)だけを見る。drive は設定に失敗しても窓を残してループへ進まず、exit 2 か 0 を返す。
  Write-Host "control result: drive exit code $driveExit (0 = the window could be set to IME on without awase)"
  exit $driveExit
}

# 結果を、後から読めるように残す(awase.log は大きいので [mozc-shm] の行と、冒頭だけ)
if (Test-Path $log) {
  Select-String -Path $log -Pattern '\[mozc-shm\]' | ForEach-Object { $_.Line } | Set-Content (Join-Path $OutDir 'mozc-shm.log') -Encoding utf8
  Get-Content $log -TotalCount 60 | Set-Content (Join-Path $OutDir 'awase-head.log') -Encoding utf8
  Copy-Item $log (Join-Path $OutDir 'awase-full.log') -ErrorAction SilentlyContinue
  Write-Host ("awase.log lines with [mozc-shm]: " + (Get-Content (Join-Path $OutDir 'mozc-shm.log') | Measure-Object -Line).Lines)
} else { Write-Host '::warning::awase.log not found' }

Write-Host '== analyze'
# stale は窓が動かないので、受け入れた記録は少ない(フォーカス 1 回 + 状態の変化)。ほかは 100 回の移動で十分な件数が出る。
$minAccepted = if ($Scenario -eq 'stale') { 1 } else { 20 }
$extra = @()
if ($Scenario -eq 'stale') { $extra += '--require-inactive' }
python "$here\shmci.py" analyze --awase-log $log --events $events --min-accepted $minAccepted --summary-json (Join-Path $OutDir 'summary.json') @extra
$analyzeExit = $LASTEXITCODE

if ($aw) { Stop-Process -Id $aw.Id -Force -ErrorAction SilentlyContinue }
if ($driveExit -ne 0) { exit $driveExit }
exit $analyzeExit
