# 旧UI(互換モード)のキー割り当てバイナリ表(StyleList\<style>\key)のコード体系を調べるスパイク。観測のみ。
# 1) 互換モードを ON にして IMJPUEXC.EXE の使い方と SETKEYTEMPLATE の候補を取る
# 2) テンプレートごとに StyleList の key をデコードして、テンプレート間の差分(=コードの意味の手がかり)を見る
param([string]$Out = 'spike-out')
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force -Path $Out | Out-Null
try { [Text.Encoding]::RegisterProvider([Text.CodePagesEncodingProvider]::Instance) } catch {}
$sjis = [Text.Encoding]::GetEncoding(932)
$log = Join-Path $Out 'spike.txt'
function Say([string]$s) { $s | Tee-Object -FilePath $log -Append }

$imejp = 'Software\Microsoft\IME\15.0\IMEJP'

function Decode-Key([byte[]]$b) {
  $t = $sjis.GetString($b)
  ($t -split "`0" | Where-Object { $_ -ne '' }) -join "`n"
}

function Dump-Ime([string]$tag) {
  Say "===== DUMP [$tag] ====="
  $msime = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("$imejp\MSIME")
  if ($msime) {
    foreach ($n in $msime.GetValueNames()) { Say ("MSIME\{0} = {1}" -f $n, $msime.GetValue($n)) }
  } else { Say 'MSIME key: (none)' }
  $sl = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("$imejp\StyleList")
  if (-not $sl) { Say 'StyleList: (none)'; return }
  foreach ($sub in $sl.GetSubKeyNames()) {
    $sk = $sl.OpenSubKey($sub)
    Say "--- StyleList\$sub"
    foreach ($n in $sk.GetValueNames()) {
      $v = $sk.GetValue($n)
      if ($v -is [byte[]]) {
        $safe = ($tag -replace '[^A-Za-z0-9_-]', '_') + '__' + $sub + '__' + $n + '.bin'
        [IO.File]::WriteAllBytes((Join-Path $Out $safe), $v)
        Say ("[{0}] binary {1} bytes" -f $n, $v.Length)
        Say (Decode-Key $v)
      } else { Say ("[{0}] = {1}" -f $n, $v) }
    }
  }
}

function Restart-Ctfmon {
  Get-Process ctfmon -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep -Seconds 2
  Start-Process ctfmon.exe -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 4
}

function Run-Tool([string]$exe, [string[]]$args_, [int]$timeoutSec = 20) {
  $so = Join-Path $Out 'tool-stdout.tmp'; $se = Join-Path $Out 'tool-stderr.tmp'
  $p = Start-Process -FilePath $exe -ArgumentList $args_ -PassThru -NoNewWindow -RedirectStandardOutput $so -RedirectStandardError $se
  if (-not $p.WaitForExit($timeoutSec * 1000)) { Say "(timeout $timeoutSec s, kill)"; Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
  $o = if (Test-Path $so) { Get-Content $so -Raw -Encoding Default } else { '' }
  $e = if (Test-Path $se) { Get-Content $se -Raw -Encoding Default } else { '' }
  Say ("exit={0} stdout=[{1}] stderr=[{2}]" -f $p.ExitCode, $o, $e)
  return "$o$e"
}

Say "OS: $([Environment]::OSVersion.VersionString)"
Say "--- IME files"
Get-ChildItem "$env:windir\System32\IME" -Recurse -Filter 'IMJPUEX*' -ErrorAction SilentlyContinue | ForEach-Object { Say $_.FullName }

Dump-Ime 'baseline'

# 互換モード ON(新UIのチェックボックス相当)。DisableNewIME は別系統の公開情報があるので両方試す。
$tsf = 'HKCU:\SOFTWARE\Microsoft\Input\TSF\Tsf3Override\{03b5835f-f03c-411b-9ce2-aa23e1171e36}'
New-Item -Path $tsf -Force | Out-Null
Set-ItemProperty -Path $tsf -Name NoTsf3Override2 -Value 1 -Type DWord
New-Item -Path "HKCU:\$imejp\MSIME" -Force | Out-Null
Set-ItemProperty -Path "HKCU:\$imejp\MSIME" -Name DisableNewIME -Value 1 -Type DWord
Restart-Ctfmon
Dump-Ime 'compat-on'

$exe = Get-ChildItem "$env:windir\System32\IME" -Recurse -Filter 'IMJPUEXC.EXE' -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $exe) { Say 'IMJPUEXC.EXE not found'; exit 0 }
Say "=== IMJPUEXC usage (no args) ==="
$usage = Run-Tool $exe.FullName @()
Say "=== IMJPUEXC /? ==="
$null = Run-Tool $exe.FullName @('/?')

# SETKEYTEMPLATE の候補を usage から拾う(a|b|c 形式)。拾えなければ推測の名前を使う。
$names = @()
foreach ($m in [regex]::Matches($usage, '(?i)setkeytemplate\s+([A-Za-z0-9_|\[\]-]+)')) {
  $names += ($m.Groups[1].Value -split '[|\[\]]' | Where-Object { $_ })
}
$names += @('MSIME','ATOK','WXG','VJE','NATURAL','KOTOERI','Custom','MS-IME')
$names = $names | Select-Object -Unique
Say ("template candidates: " + ($names -join ', '))

foreach ($n in $names) {
  Say "=== setkeytemplate $n ==="
  $null = Run-Tool $exe.FullName @('setkeytemplate', $n)
  Start-Sleep -Seconds 1
  Dump-Ime "tmpl-$n"
}
Say '=== done ==='
