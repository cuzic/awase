# Custom スタイル(NATURAL の全表のコピー)で、key / S4key のどちらを書き換えると効くか、「直接入力モードを使用しない」の ON/OFF で変わるかを測る。
param([string]$Out = 'custom-out')
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force -Path $Out | Out-Null
try { [Text.Encoding]::RegisterProvider([Text.CodePagesEncodingProvider]::Instance) } catch {}
$sjis = [Text.Encoding]::GetEncoding(932)
$log = Join-Path $Out 'custom.txt'
function Say([string]$s) { $s | Tee-Object -FilePath $log -Append }
$imejp = 'Software\Microsoft\IME\15.0\IMEJP'
$msimePath = "HKCU:\$imejp\MSIME"
$tsf = 'HKCU:\SOFTWARE\Microsoft\Input\TSF\Tsf3Override\{03b5835f-f03c-411b-9ce2-aa23e1171e36}'
$spike = (Resolve-Path 'dist\ime_key_matrix_spike.exe').Path

function Restart-Ctfmon {
  Get-Process ctfmon -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep -Seconds 2; Start-Process ctfmon.exe -ErrorAction SilentlyContinue; Start-Sleep -Seconds 4
}
function Run-Harness([string]$tag, [string]$seq) {
  $dist = Split-Path $spike
  Remove-Item (Join-Path $dist 'ime_key_matrix_spike.log') -ErrorAction SilentlyContinue
  Push-Location $dist
  $null = Start-Process -FilePath $spike -ArgumentList "--auto --hold=180 --activate-gji --msime --seq=$seq" -PassThru
  $deadline = (Get-Date).AddSeconds(90)
  while ((Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 700
    if ((Test-Path ime_key_matrix_spike.log) -and (Select-String -Path ime_key_matrix_spike.log -Pattern '全手順完了' -Quiet)) { break }
  }
  Get-Process ime_key_matrix_spike -ErrorAction SilentlyContinue | Stop-Process -Force
  Pop-Location
  $lf = Join-Path $dist 'ime_key_matrix_spike.log'
  if (Test-Path $lf) {
    $lines = Get-Content $lf -Encoding utf8
    $i = 0; $hit = $false
    for ($i = 0; $i -lt $lines.Count; $i++) {
      if ($lines[$i] -match '\] KEY \[SCRIPT') {
        for ($j = $i + 1; $j -lt [Math]::Min($i + 8, $lines.Count); $j++) { if ($lines[$j] -match '差分') { Say ("[{0}] {1}" -f $tag, ($lines[$j].Trim())); $hit = $true; break } }
      }
    }
    if (-not $hit) { Say "[$tag] (KEY 行なし)" }
  } else { Say "[$tag] no harness log" }
}

function Encode-Table([byte[]]$src, [string]$row, [string]$code) {
  $recs = New-Object System.Collections.Generic.List[byte[]]
  $cur = New-Object System.Collections.Generic.List[byte]
  foreach ($b in $src) { if ($b -eq 0) { if ($cur.Count -gt 0) { $recs.Add($cur.ToArray()); $cur.Clear() } } else { $cur.Add($b) } }
  $out = New-Object System.Collections.Generic.List[byte]
  $done = $false
  foreach ($r in $recs) {
    if ($sjis.GetString($r).StartsWith([string]"$row=")) { $r = $sjis.GetBytes("$row=$code $code $code $code $code $code"); $done = $true }
    $out.AddRange($r); $out.Add(0)
  }
  if (-not $done) { $out.AddRange($sjis.GetBytes("$row=$code $code $code $code $code $code")); $out.Add(0) }
  $out.Add(0)
  return [byte[]]$out.ToArray()
}
# Custom を NATURAL のコピーで作り、指定の表の指定行を書き換える
function Build-Custom([string]$row, [string]$code, [string[]]$tables) {
  $nat = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("$imejp\StyleList\NATURAL")
  $cust = "HKCU:\$imejp\StyleList\Custom"
  if (Test-Path $cust) { Remove-Item $cust -Recurse -Force }
  New-Item -Path $cust -Force | Out-Null
  foreach ($n in $nat.GetValueNames()) {
    $v = [byte[]]$nat.GetValue($n)
    if ($tables -contains $n) { $v = Encode-Table $v $row $code }
    Set-ItemProperty -Path $cust -Name $n -Value $v -Type Binary -Force
  }
  Set-ItemProperty -Path $msimePath -Name keystyle -Value 'Custom' -Type String -Force
}
function Set-Direct([int]$noDirect) { Set-ItemProperty -Path $msimePath -Name NoDirectInputMode -Value $noDirect -Type DWord -Force }

New-Item -Path $tsf -Force | Out-Null
Set-ItemProperty -Path $tsf -Name NoTsf3Override2 -Value 1 -Type DWord
New-Item -Path $msimePath -Force | Out-Null
Set-ItemProperty -Path $msimePath -Name DisableNewIME -Value 1 -Type DWord
Restart-Ctfmon
$exe = (Get-ChildItem "$env:windir\System32\IME" -Recurse -Filter 'IMJPUEXC.EXE' | Select-Object -First 1).FullName
& $exe setkeytemplate 'Microsoft_IME' 2>&1 | Out-Null
Say "NoDirectInputMode(既定)=$((Get-ItemProperty $msimePath).NoDirectInputMode) keystyle=$((Get-ItemProperty $msimePath).keystyle)"

foreach ($nd in 1, 0) {
  Set-Direct $nd
  Say "##### NoDirectInputMode=$nd"
  # 対照: Custom=NATURAL の無加工コピー
  Build-Custom '変換' '87' @()
  Run-Harness "nd$nd-Custom無加工-変換" '1C'
  Run-Harness "nd$nd-Custom無加工-無変換" '1D'
  foreach ($tabs in @(@('S4key'), @('key'), @('key','S4key'))) {
    $label = $tabs -join '+'
    Build-Custom '変換' '00' $tabs
    Run-Harness "nd$nd-Custom-$label-変換=00" '1C'
    Build-Custom '無変換' '87' $tabs
    Run-Harness "nd$nd-Custom-$label-無変換=87" '1D'
    Build-Custom '無変換' 'CE' $tabs
    Run-Harness "nd$nd-Custom-$label-無変換=CE" '1D'
  }
}
Say '=== done ==='
