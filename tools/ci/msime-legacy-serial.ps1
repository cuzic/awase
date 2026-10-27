# 仮説: 旧UIの表の書き換えが効かないのは、UI の Apply が書く MSIME\SerialNo(設定の通し番号)を IME が見て再読込するから。
# 変換(0x1C)の行を 00 で埋めた表を書き、SerialNo の更新有無・ctfmon 再起動の有無で、直接入力中の変換キーの実効果を比べる。
param([string]$Out = 'serial-out')
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force -Path $Out | Out-Null
try { [Text.Encoding]::RegisterProvider([Text.CodePagesEncodingProvider]::Instance) } catch {}
$sjis = [Text.Encoding]::GetEncoding(932)
$log = Join-Path $Out 'serial.txt'
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
function Set-Row([string]$row, [string]$code) {
  $base = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("$imejp\StyleList\NATURAL")
  $baseKey = $base.GetValue('key')
  $recs = New-Object System.Collections.Generic.List[byte[]]
  $cur = New-Object System.Collections.Generic.List[byte]
  foreach ($b in $baseKey) { if ($b -eq 0) { if ($cur.Count -gt 0) { $recs.Add($cur.ToArray()); $cur.Clear() } } else { $cur.Add($b) } }
  $out = New-Object System.Collections.Generic.List[byte]
  foreach ($r in $recs) {
    if ($sjis.GetString($r).StartsWith([string]"$row=")) { $r = $sjis.GetBytes("$row=$code $code $code $code $code $code") }
    $out.AddRange($r); $out.Add(0)
  }
  $out.Add(0)
  Set-ItemProperty -Path "HKCU:\$imejp\StyleList\NATURAL" -Name key -Value ([byte[]]$out.ToArray()) -Type Binary -Force
}
function Serial { (Get-ItemProperty $msimePath -ErrorAction SilentlyContinue).SerialNo }

New-Item -Path $tsf -Force | Out-Null
Set-ItemProperty -Path $tsf -Name NoTsf3Override2 -Value 1 -Type DWord
New-Item -Path $msimePath -Force | Out-Null
Set-ItemProperty -Path $msimePath -Name DisableNewIME -Value 1 -Type DWord
Restart-Ctfmon
$exe = (Get-ChildItem "$env:windir\System32\IME" -Recurse -Filter 'IMJPUEXC.EXE' | Select-Object -First 1).FullName
& $exe setkeytemplate 'Microsoft_IME' 2>&1 | Out-Null
Say "SerialNo after setkeytemplate Microsoft_IME = $(Serial)  keystyle=$((Get-ItemProperty $msimePath).keystyle)"

Run-Harness 'V0-default(変換=87のまま)' '1C'
Set-Row '変換' '00'
Run-Harness 'V1-変換=00・Serial更新なし' '1C'
$s0 = [int](Serial); if (-not $s0) { $s0 = 0 }
Set-ItemProperty -Path $msimePath -Name SerialNo -Value ($s0 + 1) -Type DWord
Say "SerialNo -> $(Serial)"
Run-Harness 'V2-変換=00・SerialNo+1' '1C'
Restart-Ctfmon
Run-Harness 'V3-変換=00・SerialNo+1・ctfmon再起動' '1C'
# 対照: ツール経由(ATOK)なら SerialNo は動くか、効くか
& $exe setkeytemplate 'ATOK' 2>&1 | Out-Null
Say "SerialNo after setkeytemplate ATOK = $(Serial) keystyle=$((Get-ItemProperty $msimePath).keystyle) option2=$((Get-ItemProperty $msimePath).option2)"
Run-Harness 'V4-ATOK(ツール)' '1C'
Say '=== done ==='
