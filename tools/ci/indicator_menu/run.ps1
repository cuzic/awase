# ADR-261 の延長: 利用者がタスクバーの入力インジケーターを右クリックしてメニューを開いたとき、(1)開いた瞬間を検知できるか(WinEvent と UI Automation のイベント)、
# (2)メニューの項目と「現在のモード」(チェック済みの項目)を読めるかを測る。測定専用。
# 使い方: run.ps1 -Ime msime|gji -Dist dist -Out out
param([Parameter(Mandatory)][string]$Ime, [string]$Dist = 'dist', [string]$Out = 'out')
$ErrorActionPreference = 'Continue'
$Dist = (Resolve-Path $Dist).Path
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$Out = (Resolve-Path $Out).Path
Add-Type -AssemblyName System.Windows.Forms, System.Drawing, UIAutomationClient, UIAutomationTypes
$uiaDir = Split-Path ([System.Windows.Automation.AutomationElement].Assembly.Location)
$refs = @("$uiaDir\UIAutomationClient.dll", "$uiaDir\UIAutomationTypes.dll", "$uiaDir\WindowsBase.dll",
  'System.Collections', 'System.Collections.Concurrent', 'System.Runtime', 'System.Linq', 'System.Console')
foreach ($r in $refs[0..2]) { Write-Host "ref $r exists=$(Test-Path $r)" }
Write-Host ("refs: " + ($refs -join '; '))
try { Add-Type -ErrorAction Stop -ReferencedAssemblies $refs -TypeDefinition @"
using System; using System.Collections.Concurrent; using System.Runtime.InteropServices; using System.Text; using System.Windows.Automation;
public static class MW {
  public delegate void WinEventProc(IntPtr hook, uint ev, IntPtr hwnd, int idObj, int idChild, uint thread, uint time);
  [DllImport("user32.dll")] static extern IntPtr SetWinEventHook(uint min, uint max, IntPtr mod, WinEventProc cb, uint pid, uint tid, uint flags);
  [DllImport("user32.dll")] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, UIntPtr e);
  public static ConcurrentQueue<string> Q = new ConcurrentQueue<string>();
  static WinEventProc proc;
  static string Ts() { return DateTime.UtcNow.ToString("HH:mm:ss.fff") + "Z"; }
  static void OnEv(IntPtr hook, uint ev, IntPtr hwnd, int idObj, int idChild, uint thread, uint time) {
    StringBuilder c = new StringBuilder(128); StringBuilder t = new StringBuilder(128);
    if (hwnd != IntPtr.Zero) { GetClassName(hwnd, c, 128); GetWindowText(hwnd, t, 128); }
    string name = ev == 3 ? "FOREGROUND" : ev == 4 ? "MENUSTART" : ev == 5 ? "MENUEND" : ev == 6 ? "POPUPSTART" : ev == 7 ? "POPUPEND" :
      ev == 0x8000 ? "CREATE" : ev == 0x8002 ? "SHOW" : ev == 0x8003 ? "HIDE" : ev == 0x8005 ? "FOCUS" : ev.ToString("X");
    if ((ev == 0x8000 || ev == 0x8002 || ev == 0x8003) && idObj != 0) return;   // 窓そのもの以外の CREATE/SHOW/HIDE は捨てる
    Q.Enqueue(Ts() + " WINEVENT " + name + " hwnd=0x" + hwnd.ToString("X") + " class=" + c + " title=" + t + " visible=" + (hwnd != IntPtr.Zero && IsWindowVisible(hwnd)) + " idObj=" + idObj + " thread=" + thread);
  }
  public static void Start() {
    proc = new WinEventProc(OnEv);
    uint[][] r = { new uint[]{3,3}, new uint[]{4,7}, new uint[]{0x8000,0x8003}, new uint[]{0x8005,0x8005} };
    foreach (uint[] x in r) SetWinEventHook(x[0], x[1], IntPtr.Zero, proc, 0, 0, 0);
    Automation.AddAutomationEventHandler(AutomationElement.MenuOpenedEvent, AutomationElement.RootElement, TreeScope.Subtree, UiaH("UIA-MenuOpened"));
    Automation.AddAutomationEventHandler(AutomationElement.MenuClosedEvent, AutomationElement.RootElement, TreeScope.Subtree, UiaH("UIA-MenuClosed"));
    Automation.AddAutomationEventHandler(WindowPattern.WindowOpenedEvent, AutomationElement.RootElement, TreeScope.Subtree, UiaH("UIA-WindowOpened"));
    Automation.AddAutomationEventHandler(InvokePattern.InvokedEvent, AutomationElement.RootElement, TreeScope.Subtree, UiaH("UIA-Invoked"));
  }
  static AutomationEventHandler UiaH(string kind) {
    return delegate(object s, AutomationEventArgs e) {
      try { AutomationElement el = s as AutomationElement; var c = el.Current;
        Q.Enqueue(Ts() + " " + kind + " name=" + c.Name + " class=" + c.ClassName + " ctype=" + c.ControlType.ProgrammaticName + " rect=" + c.BoundingRectangle); }
      catch (Exception ex) { Q.Enqueue(Ts() + " " + kind + " (取得失敗: " + ex.Message + ")"); }
    };
  }
  public static string Describe(AutomationElement e) {
    var c = e.Current; string extra = "";
    object p;
    try { if (c.ItemStatus != null && c.ItemStatus.Length > 0) extra += " itemstatus=" + c.ItemStatus; } catch {}
    try { if (c.HelpText != null && c.HelpText.Length > 0) extra += " help=" + c.HelpText; } catch {}
    try { extra += " ids=" + string.Join(",", Array.ConvertAll(e.GetSupportedPatterns(), x => x.ProgrammaticName.Replace("Identifiers.Pattern", ""))); } catch {}
    try { if (e.TryGetCurrentPattern(TogglePattern.Pattern, out p)) extra += " toggle=" + ((TogglePattern)p).Current.ToggleState; } catch {}
    try { if (e.TryGetCurrentPattern(SelectionItemPattern.Pattern, out p)) extra += " selected=" + ((SelectionItemPattern)p).Current.IsSelected; } catch {}
    return c.ControlType.ProgrammaticName.Replace("ControlType.", "") + " | name=" + c.Name + " | class=" + c.ClassName + " | id=" + c.AutomationId + " | rect=" + (int)c.BoundingRectangle.Left + "," + (int)c.BoundingRectangle.Top + "," + (int)c.BoundingRectangle.Right + "," + (int)c.BoundingRectangle.Bottom + extra;
  }
}
"@ } catch { Write-Host "Add-Type 失敗: $($_.Exception.Message)"; exit 1 }
function Log($m) { $l = "[$([DateTime]::UtcNow.ToString('HH:mm:ss.fffZ'))] $m"; Write-Host $l; Add-Content -Path "$Out\run.log" -Value $l -Encoding utf8 }
function Pump($sec) { $end = [DateTime]::UtcNow.AddSeconds($sec); while ([DateTime]::UtcNow -lt $end) { [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 20 } }
function Drain($file) { $s = ''; $x = $null; while ([MW]::Q.TryDequeue([ref]$x)) { $s += $x + "`n" }; if ($s) { Add-Content -Path $file -Value $s -Encoding utf8 }; return $s }
function Shot($name) {
  try { $b = [System.Windows.Forms.SystemInformation]::VirtualScreen; $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
    $bmp.Save("$Out\$name.png", [System.Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $bmp.Dispose() } catch { Log "shot $name 失敗" }
}
function Find-Indicator {
  $A=[System.Windows.Automation.AutomationElement]; $T=[System.Windows.Automation.TreeScope]; $tc=[System.Windows.Automation.Condition]::TrueCondition
  foreach($x in $A::RootElement.FindAll($T::Children,$tc)){ if($x.Current.ClassName -eq 'Shell_TrayWnd'){ $tray=$x } }
  if(-not $tray){ return $null }
  foreach($e in $tray.FindAll($T::Descendants,$tc)){ if($e.Current.Name -match '^(Tray Input Indicator|トレイ入力インジケーター)'){ return $e } }
  return $null
}
function Dump-Subtree($el, $file, $label) {
  $T=[System.Windows.Automation.TreeScope]; $tc=[System.Windows.Automation.Condition]::TrueCondition
  Add-Content -Path $file -Value "### $label : $([MW]::Describe($el))" -Encoding utf8
  try { foreach($d in $el.FindAll($T::Descendants,$tc)){ Add-Content -Path $file -Value ("  " + [MW]::Describe($d)) -Encoding utf8 } } catch { Add-Content -Path $file -Value "  (子要素の取得失敗: $($_.Exception.Message))" -Encoding utf8 }
}
function Top-Windows($file, $label) {
  $A=[System.Windows.Automation.AutomationElement]; $T=[System.Windows.Automation.TreeScope]; $tc=[System.Windows.Automation.Condition]::TrueCondition
  Add-Content -Path $file -Value "### トップレベル窓 $label" -Encoding utf8
  foreach($x in $A::RootElement.FindAll($T::Children,$tc)){ Add-Content -Path $file -Value ("  " + [MW]::Describe($x)) -Encoding utf8 }
}


function Start-Awase($d, $cls) {
  $w = "$d\awase"; New-Item -ItemType Directory -Force -Path $w | Out-Null
  Copy-Item "$Dist\awase.exe" $w -Force
  "[general]`n" | Set-Content -Path "$w\config.toml" -Encoding utf8
  # 窓の IMM の能力を先に学習済みにする(CI の初回プローブが遅く誤学習されるのを防ぐ。観測の無い窓の近似では unavailable)
  "[imm_capability.`"indicator_scenario.exe`"]`nScenWnd = `"$cls`"`nEDIT = `"$cls`"`n" | Set-Content -Path "$w\cache.toml" -Encoding utf8
  $env:RUST_LOG = 'debug'; $env:AWASE_TEST_INJECTION = '1'
  $p = Start-Process -FilePath "$w\awase.exe" -WorkingDirectory $w -PassThru -WindowStyle Hidden
  Pump 8
  Log "awase 起動 pid=$($p.Id) cls=$cls"
  return $p
}
function Stop-Awase($p, $d) {
  if ($p -and -not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
  Pump 1
  if (Test-Path "$d\awase\awase.log") { Copy-Item "$d\awase\awase.log" "$d\awase.log" -Force }
}
function Open-Menu($d, $label) {
  $ind = Find-Indicator
  if (-not $ind) { Log "$label : インジケーターが見つからない"; return $null }
  $r = $ind.Current.BoundingRectangle; $cx=[int](($r.Left+$r.Right)/2); $cy=[int](($r.Top+$r.Bottom)/2)
  [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point($cx, $cy); Pump 1
  [void](Drain "$d\events.log")
  Log "$label : 右クリック"
  [MW]::mouse_event(0x8,0,0,0,[UIntPtr]::Zero); [MW]::mouse_event(0x10,0,0,0,[UIntPtr]::Zero)
  Pump 2
  return $true
}
function Read-Selected($label) {
  $A=[System.Windows.Automation.AutomationElement]; $T=[System.Windows.Automation.TreeScope]
  $sel = @(); $n = 0
  foreach($m in $A::RootElement.FindAll($T::Descendants,(New-Object System.Windows.Automation.PropertyCondition($A::ControlTypeProperty,[System.Windows.Automation.ControlType]::MenuItem)))){
    $n++
    $o = $null
    if ($m.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$o)) { if ($o.Current.IsSelected) { $sel += $m.Current.AutomationId } }
  }
  Log "$label : MenuItem=$n 選択=$($sel -join ',')"
  return $sel
}
function Click-Item($itemId) {
  $A=[System.Windows.Automation.AutomationElement]; $T=[System.Windows.Automation.TreeScope]
  foreach($m in $A::RootElement.FindAll($T::Descendants,(New-Object System.Windows.Automation.PropertyCondition($A::ControlTypeProperty,[System.Windows.Automation.ControlType]::MenuItem)))){
    if ($m.Current.AutomationId -eq $itemId) {
      $tr = $m.Current.BoundingRectangle
      [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point([int](($tr.Left+$tr.Right)/2), [int](($tr.Top+$tr.Bottom)/2)); Pump 1
      [MW]::mouse_event(0x2,0,0,0,[UIntPtr]::Zero); [MW]::mouse_event(0x4,0,0,0,[UIntPtr]::Zero)
      return $true
    }
  }
  return $false
}
# awase を起動した状態で、メニューを開いて(1)読む (2)項目を選ぶ (3)もう一度開いて、選んだ項目が反映されたかを読む。awase.log を残す。
function Measure-Select($tag, $scenArgs, $itemId, $cls, [bool]$withAwase) {
  $d = "$Out\$tag"; New-Item -ItemType Directory -Force -Path $d | Out-Null
  Log "=== $tag 開始($scenArgs, 選ぶ項目=$itemId, awase=$withAwase)"
  $ap = $null
  if ($withAwase) { $ap = Start-Awase $d $cls }
  $p = Start-Process -FilePath "$Dist\indicator_scenario.exe" -ArgumentList "--scen=cycle $scenArgs --n=1 --hold=60 --log=$d\scen.log" -PassThru -WindowStyle Hidden
  Pump 10
  [void](Drain "$d\events-before.log")
  $t0 = [DateTime]::UtcNow
  if (Open-Menu $d '1回目') {
    $before = Read-Selected '選択前'
    Shot "$tag-menu1"
    $ev1 = Drain "$d\events.log"
    Log "1回目の検知: POPUPSTART=$(([regex]::Matches($ev1,'WINEVENT POPUPSTART')).Count) MenuOpened=$(([regex]::Matches($ev1,'UIA-MenuOpened')).Count)"
    $ok = Click-Item $itemId
    Log "項目 $itemId を押した: $ok"
    Pump 3
    $ev2 = Drain "$d\events.log"
    Log "選択後のイベント: Invoked=$(([regex]::Matches($ev2,'UIA-Invoked')).Count) POPUPEND=$(([regex]::Matches($ev2,'WINEVENT POPUPEND')).Count) MENUEND=$(([regex]::Matches($ev2,'WINEVENT MENUEND')).Count) MenuClosed=$(([regex]::Matches($ev2,'UIA-MenuClosed')).Count)"
    Pump 2
    if (Open-Menu $d '2回目') {
      $after = Read-Selected '選択後(再読)'
      Shot "$tag-menu2"
      [void](Drain "$d\events.log")
      [System.Windows.Forms.SendKeys]::SendWait('{ESC}'); Pump 2
    }
  }
  [void](Drain "$d\events.log")
  Log "終了 $tag (開始から $([int]([DateTime]::UtcNow - $t0).TotalSeconds) 秒)"
  Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; Pump 2
  if ($ap) { Stop-Awase $ap $d }
}

function Measure-State($tag, $scenArgs) {
  $d = "$Out\$tag"; New-Item -ItemType Directory -Force -Path $d | Out-Null
  Log "=== $tag 開始($scenArgs)"
  $p = Start-Process -FilePath "$Dist\indicator_scenario.exe" -ArgumentList "--scen=cycle $scenArgs --n=1 --hold=40 --log=$d\scen.log" -PassThru -WindowStyle Hidden
  Pump 10
  [void](Drain "$d\events-before.log")
  Top-Windows "$d\windows.txt" 'クリック前'
  $ind = Find-Indicator
  if (-not $ind) { Log 'インジケーターが見つからない'; Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; return }
  $r = $ind.Current.BoundingRectangle; $cx=[int](($r.Left+$r.Right)/2); $cy=[int](($r.Top+$r.Bottom)/2)
  Log "インジケーター rect=$([int]$r.Left),$([int]$r.Top),$([int]$r.Right),$([int]$r.Bottom) name=$($ind.Current.Name -replace '\s+',' ')"
  [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point($cx, $cy); Pump 1
  [void](Drain "$d\events-before.log")
  Log "右クリック"
  $t0 = [DateTime]::UtcNow
  [MW]::mouse_event(0x8,0,0,0,[UIntPtr]::Zero); [MW]::mouse_event(0x10,0,0,0,[UIntPtr]::Zero)
  Pump 3
  Shot "$tag-menu"
  $ev = Drain "$d\events.log"
  Log "検知イベント: WinEvent MENUPOPUPSTART=$(([regex]::Matches($ev,'WINEVENT POPUPSTART')).Count) MENUSTART=$(([regex]::Matches($ev,'WINEVENT MENUSTART')).Count) UIA-MenuOpened=$(([regex]::Matches($ev,'UIA-MenuOpened')).Count) UIA-WindowOpened=$(([regex]::Matches($ev,'UIA-WindowOpened')).Count)"
  Top-Windows "$d\windows.txt" 'クリック後'
  # フォーカスされた要素と、その親(メニューが出ていればメニュー項目/メニューが取れる)
  try {
    $f = [System.Windows.Automation.AutomationElement]::FocusedElement
    Add-Content -Path "$d\menu.txt" -Value "### FocusedElement : $([MW]::Describe($f))" -Encoding utf8
    $w = [System.Windows.Automation.TreeWalker]::ControlViewWalker; $q = $f
    for($i=0; $i -lt 6 -and $q; $i++){ $q = $w.GetParent($q); if($q){ Add-Content -Path "$d\menu.txt" -Value "  parent$i : $([MW]::Describe($q))" -Encoding utf8 } }
  } catch { Add-Content -Path "$d\menu.txt" -Value "FocusedElement 取得失敗: $($_.Exception.Message)" -Encoding utf8 }
  # クリック後に出た窓(WinEvent の SHOW/CREATE で見えた hwnd)の UIA 部分木
  $hw = @(); foreach($l in ($ev -split "`n")){ if($l -match 'WINEVENT (SHOW|CREATE|POPUPSTART) hwnd=0x([0-9A-F]+) class=(.*?) title=.* visible=True'){ $hw += $Matches[2] } }
  foreach($h in ($hw | Select-Object -Unique)) {
    try { $el = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr][Convert]::ToInt64($h,16)); Dump-Subtree $el "$d\menu.txt" "hwnd=0x$h" } catch { Add-Content -Path "$d\menu.txt" -Value "hwnd=0x$h 取得失敗" -Encoding utf8 }
  }
  # デスクトップ直下から、メニューらしいもの(ControlType=Menu / MenuItem)を一括で探す
  $A=[System.Windows.Automation.AutomationElement]; $T=[System.Windows.Automation.TreeScope]
  foreach($ct in @([System.Windows.Automation.ControlType]::Menu,[System.Windows.Automation.ControlType]::MenuItem)) {
    $cnt=0; foreach($m in $A::RootElement.FindAll($T::Descendants,(New-Object System.Windows.Automation.PropertyCondition($A::ControlTypeProperty,$ct)))){ $cnt++; Add-Content -Path "$d\menuitems.txt" -Value ([MW]::Describe($m)) -Encoding utf8 }
    Log "ControlType=$($ct.ProgrammaticName) の要素 $cnt 件"
  }
  [System.Windows.Forms.SendKeys]::SendWait('{ESC}'); Pump 2
  [void](Drain "$d\events.log")
  Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; Pump 2
}

Log "ime=$Ime"
[MW]::Start()
if ($Ime -eq 'msime') {
  # 通常の窓(IMM が読める。cache は works)
  Measure-Select 'msime-normal-noawase'   '--msime --steps=1:19'  'Item 103' 'works'       $false
  Measure-Select 'msime-normal-awase'     '--msime --steps=1:19'  'Item 103' 'works'       $true
  # 観測の無い窓の近似(TSF text store の窓 assoc。cache は unavailable)
  Measure-Select 'msime-tsf-noawase'      '--msime --win=tsf --imm=assoc --keys=F2' 'Item 103' 'unavailable' $false
  Measure-Select 'msime-tsf-awase'        '--msime --win=tsf --imm=assoc --keys=F2' 'Item 103' 'unavailable' $true
} else {
  Measure-Select 'gji-normal-noawase'     '--keys=16' 'Item 10' 'works'       $false
  Measure-Select 'gji-normal-awase'       '--keys=16' 'Item 10' 'works'       $true
  Measure-Select 'gji-tsf-noawase'        '--win=tsf --imm=assoc --keys=16' 'Item 10' 'unavailable' $false
  Measure-Select 'gji-tsf-awase'          '--win=tsf --imm=assoc --keys=16' 'Item 10' 'unavailable' $true
}
Log '完了'
