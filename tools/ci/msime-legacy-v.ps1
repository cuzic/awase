# ADR-248 の検証スイート(Opus r1 の V0〜V7 の CI 化)。観測のみ。各セルは複数回、各ステップの「前」の状態も記録する。
#   v0  : key の列と状態の対応(V0)と ND=0(V7)
#   v0p : ND の陽性対照(V0′)
#   v3  : 互換モード検出 4 通り(V3)
#   v4  : 名前付きスタイルの開状態(V4)
param([string]$Out = 'v-out', [string]$Suite = 'v0', [int]$Reps = 5)
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force -Path $Out | Out-Null
try { [Text.Encoding]::RegisterProvider([Text.CodePagesEncodingProvider]::Instance) } catch {}
$sjis = [Text.Encoding]::GetEncoding(932)
$log = Join-Path $Out "v-$Suite.txt"
function Say([string]$s) { $s | Tee-Object -FilePath $log -Append }
$imejp = 'Software\Microsoft\IME\15.0\IMEJP'
$msimePath = "HKCU:\$imejp\MSIME"
$tsf = 'HKCU:\SOFTWARE\Microsoft\Input\TSF\Tsf3Override\{03b5835f-f03c-411b-9ce2-aa23e1171e36}'
$spike = (Resolve-Path 'dist\ime_key_matrix_spike.exe').Path

function Restart-Ctfmon {
  Get-Process ctfmon -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep -Seconds 2; Start-Process ctfmon.exe -ErrorAction SilentlyContinue; Start-Sleep -Seconds 4
}
function Fmt([string]$l) {
  $o = [regex]::Match($l, 'A\(open=(\S+) conv=(\S+?)\)')
  $c = [regex]::Match($l, 'comp="(.*?)"')
  $t = [regex]::Match($l, 'tail="(.*?)"')
  return ('o{0}/c{1}/comp[{2}]/tail[{3}]' -f $o.Groups[1].Value, $o.Groups[2].Value, $c.Groups[1].Value, $t.Groups[1].Value)
}
function Reg-State {
  $m = Get-ItemProperty $msimePath -ErrorAction SilentlyContinue
  $t = (Get-ItemProperty $tsf -ErrorAction SilentlyContinue).NoTsf3Override2
  return ('keystyle={0} ND={1} option1={2} option2={3} NoTsf3Override2={4} DisableNewIME={5}' -f $m.keystyle, $m.NoDirectInputMode, $m.option1, $m.option2, $t, $m.DisableNewIME)
}
# --no-probe: 各キーの後に k/ESC を打たない。seq の 4B=k, 1B=ESC を明示して状態を作る。
$script:cellNo = 0
function Probe([string]$tag, [string]$seq, [int]$reps) {
  $script:cellNo++
  Say "### [c$($script:cellNo)] $tag seq=$seq  [$(Reg-State)]"
  $dist = Split-Path $spike
  for ($r = 1; $r -le $reps; $r++) {
    Remove-Item (Join-Path $dist 'ime_key_matrix_spike.log') -ErrorAction SilentlyContinue
    Push-Location $dist
    $null = Start-Process -FilePath $spike -ArgumentList "--auto --hold=180 --activate-gji --msime --no-probe --seq=$seq" -PassThru
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
    $lines = Get-Content $lf -Encoding utf8
    $parts = @()
    for ($i = 0; $i -lt $lines.Count; $i++) {
      $m = [regex]::Match($lines[$i], '\] KEY \[.*?\].*?vk=0x([0-9A-Fa-f]+).*?状態=(.*)$')
      if (-not $m.Success) { continue }
      $before = ''; $a400 = ''
      for ($j = $i + 1; $j -lt [Math]::Min($i + 9, $lines.Count); $j++) {
        if ($lines[$j] -match '\] KEY \[') { break }
        if (($lines[$j] -match '^\s+前\s+:') -and (-not $before)) { $before = Fmt $lines[$j] }
        elseif (($lines[$j] -match '^\s+\+400ms:') -and (-not $a400)) { $a400 = Fmt $lines[$j] }
      }
      $parts += ('{0}[{1}] {2} => {3}' -f $m.Groups[1].Value, $m.Groups[2].Value.Trim(), $before, $a400)
    }
    Say ("  #{0} {1}" -f $r, ($parts -join '  ||  '))
  }
}

