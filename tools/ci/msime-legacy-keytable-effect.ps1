# 旧UI(互換モード)のキー表の「コード」が実際の IME 挙動に効くかを測るスパイク。観測のみ。
# A) テンプレート(ATOK/VJE/WX/MS-IME2000)ごとに SETKEYTEMPLATE → 無変換/変換の実効果を測る(表が効いているか)
# B) StyleList\Custom の「無変換」行を1つのコードで埋めて keystyle=Custom にし、無変換の実効果を測る(コード掃引)
param([string]$Out = 'effect-out', [string]$Key = '1D', [string]$Table = 'key', [switch]$SkipTemplates,
  [string]$Codes = '00,80,81,83,84,87,88,97,98,A2,A4,B3,C9,CA,CD,CE,CF,D5,F5,28,FF')
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force -Path $Out | Out-Null
try { [Text.Encoding]::RegisterProvider([Text.CodePagesEncodingProvider]::Instance) } catch {}
$sjis = [Text.Encoding]::GetEncoding(932)
$log = Join-Path $Out 'effect.txt'
function Say([string]$s) { $s | Tee-Object -FilePath $log -Append }
$imejp = 'Software\Microsoft\IME\15.0\IMEJP'
$tsf = 'HKCU:\SOFTWARE\Microsoft\Input\TSF\Tsf3Override\{03b5835f-f03c-411b-9ce2-aa23e1171e36}'
$spike = (Resolve-Path 'dist\ime_key_matrix_spike.exe').Path

function Restart-Ctfmon {
  Get-Process ctfmon -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep -Seconds 2
  Start-Process ctfmon.exe -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 4
}

# ハーネスを1回走らせて KEY 行を返す
function Run-Harness([string]$tag, [string]$seq) {
  $dist = Split-Path $spike
  Remove-Item (Join-Path $dist 'ime_key_matrix_spike.log') -ErrorAction SilentlyContinue
  Push-Location $dist
  $p = Start-Process -FilePath $spike -ArgumentList "--auto --hold=180 --activate-gji --msime --seq=$seq" -PassThru
  $deadline = (Get-Date).AddSeconds(90)
  while ((Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 700
    if ((Test-Path ime_key_matrix_spike.log) -and (Select-String -Path ime_key_matrix_spike.log -Pattern '全手順完了' -Quiet)) { break }
  }
  Get-Process ime_key_matrix_spike -ErrorAction SilentlyContinue | Stop-Process -Force
  Pop-Location
  $lf = Join-Path $dist 'ime_key_matrix_spike.log'
  if (Test-Path $lf) {
    Copy-Item $lf (Join-Path (Join-Path $PSScriptRoot '..\..') "$Out\harness-$tag.log") -ErrorAction SilentlyContinue
    $keys = Get-Content $lf -Encoding utf8 | Where-Object { $_ -match '\] KEY \[(SCRIPT|SEQ|seq)|\] KEY .*(無変換|変換)|差分|\[FATAL\]' }
    Say "--- [$tag] seq=$seq"
    $keys | ForEach-Object { Say $_ }
  } else { Say "--- [$tag] no harness log" }
}

# 互換モード ON
New-Item -Path $tsf -Force | Out-Null
Set-ItemProperty -Path $tsf -Name NoTsf3Override2 -Value 1 -Type DWord
New-Item -Path "HKCU:\$imejp\MSIME" -Force | Out-Null
Set-ItemProperty -Path "HKCU:\$imejp\MSIME" -Name DisableNewIME -Value 1 -Type DWord
Restart-Ctfmon

$exe = (Get-ChildItem "$env:windir\System32\IME" -Recurse -Filter 'IMJPUEXC.EXE' | Select-Object -First 1).FullName
Say '=== help setkeytemplate ==='
Say ((& $exe help setkeytemplate 2>&1) -join "`n")

Say '=== A) テンプレートごとの実効果 ==='
if (-not $SkipTemplates) {
Run-Harness 'none' '1D,1C'
foreach ($t in 'Microsoft_IME', 'IME_Standard', 'ATOK', 'VJE', 'WX') {
  & $exe setkeytemplate $t 2>&1 | Out-Null
  Say ("template=$t keystyle=" + (Get-ItemProperty "HKCU:\$imejp\MSIME").keystyle)
  Run-Harness "tmpl-$t" '1D,1C'
}
}

Say '=== B) 名前付きスタイル(NATURAL)の表を直接書き換えた掃引(keystyle=NATURAL のまま) ==='
& $exe setkeytemplate 'Microsoft_IME' 2>&1 | Out-Null
$natPath = "HKCU:\$imejp\StyleList\NATURAL"
$base = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("$imejp\StyleList\NATURAL")
if (-not $base) { Say 'NATURAL style missing'; exit 0 }
$baseKey = $base.GetValue($Table)
Say "table=$Table bytes=$($baseKey.Length)"
$rowName = if ($Key -eq '1D') { '無変換' } else { '変換' }
Say "row=$rowName seq=$Key keystyle=$((Get-ItemProperty "HKCU:\$imejp\MSIME").keystyle)"
foreach ($code in ($Codes -split ',')) {
  $recs = New-Object System.Collections.Generic.List[byte[]]
  $cur = New-Object System.Collections.Generic.List[byte]
  foreach ($b in $baseKey) { if ($b -eq 0) { if ($cur.Count -gt 0) { $recs.Add($cur.ToArray()); $cur.Clear() } } else { $cur.Add($b) } }
  $out = New-Object System.Collections.Generic.List[byte]
  $replaced = $false
  foreach ($r in $recs) {
    $str = $sjis.GetString($r)
    if ($str.StartsWith([string]"$rowName=")) { $r = $sjis.GetBytes("$rowName=$code $code $code $code $code $code"); $replaced = $true }
    $out.AddRange($r); $out.Add(0)
  }
  if (-not $replaced) { $out.AddRange($sjis.GetBytes("$rowName=$code $code $code $code $code $code")); $out.Add(0) }
  $out.Add(0)
  Set-ItemProperty -Path $natPath -Name $Table -Value ([byte[]]$out.ToArray()) -Type Binary -Force
  Run-Harness "$Table-$rowName-$code" $Key
}
Say '=== done ==='
