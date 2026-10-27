# ADR-264 段階 1 の実機(windows-latest)検証: `[diagnostics] indicator_menu_shadow = true` の awase を起動し、
# 入力インジケーターを右クリックしたとき、awase.log に期待どおりの `[indicator-menu]`(journal)が残るかを確かめる。
# 否定の対照: (1)設定が無効なら何も出ない (2)タスクバー以外のメニュー(自前の入力欄の標準メニュー)では読まない。
# 使い方: run.ps1 -Ime msime|gji -Dist dist -Out out
param([Parameter(Mandatory)][string]$Ime, [string]$Dist = 'dist', [string]$Out = 'out')
$ErrorActionPreference = 'Continue'
$Dist = (Resolve-Path $Dist).Path
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$Out = (Resolve-Path $Out).Path
Add-Type -AssemblyName System.Windows.Forms, System.Drawing, UIAutomationClient, UIAutomationTypes
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices;
public static class Ms {
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern IntPtr FindWindow(string c, string t);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int l,t,r,b; }
}
"@
function Log($m) { $l = "[$([DateTime]::UtcNow.ToString('HH:mm:ss.fffZ'))] $m"; Write-Host $l; Add-Content -Path "$Out\run.log" -Value $l -Encoding utf8 }
function Shot($name) {
  try { $b = [System.Windows.Forms.SystemInformation]::VirtualScreen; $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
    $bmp.Save("$Out\$name.png", [System.Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $bmp.Dispose() } catch { Log "shot $name 失敗" }
}
function Find-Indicator {
  $A=[System.Windows.Automation.AutomationElement]; $T=[System.Windows.Automation.TreeScope]; $tc=[System.Windows.Automation.Condition]::TrueCondition
  $tray = $null
  foreach($x in $A::RootElement.FindAll($T::Children,$tc)){ if($x.Current.ClassName -eq 'Shell_TrayWnd'){ $tray=$x } }
  if(-not $tray){ return $null }
  foreach($e in $tray.FindAll($T::Descendants,$tc)){ if($e.Current.Name -match '^(Tray Input Indicator|トレイ入力インジケーター)'){ return $e } }
  return $null
}
function Click-Right($x, $y) {
  [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point($x, $y); Start-Sleep -Milliseconds 800
  [Ms]::mouse_event(0x8,0,0,0,[UIntPtr]::Zero); [Ms]::mouse_event(0x10,0,0,0,[UIntPtr]::Zero)
}
function Start-Awase($d, $cls, [bool]$shadow) {
  $w = "$d\awase"; New-Item -ItemType Directory -Force -Path $w | Out-Null
  Copy-Item "$Dist\awase.exe" $w -Force
  $flag = if ($shadow) { 'true' } else { 'false' }
  "[general]`n`n[diagnostics]`nindicator_menu_shadow = $flag`n" | Set-Content -Path "$w\config.toml" -Encoding utf8
  "[imm_capability.`"indicator_scenario.exe`"]`nScenWnd = `"$cls`"`nEDIT = `"$cls`"`n" | Set-Content -Path "$w\cache.toml" -Encoding utf8
  $env:RUST_LOG = 'debug'; $env:AWASE_TEST_INJECTION = '1'
  $p = Start-Process -FilePath "$w\awase.exe" -WorkingDirectory $w -PassThru -WindowStyle Hidden
  $t0 = Get-Date
  while (((Get-Date) - $t0).TotalSeconds -lt 20) {
    Start-Sleep -Milliseconds 500
    if ((Test-Path "$w\awase.log") -and (Select-String -Path "$w\awase.log" -Pattern 'Hook installed|Running message loop' -Quiet)) { break }
  }
  Start-Sleep -Seconds 4
  Log "awase 起動 pid=$($p.Id) shadow=$flag cls=$cls"
  return $p
}

# 判定の結果を集める
$results = @()
function Run-Case($tag, $scenArgs, $cls, [bool]$shadow, $expect, [string]$rightClickAt = 'indicator') {
  $d = "$Out\$tag"; New-Item -ItemType Directory -Force -Path $d | Out-Null
  Log "=== $tag 開始($scenArgs, shadow=$shadow, 右クリック=$rightClickAt, 期待=$($expect | ConvertTo-Json -Compress))"
  $ap = Start-Awase $d $cls $shadow
  $sp = Start-Process -FilePath "$Dist\indicator_scenario.exe" -ArgumentList "--scen=cycle $scenArgs --n=1 --hold=45 --log=$d\scen.log" -PassThru -WindowStyle Hidden
  Start-Sleep -Seconds 11
  if ($rightClickAt -eq 'indicator') {
    $ind = Find-Indicator
    if (-not $ind) { Log 'インジケーターが見つからない' }
    else {
      $r = $ind.Current.BoundingRectangle
      Click-Right ([int](($r.Left+$r.Right)/2)) ([int](($r.Top+$r.Bottom)/2))
    }
  } else {
    # タスクバー以外: 自前の窓(ScenWnd)の中央を右クリックし、入力欄の標準メニュー(#32768)を出す
    $h = [Ms]::FindWindow('ScenWnd', $null); $rc = New-Object Ms+RECT
    if ($h -ne [IntPtr]::Zero -and [Ms]::GetWindowRect($h, [ref]$rc)) {
      Click-Right ([int](($rc.l+$rc.r)/2)) ([int](($rc.t+$rc.b)/2))
    } else { Log 'ScenWnd が見つからない' }
  }
  Start-Sleep -Seconds 4
  Shot "$tag-menu"
  Stop-Process -Id $sp.Id -Force -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 2
  Stop-Process -Id $ap.Id -Force -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 1
  $log = "$d\awase\awase.log"
  if (Test-Path $log) { Copy-Item $log "$d\awase.log" -Force }
  $lines = @(); if (Test-Path $log) { $lines = @(Select-String -Path $log -Pattern '\[indicator-menu\]' | ForEach-Object { $_.Line }) }
  $lines | Out-File -FilePath "$d\indicator-menu.lines.txt" -Encoding utf8
  $requested = @($lines | Where-Object { $_ -match 'read requested' })
  $started   = @($lines | Where-Object { $_ -match 'worker thread started|hook installed' })
  $records   = @($lines | Where-Object { $_ -match 'verdict' })
  $ok = $false; $why = ''
  if ($null -eq $expect) {
    $ok = ($requested.Count -eq 0 -and $records.Count -eq 0)
    $why = "依頼 $($requested.Count) 件・記録 $($records.Count) 件(期待: どちらも 0)"
  } elseif ($records.Count -eq 0) {
    $why = "記録が 0 件(依頼 $($requested.Count) 件)"
  } else {
    $last = $records[-1]
    $miss = @()
    foreach ($k in $expect.Keys) { if ($last -notmatch [regex]::Escape($expect[$k])) { $miss += "$k=$($expect[$k])" } }
    $ok = ($miss.Count -eq 0)
    $why = if ($ok) { "記録 $($records.Count) 件。最後: $($last.Substring([Math]::Max(0,$last.IndexOf('[indicator-menu]'))))" } else { "不一致: $($miss -join ', ')。最後: $last" }
  }
  $script:results += [pscustomobject]@{ case = $tag; pass = $ok; detail = $why; started = $started.Count }
  Log "結果 $tag : $(if ($ok) { 'PASS' } else { 'FAIL' }) — $why"
}

Log "ime=$Ime"
if ($Ime -eq 'msime') {
  Run-Case 'msime-normal-hira'     '--msime --steps=1:19'                          'works'       $true  @{ ime='ms-ime'; verdict='hit'; mode='hiragana' }
  Run-Case 'msime-tsf-hira'        '--msime --win=tsf --imm=assoc --keys=F2'       'unavailable' $true  @{ ime='ms-ime'; verdict='hit'; mode='hiragana' }
  Run-Case 'msime-normal-halfalnum' '--msime --steps=0:10'                        'works'       $true  @{ ime='ms-ime'; verdict='hit'; mode='half-alnum' }
  Run-Case 'msime-disabled'        '--msime --steps=1:19'                          'works'       $false $null
  Run-Case 'msime-non-taskbar-menu' '--msime --steps=1:19'                         'works'       $true  $null 'window'
} else {
  Run-Case 'gji-normal-on'         '--keys=16'                                     'works'       $true  @{ ime='gji'; verdict='hit'; mode='hiragana' }
  Run-Case 'gji-normal-off'        '--keys=1A'                                     'works'       $true  @{ ime='gji'; verdict='hit'; mode='direct-input' }
  Run-Case 'gji-tsf-on'            '--win=tsf --imm=assoc --keys=16'               'unavailable' $true  @{ ime='gji'; verdict='hit'; mode='hiragana' }
  Run-Case 'gji-disabled'          '--keys=16'                                     'works'       $false $null
  Run-Case 'gji-non-taskbar-menu'  '--keys=16'                                     'works'       $true  $null 'window'
}

# 概要
$md = @("# ADR-264 段階 1 の実機検証($Ime)", "", "| ケース | 結果 | 詳細 |", "|---|---|---|")
foreach ($r in $results) { $md += "| $($r.case) | $(if ($r.pass) { 'PASS' } else { 'FAIL' }) | $($r.detail -replace '\|','/') |" }
$md | Out-File -FilePath "$Out\summary.md" -Encoding utf8
$fail = @($results | Where-Object { -not $_.pass })
Log "完了: PASS $(@($results | Where-Object { $_.pass }).Count) / FAIL $($fail.Count)"
if ($fail.Count -gt 0) { exit 1 }