function Encode-Table([byte[]]$src, [string]$row, [string]$codes) {
  $recs = New-Object System.Collections.Generic.List[byte[]]
  $cur = New-Object System.Collections.Generic.List[byte]
  foreach ($b in $src) { if ($b -eq 0) { if ($cur.Count -gt 0) { $recs.Add($cur.ToArray()); $cur.Clear() } } else { $cur.Add($b) } }
  $out = New-Object System.Collections.Generic.List[byte]
  $done = $false
  foreach ($r in $recs) {
    if ($sjis.GetString($r).StartsWith([string]"$row=")) { $r = $sjis.GetBytes("$row=$codes"); $done = $true }
    $out.AddRange($r); $out.Add(0)
  }
  if (-not $done) { $out.AddRange($sjis.GetBytes("$row=$codes")); $out.Add(0) }
  $out.Add(0)
  return [byte[]]$out.ToArray()
}
# Custom を NATURAL の全表のコピーで作り、edits(表名・行・6列のコード)を適用して keystyle=Custom にする
function Build-Custom($edits) {
  $nat = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("$imejp\StyleList\NATURAL")
  $cust = "HKCU:\$imejp\StyleList\Custom"
  if (Test-Path $cust) { Remove-Item $cust -Recurse -Force }
  New-Item -Path $cust -Force | Out-Null
  foreach ($n in $nat.GetValueNames()) {
    $v = [byte[]]$nat.GetValue($n)
    foreach ($e in $edits) { if ($e.Table -eq $n) { $v = Encode-Table $v $e.Row $e.Codes } }
    Set-ItemProperty -Path $cust -Name $n -Value $v -Type Binary -Force
  }
  Set-ItemProperty -Path $msimePath -Name keystyle -Value 'Custom' -Type String -Force
}
function Set-Direct([int]$nd) { Set-ItemProperty -Path $msimePath -Name NoDirectInputMode -Value $nd -Type DWord -Force }
function Set-Compat($tsfVal, $dnewVal) {
  if ($null -eq $tsfVal) { Remove-ItemProperty -Path $tsf -Name NoTsf3Override2 -ErrorAction SilentlyContinue } else { New-Item -Path $tsf -Force | Out-Null; Set-ItemProperty -Path $tsf -Name NoTsf3Override2 -Value $tsfVal -Type DWord }
  if ($null -eq $dnewVal) { Remove-ItemProperty -Path $msimePath -Name DisableNewIME -ErrorAction SilentlyContinue } else { Set-ItemProperty -Path $msimePath -Name DisableNewIME -Value $dnewVal -Type DWord }
}

# --- 共通の準備: 互換モード ON、Microsoft_IME テンプレートで StyleList を作る
New-Item -Path $msimePath -Force | Out-Null
Set-Compat 1 1
Restart-Ctfmon
$exe = (Get-ChildItem "$env:windir\System32\IME" -Recurse -Filter 'IMJPUEXC.EXE' | Select-Object -First 1).FullName
& $exe setkeytemplate 'Microsoft_IME' 2>&1 | Out-Null
Say "OS $([Environment]::OSVersion.VersionString)  [$(Reg-State)]"

