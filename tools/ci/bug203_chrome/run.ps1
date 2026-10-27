# BUG-203 の実アプリ確認: Chrome(IMM が読めない窓)+GJI で、入力インジケーターのメニューから Direct Input(Item 10)を選ぶと、
# 直後に GJI の I/O 推論(gji-poll → effective-open-flip → true)が出るかを測る。測定専用。
param([string]$Dist = 'dist', [string]$Out = 'out', [string]$Rounds = '3')
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
    Q.Enqueue(Ts() + " WINEVENT " + name + " hwnd=0x" + hwnd.ToString("X") + " class=" + c + " title=" + t + " visible=" + (hwnd != IntPtr.Zero && IsWindowVisible(hwnd)) + " idObj=" + idObj + " idChild=" + idChild + " thread=" + thread);
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

Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices;
public static class KB { [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra); }
"@
function Tap($vk) { [KB]::keybd_event([byte]$vk,0,0,[UIntPtr]::Zero); Start-Sleep -Milliseconds 40; [KB]::keybd_event([byte]$vk,0,2,[UIntPtr]::Zero) }

function Start-AwaseChrome($d) {
  $w = "$d\awase"; New-Item -ItemType Directory -Force -Path $w | Out-Null
  Copy-Item "$Dist\awase.exe" $w -Force
  "[general]`n" | Set-Content -Path "$w\config.toml" -Encoding utf8
  $env:RUST_LOG = 'debug'; $env:AWASE_TEST_INJECTION = '1'
  $p = Start-Process -FilePath "$w\awase.exe" -WorkingDirectory $w -PassThru -WindowStyle Hidden
  Pump 8; Log "awase 起動 pid=$($p.Id)"; return $p
}
function Find-Chrome { foreach($c in @('C:\Program Files\Google\Chrome\Application\chrome.exe','C:\Program Files (x86)\Google\Chrome\Application\chrome.exe')) { if (Test-Path $c) { return $c } }; return $null }

$chrome = Find-Chrome
Log "chrome=$chrome"
[MW]::Start()
for ($i = 1; $i -le [int]$Rounds; $i++) {
  $tag = "chrome-r$i"; $d = "$Out\$tag"; New-Item -ItemType Directory -Force -Path $d | Out-Null
  Log "=== $tag 開始"
  $ap = Start-AwaseChrome $d
  $ud = "$d\profile"
  $html = 'data:text/html,<title>bug203</title><input id=i autofocus style="width:600px;height:80px;font-size:40px">'
  $cp = Start-Process -FilePath $chrome -ArgumentList "--user-data-dir=$ud --no-first-run --no-default-browser-check --window-position=100,100 --window-size=900,500 `"$html`"" -PassThru
  Pump 8
  # 入力欄にフォーカス(クリック)
  [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point(400, 250); Pump 1
  [MW]::mouse_event(0x2,0,0,0,[UIntPtr]::Zero); [MW]::mouse_event(0x4,0,0,0,[UIntPtr]::Zero); Pump 2
  # IME ON(VK 0x16)→ 開いている状態にしておく
  Tap 0x16; Pump 3
  Log "IME ON キーを送った"
  Shot "$tag-0-chrome"
  [void](Drain "$d\events-before.log")
  if (Open-Menu $d '1回目') {
    $before = Read-Selected '選択前'
    $t0 = [DateTime]::UtcNow.ToString('HH:mm:ss.fffZ')
    $ok = Click-Item 'Item 10'
    Log "Direct Input(Item 10)を押した: $ok at $t0"
    Pump 4
    [void](Drain "$d\events.log")
    if (Open-Menu $d '2回目') { $after = Read-Selected '選択後(再読)'; Shot "$tag-menu2"; [System.Windows.Forms.SendKeys]::SendWait('{ESC}'); Pump 2 }
  }
  # さらに 3 秒待って、定期観測の flip が出る時間を取る
  Pump 3
  Stop-Process -Id $cp.Id -Force -ErrorAction SilentlyContinue
  Get-Process chrome -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  Pump 2
  Stop-Awase $ap $d
  Log "終了 $tag"
}
Log '完了'
