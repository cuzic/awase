# ADR-265 belief experiment E1 (automatic): drive the taskbar input indicator's right-click menu with UI Automation.
# usage: e1_uia.ps1 -Mode dump|run -Log <jsonl> [-Rounds 10]
#   dump: open the menu once, write the item names / AutomationIds, close it with Escape (selects nothing).
#   run : alternate "direct input" and "hiragana" items (Rounds each), logging the UTC time of every action.
# Menu text is the TIP's own item names; only names and AutomationIds are logged (never document text).
param([string]$Mode = 'dump', [string]$Log = 'e1_uia.jsonl', [int]$Rounds = 10)
$ErrorActionPreference = 'Continue'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class Mx { [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte sc, uint f, UIntPtr e); }
"@
function Emit($o) { $o['t_utc'] = (Get-Date).ToUniversalTime().ToString('o'); Add-Content -Path $Log -Value ($o | ConvertTo-Json -Compress -Depth 4) -Encoding utf8 }
[void][Mx]::SetProcessDPIAware()  # UIA rectangles are physical pixels; a DPI-unaware process would move the cursor to the wrong place
$AE = [System.Windows.Automation.AutomationElement]
$root = $AE::RootElement
function Find-Indicator {
  $tray = $root.FindFirst('Children', (New-Object System.Windows.Automation.PropertyCondition($AE::ClassNameProperty, 'Shell_TrayWnd')))
  if (-not $tray) { return $null }
  $all = $tray.FindAll('Descendants', [System.Windows.Automation.Condition]::TrueCondition)
  foreach ($e in $all) {
    $cn = $e.Current.ClassName
    if ($cn -like 'SystemTray.*Button' -and ($e.Current.Name -match 'Tray Input Indicator|トレイ入力インジケーター|入力インジケーター')) { return $e }
  }
  return $null
}
function Top-Windows { $o=@(); foreach ($c in $root.FindAll('Children', [System.Windows.Automation.Condition]::TrueCondition)) { $o += ("{0}|{1}" -f $c.Current.ClassName, $c.Current.ProcessId) }; return $o }
function Open-Menu($ind) {
  $before = Top-Windows
  $r = $ind.Current.BoundingRectangle
  $x = [int]($r.X + $r.Width / 2); $y = [int]($r.Y + $r.Height / 2)
  [void][Mx]::SetCursorPos($x, $y); Start-Sleep -Milliseconds 150
  [Mx]::mouse_event(8, 0, 0, 0, [UIntPtr]::Zero); [Mx]::mouse_event(16, 0, 0, 0, [UIntPtr]::Zero)  # right down / up
  $t0 = Get-Date
  $script:diagAfter = $null
  while (((Get-Date) - $t0).TotalSeconds -lt 3) {
    $m = $root.FindFirst('Children', (New-Object System.Windows.Automation.PropertyCondition($AE::ClassNameProperty, '#32768')))
    if ($m) { return $m }
    Start-Sleep -Milliseconds 30
  }
  $after = Top-Windows
  Emit @{ event = 'diag_no_menu'; click = "$x,$y"; new_windows = @($after | Where-Object { $_ -notin $before }); gone_windows = @($before | Where-Object { $_ -notin $after }); cursor = ([System.Windows.Forms.Cursor]::Position.ToString()) }
  return $null
}
function Menu-Items($menu) {
  $items = $menu.FindAll('Descendants', (New-Object System.Windows.Automation.PropertyCondition($AE::ControlTypeProperty, [System.Windows.Automation.ControlType]::MenuItem)))
  $out = @(); foreach ($i in $items) { $out += [pscustomobject]@{ name = $i.Current.Name; id = $i.Current.AutomationId; el = $i } }
  return $out
}
function Close-Menu { [Mx]::keybd_event(0x1B, 0, 0, [UIntPtr]::Zero); [Mx]::keybd_event(0x1B, 0, 2, [UIntPtr]::Zero); Start-Sleep -Milliseconds 300 }

$ind = Find-Indicator
if (-not $ind) { Emit @{ event = 'abort'; reason = 'indicator element not found' }; exit 2 }
$rc = $ind.Current.BoundingRectangle
Emit @{ event = 'indicator'; name = $ind.Current.Name; cls = $ind.Current.ClassName; rect = "$($rc.X),$($rc.Y),$($rc.Width),$($rc.Height)" }
if ($Mode -eq 'dump') {
  $m = Open-Menu $ind
  if (-not $m) { Emit @{ event = 'abort'; reason = 'menu did not open' }; exit 3 }
  $its = Menu-Items $m
  Emit @{ event = 'menu'; count = @($its).Count; items = @($its | ForEach-Object { @{ name = $_.name; id = $_.id } }) }
  Close-Menu
  Emit @{ event = 'done'; mode = 'dump' }
  exit 0
}
# run: step i even = pick "direct input" (close), odd = pick "hiragana" (open)
for ($i = 0; $i -lt ($Rounds * 2); $i++) {
  $want = if ($i % 2 -eq 0) { 0 } else { 1 }
  $pat = if ($want -eq 0) { '直接入力|Direct' } else { 'ひらがな|Hiragana' }
  $m = Open-Menu $ind
  if (-not $m) { Emit @{ event = 'step_failed'; step = $i; reason = 'menu did not open' }; Start-Sleep 1; continue }
  $its = Menu-Items $m
  $target = $its | Where-Object { $_.name -match $pat } | Select-Object -First 1
  if (-not $target) { Emit @{ event = 'step_failed'; step = $i; reason = 'item not found'; names = @($its | ForEach-Object { $_.name }) }; Close-Menu; continue }
  $inv = $target.el.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
  Emit @{ event = 'action'; step = $i; want_open = $want; item_id = $target.id; item_name = $target.name }
  $inv.Invoke()
  Start-Sleep -Seconds 4
}
Emit @{ event = 'done'; mode = 'run' }
