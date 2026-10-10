# 実機(Windows 11 クライアント)での検証(ADR-248 V1)その2: 互換モードを設定アプリのトグルで正規に切り替える。Windows PowerShell 5.1 互換。
# 触るのは「以前のバージョンの Microsoft IME を使う」トグル(設定アプリ経由)と keystyle だけ。StyleList\Custom の中身は書き換えない。
# 開始時にバックアップし、try/finally で必ず元に戻して検証する。awase は止めて、終わったら同じパスで再起動する。
param([string]$Out = "$env:USERPROFILE\msime-real-out", [int]$Reps = 3,
      [string]$Spike = 'C:\awase-spike-target\debug\examples\ime_key_matrix_spike.exe',
      [string]$Probe = 'C:\Users\cuzic\awase-spike-target\debug\examples\msime_key_assignment_settings_probe.exe')
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$log = Join-Path $Out 'real.txt'
function Say([string]$s) { $s | Tee-Object -FilePath $log -Append }
$imejp = 'Software\Microsoft\IME\15.0\IMEJP'
$msimePath = "HKCU:\$imejp\MSIME"
$tsf = 'HKCU:\SOFTWARE\Microsoft\Input\TSF\Tsf3Override\{03b5835f-f03c-411b-9ce2-aa23e1171e36}'
if (-not (Test-Path $Spike)) { Say "harness not found: $Spike"; exit 2 }

function Reg-State {
  $m = Get-ItemProperty $msimePath -ErrorAction SilentlyContinue
  $t = (Get-ItemProperty $tsf -ErrorAction SilentlyContinue).NoTsf3Override2
  return ('keystyle={0} ND={1} NoTsf3Override2={2}' -f $m.keystyle, $m.NoDirectInputMode, $t)
}
function Fmt([string]$l) {
  $o = [regex]::Match($l, 'A\(open=(\S+) conv=(\S+?)\)')
  $c = [regex]::Match($l, 'comp="(.*?)"')
  return ('o{0}/c{1}/comp[{2}]' -f $o.Groups[1].Value, $o.Groups[2].Value, $c.Groups[1].Value)
}
$script:cellNo = 0
function Probe([string]$tag, [string]$seq, [int]$reps) {
  $script:cellNo++
  Say "### [c$($script:cellNo)] $tag seq=$seq  [$(Reg-State)]"
  $dist = Split-Path $Spike
  for ($r = 1; $r -le $reps; $r++) {
    Remove-Item (Join-Path $dist 'ime_key_matrix_spike.log') -ErrorAction SilentlyContinue
    Push-Location $dist
    $null = Start-Process -FilePath $Spike -ArgumentList "--auto --hold=180 --activate-gji --msime --no-probe --seq=$seq" -PassThru
    $deadline = (Get-Date).AddSeconds(90)
    while ((Get-Date) -lt $deadline) {
      Start-Sleep -Milliseconds 700
      if ((Test-Path ime_key_matrix_spike.log) -and (Select-String -Path ime_key_matrix_spike.log -Pattern '全手順完了' -Quiet)) { break }
    }
    Get-Process ime_key_matrix_spike -ErrorAction SilentlyContinue | Stop-Process -Force
    Pop-Location
    $lf = Join-Path $dist 'ime_key_matrix_spike.log'
    if (-not (Test-Path $lf)) { Say "  #$r (no log)"; continue }
    Copy-Item $lf (Join-Path $Out ("log-c{0}-r{1}.log" -f $script:cellNo, $r)) -ErrorAction SilentlyContinue
    $lines = Get-Content $lf -Encoding UTF8
    $parts = @()
    for ($i = 0; $i -lt $lines.Count; $i++) {
      $m = [regex]::Match($lines[$i], '\] KEY \[.*?\].*?vk=0x([0-9A-Fa-f]+).*?')
      if (-not $m.Success) { continue }
      $before = ''; $a400 = ''
      for ($j = $i + 1; $j -lt [Math]::Min($i + 9, $lines.Count); $j++) {
        if ($lines[$j] -match '\] KEY \[') { break }
        if (($lines[$j] -match '^\s+.+\s:\sA\(') -and ($lines[$j] -notmatch '\+\d+ms') -and (-not $before)) { $before = Fmt $lines[$j] }
        elseif (($lines[$j] -match '\+400ms:') -and (-not $a400)) { $a400 = Fmt $lines[$j] }
      }
      $parts += ('{0} {1} => {2}' -f $m.Groups[1].Value, $before, $a400)
    }
    Say ("  #{0} {1}" -f $r, ($parts -join '  ||  '))
  }
}
function Restart-Ctfmon {
  Get-Process ctfmon -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep -Seconds 2; Start-Process ctfmon.exe -ErrorAction SilentlyContinue; Start-Sleep -Seconds 4
}