function Set-KeyAssign($enabled, $muhenkan) {
  if ($null -eq $enabled) { Remove-ItemProperty -Path $msimePath -Name IsKeyAssignmentEnabled -ErrorAction SilentlyContinue } else { Set-ItemProperty -Path $msimePath -Name IsKeyAssignmentEnabled -Value $enabled -Type DWord -Force }
  if ($null -eq $muhenkan) { Remove-ItemProperty -Path $msimePath -Name KeyAssignmentMuhenkan -ErrorAction SilentlyContinue } else { Set-ItemProperty -Path $msimePath -Name KeyAssignmentMuhenkan -Value $muhenkan -Type DWord -Force }
}
switch ($Suite) {
  'v3c' {
    # M3-1: エンジンの陽性対照。新UIの割り当て(IsKeyAssignmentEnabled=1・KeyAssignmentMuhenkan=0=IME-オン)を入れ、
    # keystyle=NATURAL のまま、互換フラグ別に閉・無変換を押す。新エンジンが動いていれば開く。旧エンジンなら変化なし。
    foreach ($ka in @(@($null, $null), @(1, 0))) {
      Set-KeyAssign $ka[0] $ka[1]
      foreach ($c in @(@(1, 1), @(1, $null), @($null, 1), @($null, $null), @(0, 0))) {
        Set-Compat $c[0] $c[1]; Restart-Ctfmon
        Probe ("KeyAssign enabled={0} muhenkan={1} / NoTsf3Override2={2} DisableNewIME={3} 閉・無変換" -f $ka[0], $ka[1], $c[0], $c[1]) '1D' $Reps
      }
    }
    # 参考: 読み込まれた IME の DLL(旧エンジンか新エンジンかの手がかり)
    Say 'modules (ctfmon / IME 関連のロード済み DLL):'
    Get-Process | Where-Object { $_.ProcessName -match 'ctfmon|TextInputHost|ime_key_matrix' } | ForEach-Object {
      try { $mods = ($_.Modules | Where-Object { $_.ModuleName -match 'msime|imjp|imetip|tsf3|InputSwitch|Windows.UI.Input' } | ForEach-Object { $_.ModuleName }) -join ','; Say ("  {0}: {1}" -f $_.ProcessName, $mods) } catch { Say ("  {0}: (modules unreadable)" -f $_.ProcessName) }
    }
  }
  'v0d' {
    # M3-2: ローマ字の途中(ｋ)・かな未確定(か)と列の対応。
    foreach ($nd in 1, 0) {
      Set-Direct $nd
      $n = if ($nd -eq 1) { $Reps } else { 3 }
      foreach ($cfg in @(@('col1=00,他=CD', '00 CD CD CD CD CD'), @('col1=CE,他=00', 'CE 00 00 00 00 00'), @('col1=CE,他=CD(dragonflyg4型)', 'CE CD CD CD CD CD'))) {
        Build-Custom @(@{ Table = 'key'; Row = '無変換'; Codes = $cfg[1] })
        Probe ("ND{0} {1} 開・ｋ(ローマ字の途中)" -f $nd, $cfg[0]) '1C,4B,1D' $n
        Probe ("ND{0} {1} 開・か(未確定)" -f $nd, $cfg[0]) '1C,4B,41,1D' $n
      }
    }
    Set-Direct 1
    foreach ($col in 2..6) {
      $codes = (1..6 | ForEach-Object { if ($_ -eq $col) { 'CD' } else { '00' } }) -join ' '
      Build-Custom @(@{ Table = 'key'; Row = '無変換'; Codes = $codes })
      Probe "col1=00,col$col だけ=CD 開・ｋ" '1C,4B,1D' 3
    }
  }
  'v0e' {
    # 1 列目に置かれうる(半角/全角など)コードを、Custom の 1 列目に置いた試行(開・入力なし)。ND=1/0。
    foreach ($nd in 1, 0) {
      Set-Direct $nd
      foreach ($code in 'CD', 'B3', 'CE', 'A4', 'CA', 'C9') {
        Build-Custom @(@{ Table = 'key'; Row = '無変換'; Codes = "$code 00 00 00 00 00" })
        Probe ("ND{0} col1={1}(他00) 開・入力なし" -f $nd, $code) '1C,1D' 3
      }
    }
  }
}
Say '=== done ==='
