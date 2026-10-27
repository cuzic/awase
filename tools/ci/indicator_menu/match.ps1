# 実機用: MS-IME の自前の窓(indicator_scenario)に IMM で状態を書き、右クリックメニューの「選択中の項目 ID」を読み、
# 実状態(TRUTH)と突き合わせる。記録するのは状態の数値と項目 ID だけ。他のアプリの文字列・本文は読まない・保存しない。
param([string]$Bin = 'C:\Users\cuzic\awase-menu\target\debug\examples', [string]$Out = 'C:\Users\cuzic\menu-match-out', [int]$Rounds = 4)
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force -Path $Out | Out-Null
Add-Type -AssemblyName System.Windows.Forms, System.Drawing, UIAutomationClient, UIAutomationTypes
Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public static class MM { [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, UIntPtr e); }'
function Log($m) { $l = "[$([DateTime]::UtcNow.ToString('HH:mm:ss.fffZ'))] $m"; Write-Host $l; Add-Content -Path "$Out\run.log" -Value $l -Encoding utf8 }
function Pump($sec) { $end = [DateTime]::UtcNow.AddSeconds($sec); while ([DateTime]::UtcNow -lt $end) { [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 20 } }
$A = [System.Windows.Automation.AutomationElement]; $T = [System.Windows.Automation.TreeScope]; $tc = [System.Windows.Automation.Condition]::TrueCondition
function Find-Indicator {
  foreach ($x in $A::RootElement.FindAll($T::Children, $tc)) { if ($x.Current.ClassName -eq 'Shell_TrayWnd') { $tray = $x } }
  if (-not $tray) { return $null }
  foreach ($e in $tray.FindAll($T::Descendants, $tc)) { if ($e.Current.Name -match '^(Tray Input Indicator|トレイ入力インジケーター)' -and $e.Current.Name -match 'IME のオプション|IME options|Right click|右クリック') { return $e } }
  return $null
}
function Read-Selected {
  $sel = @(); $n = 0
  $cond = New-Object System.Windows.Automation.PropertyCondition($A::ControlTypeProperty, [System.Windows.Automation.ControlType]::MenuItem)
  foreach ($m in $A::RootElement.FindAll($T::Descendants, $cond)) {
    $n++; $o = $null
    if ($m.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$o)) { if ($o.Current.IsSelected) { $sel += $m.Current.AutomationId } }
  }
  return @($n, ($sel -join '+'))
}
$aw = Get-Process awase -ErrorAction SilentlyContinue | Select-Object -First 1
$awPath = $null; if ($aw) { $awPath = $aw.Path }
Stop-Process -Name awase -Force -ErrorAction SilentlyContinue
Log "awase 停止(再開パス=$awPath)"
$states = @('1:19', '1:1b', '1:13', '1:18', '1:10', '0:19')
"round`tset`ttruth_open`ttruth_conv`tmenu_items`tmenu_selected" | Set-Content "$Out\match.tsv" -Encoding utf8
try {
  for ($r = 1; $r -le $Rounds; $r++) {
    foreach ($s in ($states | Get-Random -Count $states.Count)) {
      $d = "$Out\r$r-$($s -replace ':','_')"; New-Item -ItemType Directory -Force -Path $d | Out-Null
      $p = Start-Process -FilePath "$Bin\indicator_scenario.exe" -ArgumentList "--scen=cycle --msime --steps=$s --n=1 --hold=25 --log=$d\scen.log" -PassThru -WindowStyle Hidden
      Pump 9
      $truth = (Get-Content "$d\scen.log" -ErrorAction SilentlyContinue | Where-Object { $_ -match 'TRUTH' } | Select-Object -Last 1)
      $to = ''; $tcv = ''
      if ($truth -match 'open=(\S+) conv=(\S+)') { $to = $Matches[1]; $tcv = $Matches[2] }
      $ind = Find-Indicator
      if (-not $ind) { Log "r$r $s : インジケーター無し"; Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; continue }
      $rc = $ind.Current.BoundingRectangle
      [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point([int](($rc.Left + $rc.Right) / 2), [int](($rc.Top + $rc.Bottom) / 2)); Start-Sleep -Milliseconds 80
      [MM]::mouse_event(0x8, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 100; [MM]::mouse_event(0x10, 0, 0, 0, [UIntPtr]::Zero)
      Pump 1.5
      $res = Read-Selected
      [System.Windows.Forms.SendKeys]::SendWait('{ESC}'); Pump 1
      "$r`t$s`t$to`t$tcv`t$($res[0])`t$($res[1])" | Add-Content "$Out\match.tsv" -Encoding utf8
      Log "r$r set=$s truth=open:$to conv:$tcv menu=items:$($res[0]) selected:$($res[1])"
      Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; Pump 1.5
    }
  }
} finally {
  & "$Bin\indicator_scenario.exe" --scen=cycle --n=0 "--log=$Out\restore.log" | Out-Null
  if ($awPath) { Start-Process -FilePath $awPath -WorkingDirectory (Split-Path $awPath) -WindowStyle Hidden; Log 'awase 再開' }
  Log '完了'
}