# ---- バックアップ(既にあれば上書きしない) ----
$bk = "$env:USERPROFILE\msime-real-backup"
New-Item -ItemType Directory -Force -Path $bk | Out-Null
if (-not (Test-Path "$bk\msime.reg")) {
  reg export "HKCU\$imejp\MSIME" "$bk\msime.reg" /y | Out-Null
  reg export "HKCU\$imejp\StyleList" "$bk\stylelist.reg" /y | Out-Null
  reg export 'HKCU\SOFTWARE\Microsoft\Input\TSF\Tsf3Override' "$bk\tsf3.reg" /y 2>&1 | Out-Null
}
$orig = @{
  keystyle = (Get-ItemProperty $msimePath).keystyle
  tsfExists = ($null -ne (Get-ItemProperty $tsf -ErrorAction SilentlyContinue).NoTsf3Override2)
  tsfVal = (Get-ItemProperty $tsf -ErrorAction SilentlyContinue).NoTsf3Override2
}
Say ("orig: keystyle={0} NoTsf3Override2 exists={1} value={2}" -f $orig.keystyle, $orig.tsfExists, $orig.tsfVal)
$apps = @()
foreach ($p in (Get-Process awase, awase-settings -ErrorAction SilentlyContinue)) { $apps += ,@($p.Path, (Split-Path $p.Path)) }
Say ("stopping: " + (($apps | ForEach-Object { $_[0] }) -join ', '))


function Set-CompatUI([string]$want) {
  $dir = Split-Path $Probe
  Push-Location $dir
  Remove-Item msime_key_assignment_settings_probe.log -ErrorAction SilentlyContinue
  $p = Start-Process -FilePath $Probe -ArgumentList "--general-only --set-compat=$want" -PassThru -WindowStyle Hidden
  if (-not $p.WaitForExit(90000)) { Stop-Process -Id $p.Id -Force }
  Get-Content msime_key_assignment_settings_probe.log -Encoding UTF8 -ErrorAction SilentlyContinue | Where-Object { $_ -match 'set-compat|toggle_element|RESULT: --set|invoke OK' } | ForEach-Object { Say ("   probe: " + $_) }
  Pop-Location
  Get-Process SystemSettings -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep -Seconds 4
  Say ("after set-compat=$want : [$(Reg-State)]")
}
function Ensure-Compat([string]$want, [int]$expect) {
  for ($try = 1; $try -le 3; $try++) {
    Set-CompatUI $want
    $v = (Get-ItemProperty $tsf -ErrorAction SilentlyContinue).NoTsf3Override2
    if ($v -eq $expect) { return }
    Say "   (NoTsf3Override2=$v, expected $expect; retry $try)"
  }
}
try {
  Get-Process awase, awase-settings -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep -Seconds 2
  Ensure-Compat 'on' 1
  foreach ($style in 'NATURAL', 'Custom', 'ATOK') {
    Set-ItemProperty -Path $msimePath -Name keystyle -Value $style -Type String -Force
    Start-Sleep -Seconds 2
    $name = "compat=ON(UI) keystyle=$style"
    Probe "$name 閉・変換" '1C' $Reps
    Probe "$name 閉・無変換" '1D' $Reps
    Probe "$name 開・入力なし 無変換" '1C,1D' $Reps
    Probe "$name 開・かな未確定 無変換" '1C,4B,41,1D' $Reps
  }
}
finally {
  Set-ItemProperty -Path $msimePath -Name keystyle -Value $orig.keystyle -Type String -Force
  Ensure-Compat 'off' 0
  # 検証: トグルで戻らなかった場合のフォールバック(レジストリを元の値へ)
  $now = (Get-ItemProperty $tsf -ErrorAction SilentlyContinue).NoTsf3Override2
  if ($orig.tsfExists -and ($now -ne $orig.tsfVal)) { Say "fallback: NoTsf3Override2 $now -> $($orig.tsfVal)"; Set-ItemProperty -Path $tsf -Name NoTsf3Override2 -Value $orig.tsfVal -Type DWord; Restart-Ctfmon }
  Say ("restored: [$(Reg-State)]  (orig keystyle=" + $orig.keystyle + " tsf=" + $orig.tsfVal + ")")
  foreach ($a in $apps) { if ($a[0]) { Start-Process -FilePath $a[0] -WorkingDirectory $a[1] } }
  Start-Sleep -Seconds 3
  Say ("restarted: " + ((Get-Process awase, awase-settings -ErrorAction SilentlyContinue | ForEach-Object { $_.ProcessName }) -join ', '))
  Say '=== done ==='
}
