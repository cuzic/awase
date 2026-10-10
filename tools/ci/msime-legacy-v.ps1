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
function Probe([string]$tag, [string]$seq, [int]$reps) {
  Say "### $tag seq=$seq  [$(Reg-State)]"
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
    Copy-Item $lf (Join-Path $Out ("log-{0}-{1}.log" -f ($tag -replace '[^A-Za-z0-9_-]', '_'), $r)) -ErrorAction SilentlyContinue
    $lines = Get-Content $lf -Encoding utf8
    $parts = @()
    for ($i = 0; $i -lt $lines.Count; $i++) {
      $m = [regex]::Match($lines[$i], '\] KEY \[.*?\].*?vk=0x([0-9A-Fa-f]+).*?状態=(.*)$')
      if (-not $m.Success) { continue }
      $before = ''; $a400 = ''
      for ($j = $i + 1; $j -lt [Math]::Min($i + 9, $lines.Count); $j++) {
        if ($lines[$j] -match '^\s+前\s+:') { $before = Fmt $lines[$j] }
        elseif ($lines[$j] -match '^\s+\+400ms:') { $a400 = Fmt $lines[$j] }
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

switch ($Suite) {
  'v0b' {
    # V0(再): 列と状態の対応。入力中は 4B,41(か)で作る。1 列目だけ/2〜6 列目の 1 列だけに既知のコードを置く。
    #   97: 1 列目で効果が分かっている(開・入力なしで conv→0x1B)。CD: 入力中の列で効果が分かっている(ND=1 で conv→0x10、ND=0 で閉じる)。
    foreach ($nd in 1, 0) {
      Set-Direct $nd
      $nreps = if ($nd -eq 1) { $Reps } else { 3 }
      Build-Custom @(@{ Table = 'key'; Row = '無変換'; Codes = '97 00 00 00 00 00' })
      Probe "ND$nd col1=97(他00) 開・入力なし" '1C,1D' $nreps
      Probe "ND$nd col1=97(他00) 開・入力中か" '1C,4B,41,1D' $nreps
      Build-Custom @(@{ Table = 'key'; Row = '無変換'; Codes = '00 CD CD CD CD CD' })
      Probe "ND$nd col1=00(他CD) 開・入力なし" '1C,1D' $nreps
      Probe "ND$nd col1=00(他CD) 開・入力中か" '1C,4B,41,1D' $nreps
      Build-Custom @()
      Probe "ND$nd 無加工Custom 開・入力中か" '1C,4B,41,1D' $nreps
    }
  }
  'v0c' {
    # 2〜6 列目の 1 列だけに CD を置いて、入力中(か)・変換済み(か+Space)のどれで効くかを調べる(ND=1、n=3)。
    Set-Direct 1
    foreach ($col in 2..6) {
      $codes = (1..6 | ForEach-Object { if ($_ -eq $col) { 'CD' } else { '00' } }) -join ' '
      Build-Custom @(@{ Table = 'key'; Row = '無変換'; Codes = $codes })
      Probe "col$col=CD 開・入力中か" '1C,4B,41,1D' 3
      Probe "col$col=CD 変換済み(か+Space)" '1C,4B,41,20,1D' 3
    }
  }
  'v3b' {
    # V3(補遺): keystyle の不在=NATURAL か、名前付き(ATOK)は互換フラグに関係なく効くか、Custom は互換フラグ OFF でも効くか(閉で 変換 を押す)。
    # (a) keystyle 値を消した状態(StyleList は残す)
    Set-Compat 1 1; Restart-Ctfmon
    Remove-ItemProperty -Path $msimePath -Name keystyle -ErrorAction SilentlyContinue
    Probe 'keystyle値なし(互換ON) 閉・変換' '1C' $Reps
    Set-Compat $null $null; Restart-Ctfmon
    Remove-ItemProperty -Path $msimePath -Name keystyle -ErrorAction SilentlyContinue
    Probe 'keystyle値なし(互換フラグなし) 閉・変換' '1C' $Reps
    # (b) 名前付き ATOK(変換は閉で開かない)を互換フラグ別に
    foreach ($c in @(@(1, 1), @($null, $null), @(0, 0))) {
      & $exe setkeytemplate 'ATOK' 2>&1 | Out-Null
      Set-Compat $c[0] $c[1]; Restart-Ctfmon
      Probe ("ATOK NoTsf3Override2={0} DisableNewIME={1} 閉・変換" -f $c[0], $c[1]) '1C' $Reps
    }
    # (c) Custom の S4key 変換行を 00 にして(閉で開かなくなる)、互換フラグなし/0 で効くか
    foreach ($c in @(@($null, $null), @(0, 0))) {
      Set-Compat $c[0] $c[1]; Restart-Ctfmon
      Build-Custom @(@{ Table = 'S4key'; Row = '変換'; Codes = '00 00 00 00 00 00' })
      Probe ("Custom(S4key 変換=00) NoTsf3Override2={0} DisableNewIME={1} 閉・変換" -f $c[0], $c[1]) '1C' $Reps
    }
  }
}
Say '=== done ==='
