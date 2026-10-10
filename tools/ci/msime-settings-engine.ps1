# 設定アプリ(ms-settings:regionlanguage-jpnime)を UIA で読み、互換モードのチェックボックスの状態と
# 「キーの割り当て」セクションの有無を、レジストリのフラグの状態ごとに採取する(実際のエンジンの判別の手がかり)。観測のみ。
param([string]$Out = 'settings-out')
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$Out = (Resolve-Path $Out).Path
Add-Type -AssemblyName System.Drawing, System.Windows.Forms
$log = Join-Path $Out 'settings.txt'
function Say([string]$s) { $s | Tee-Object -FilePath $log -Append }
$imejp = 'Software\Microsoft\IME\15.0\IMEJP'
$msimePath = "HKCU:\$imejp\MSIME"
$tsf = 'HKCU:\SOFTWARE\Microsoft\Input\TSF\Tsf3Override\{03b5835f-f03c-411b-9ce2-aa23e1171e36}'
$probe = (Resolve-Path 'dist\msime_key_assignment_settings_probe.exe').Path

function Shot([string]$name) {
  try {
    $b = [Windows.Forms.SystemInformation]::VirtualScreen
    $bmp = New-Object Drawing.Bitmap $b.Width, $b.Height
    $g = [Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
    $bmp.Save((Join-Path $Out "$name.png"), [Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
  } catch { Say "shot failed: $_" }
}
function Set-Flags($tsfVal, $dnewVal) {
  New-Item -Path $msimePath -Force | Out-Null
  if ($null -eq $tsfVal) { Remove-ItemProperty -Path $tsf -Name NoTsf3Override2 -ErrorAction SilentlyContinue } else { New-Item -Path $tsf -Force | Out-Null; Set-ItemProperty -Path $tsf -Name NoTsf3Override2 -Value $tsfVal -Type DWord }
  if ($null -eq $dnewVal) { Remove-ItemProperty -Path $msimePath -Name DisableNewIME -ErrorAction SilentlyContinue } else { Set-ItemProperty -Path $msimePath -Name DisableNewIME -Value $dnewVal -Type DWord }
}
function Restart-Ctfmon {
  Get-Process ctfmon -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep -Seconds 2; Start-Process ctfmon.exe -ErrorAction SilentlyContinue; Start-Sleep -Seconds 4
}
$n = 0
foreach ($st in @(@($null, $null), @(1, $null), @(1, 1), @(0, 0), @($null, 1))) {
  foreach ($mode in '--general-only', '--dump-root') {
  $n++
  Set-Flags $st[0] $st[1]
  Restart-Ctfmon
  $tag = "s$n"
  Say "##### [$tag] NoTsf3Override2=$($st[0]) DisableNewIME=$($st[1]) mode=$mode"
  Push-Location (Split-Path $probe)
  Remove-Item msime_key_assignment_settings_probe.log -ErrorAction SilentlyContinue
  $p = Start-Process -FilePath $probe -ArgumentList $mode -PassThru -RedirectStandardOutput (Join-Path $Out "$tag-stdout.txt") -RedirectStandardError (Join-Path $Out "$tag-stderr.txt")
  Start-Sleep -Seconds 12
  Shot "$tag-a"
  if (-not $p.WaitForExit(90000)) { Say '(timeout, kill)'; Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
  Shot "$tag-b"
  if (Test-Path msime_key_assignment_settings_probe.log) {
    Copy-Item msime_key_assignment_settings_probe.log (Join-Path $Out "$tag-probe.log")
    Get-Content msime_key_assignment_settings_probe.log -Encoding utf8 | Where-Object { $_ -match 'RESULT|ompat|revious|KeyAssignment|toggle=|IsKeyAssignment|assign|Assign' } | Select-Object -First 40 | ForEach-Object { Say ("   " + $_) }
  } else { Say '   (no probe log)' }
  Pop-Location
  Get-Process SystemSettings -ErrorAction SilentlyContinue | Stop-Process -Force
  }
}
Say '=== done ==='
