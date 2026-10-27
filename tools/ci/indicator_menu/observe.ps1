# 実機観察: 利用者が手でインジケーターを右クリックしたとき、メニューが UIA でどう見えるかを記録する。注入はしない。
# 記録: クラス名・要素の種類・AutomationId・名前(IME のメニュー項目)・IsSelected。他の窓の内容は読まない。
param([string]$Out = 'C:\Users\cuzic\menu-diag-out', [int]$Seconds = 75)
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force -Path $Out | Out-Null
Add-Type -AssemblyName System.Windows.Forms, System.Drawing, UIAutomationClient, UIAutomationTypes
$A = [System.Windows.Automation.AutomationElement]; $TS = [System.Windows.Automation.TreeScope]; $tc = [System.Windows.Automation.Condition]::TrueCondition
function Log($m) { Add-Content -Path "$Out\observe.log" -Value ("[" + [DateTime]::UtcNow.ToString('HH:mm:ss.fff') + "Z] " + $m) -Encoding utf8 }
Remove-Item "$Out\observe.log" -ErrorAction SilentlyContinue
# CI の測定(run.ps1 の MW::Start)と同じく、UIA のイベントを購読してから待つ。購読が無いと Win32 メニューの項目が公開されないかを確かめる。
$handler = [System.Windows.Automation.AutomationEventHandler]{ param($sender, $e) }
if ($env:OBS_SUBSCRIBE -ne '0') {
  [System.Windows.Automation.Automation]::AddAutomationEventHandler($A::MenuOpenedEvent, $A::RootElement, $TS::Subtree, $handler)
  [System.Windows.Automation.Automation]::AddAutomationEventHandler([System.Windows.Automation.WindowPattern]::WindowOpenedEvent, $A::RootElement, $TS::Subtree, $handler)
  Log 'subscribed to UIA MenuOpened / WindowOpened'
}
$base = @{}; foreach ($x in $A::RootElement.FindAll($TS::Children, $tc)) { $base[$x.Current.ClassName] = 1 }
foreach ($x in $A::RootElement.FindAll($TS::Children, $tc)) { if ($x.Current.ClassName -eq 'Shell_TrayWnd') { $tray = $x } }
$tbase = $tray.FindAll($TS::Descendants, $tc).Count
Log "start base-toplevel=$($base.Count) tray-desc=$tbase"
$mi = New-Object System.Windows.Automation.PropertyCondition($A::ControlTypeProperty, [System.Windows.Automation.ControlType]::MenuItem)
$end = [DateTime]::UtcNow.AddSeconds($Seconds); $seen = @{}
function Dump-Menu($el) {
  $h = $el.Current.NativeWindowHandle
  Log "menu window hwnd=0x$($h.ToString('X')) rect=$([int]$el.Current.BoundingRectangle.Left),$([int]$el.Current.BoundingRectangle.Top),$([int]$el.Current.BoundingRectangle.Right),$([int]$el.Current.BoundingRectangle.Bottom) ctype=$($el.Current.ControlType.ProgrammaticName)"
  foreach ($d in $el.FindAll($TS::Descendants, $tc)) {
    $c = $d.Current; $o = $null; $sel = ''; $pats = ''
    try { $pats = ($d.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName.Replace('Identifiers.Pattern', '') }) -join '+' } catch {}
    if ($d.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$o)) { $sel = "selected=$($o.Current.IsSelected)" }
    elseif ($d.TryGetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern, [ref]$o)) { $sel = "toggle=$($o.Current.ToggleState)" }
    Log "  el ctype=$($c.ControlType.ProgrammaticName) id=$($c.AutomationId) class=$($c.ClassName) name=$($c.Name) pats=$pats $sel"
  }
}
while ([DateTime]::UtcNow -lt $end) {
  Start-Sleep -Milliseconds 120
  foreach ($x in $A::RootElement.FindAll($TS::Children, $tc)) {
    if ($x.Current.ClassName -eq '#32768') {
      $k = $x.Current.NativeWindowHandle
      if (-not $seen.ContainsKey($k)) {
        $seen[$k] = 1; Log "found #32768 hwnd=0x$($k.ToString('X'))"
        foreach ($wait in 0, 500, 1000) {
          Start-Sleep -Milliseconds $wait
          Log "--- dump +${wait}ms (cumulative)"
          try { $e2 = $A::FromHandle([IntPtr]$k); Dump-Menu $e2 } catch { Log "FromHandle failed: $($_.Exception.Message)" }
        }
      }
    }
  }
}
Log 'end'
