# ADR-261 D0-4: タスクバーの状態・MS-IME ツールバーで、入力インジケーターの字形の取り込みがどう変わるかを windows-latest で測る。
# 使い方: run.ps1 -Cond shown|autohide|top|left|right|toolbar -Dist dist -Out out
param([Parameter(Mandatory)][string]$Cond, [string]$Dist = 'dist', [string]$Out = 'out')
$ErrorActionPreference = 'Continue'
$Dist = (Resolve-Path $Dist).Path
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$Out = (Resolve-Path $Out).Path
Add-Type -AssemblyName System.Windows.Forms, System.Drawing, UIAutomationClient, UIAutomationTypes
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices;
public static class Tb {
  [StructLayout(LayoutKind.Sequential)] public struct APPBARDATA { public uint cbSize; public IntPtr hWnd; public uint uCallbackMessage; public uint uEdge; public int l,t,r,b; public IntPtr lParam; }
  [DllImport("shell32.dll")] static extern UIntPtr SHAppBarMessage(uint msg, ref APPBARDATA d);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern IntPtr FindWindow(string c, string t);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int l,t,r,b; }
  public static uint GetState() { APPBARDATA d = new APPBARDATA(); d.cbSize=(uint)Marshal.SizeOf(typeof(APPBARDATA)); return (uint)SHAppBarMessage(4, ref d); }
  public static void SetState(uint s) { APPBARDATA d = new APPBARDATA(); d.cbSize=(uint)Marshal.SizeOf(typeof(APPBARDATA)); d.lParam=(IntPtr)s; SHAppBarMessage(0xA, ref d); }
  public static string TrayRect() { RECT r; IntPtr h=FindWindow("Shell_TrayWnd",null); if(h==IntPtr.Zero) return "no-tray"; GetWindowRect(h,out r); return r.l+","+r.t+","+r.r+","+r.b; }
}
"@
function Log($m) { $l = "[$([DateTime]::UtcNow.ToString('HH:mm:ss.fffZ'))] $m"; Write-Host $l; Add-Content -Path "$Out\run.log" -Value $l -Encoding utf8 }
function Shot($name) {
  try {
    $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
    $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
    $bmp.Save("$Out\$name.png", [System.Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $bmp.Dispose()
  } catch { Log "shot $name 失敗: $($_.Exception.Message)" }
}
function Phase($name, $n) {
  $d = "$Out\$name"; New-Item -ItemType Directory -Force -Path $d | Out-Null
  Log "phase $name 開始 tray=$([Tb]::TrayRect()) appbar_state=$([Tb]::GetState())"
  $p = Start-Process -FilePath "$Dist\ime_indicator_probe.exe" -ArgumentList "--log=$d\probe.log","--duration=$(14 + 9 * $n + 8)","--interval=50","--raw=$d" -PassThru -WindowStyle Hidden
  Start-Sleep 2
  & "$Dist\indicator_scenario.exe" --scen=cycle --msime --n=$n "--steps=1:19,1:1b,1:13,1:18,1:10,0:10" "--log=$d\scen.log" | Out-Null
  Start-Sleep 2
  if (-not $p.HasExited) { $p.WaitForExit(40000) | Out-Null }
  Shot "$name-after"
  Log "phase $name 終了 pgm=$((Get-ChildItem $d -Filter *.pgm -ErrorAction SilentlyContinue).Count)"
}
function Move-Mouse($x, $y) { [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point($x, $y) }
function Dump-Tray($tag) {
  # トレイ内の入力インジケーターと、トップレベルの IME 関連の窓を一覧にする(ツールバー・メニューの確認用)
  $A=[System.Windows.Automation.AutomationElement]; $T=[System.Windows.Automation.TreeScope]; $tc=[System.Windows.Automation.Condition]::TrueCondition
  $lines=@()
  foreach($x in $A::RootElement.FindAll($T::Children,$tc)){
    $c=$x.Current; $r=$c.BoundingRectangle
    if($c.ClassName -match 'IME|Tool|Lang|CiceroUI|Input|Menu|Popup|TrayWnd'){ $lines += "TOP class=$($c.ClassName) name=$($c.Name) rect=$([int]$r.Left),$([int]$r.Top),$([int]$r.Right),$([int]$r.Bottom) offscreen=$($c.IsOffscreen)" }
  }
  $lines | Out-File -FilePath "$Out\dump-$tag.txt" -Encoding utf8
}

Log "cond=$Cond tray=$([Tb]::TrayRect()) appbar_state=$([Tb]::GetState()) screen=$([System.Windows.Forms.SystemInformation]::VirtualScreen)"
# MS-IME を有効にし、インジケーターが MS-IME になるまで待つ(窓を 1 つ作って戻るだけ)
& "$Dist\indicator_scenario.exe" --scen=cycle --msime --n=0 "--log=$Out\init.log" | Out-Null
Start-Sleep 3
Shot 'initial'
Dump-Tray 'initial'
$screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds

switch ($Cond) {
  'shown' { Phase 'shown' 2 }
  'autohide' {
    $orig = [Tb]::GetState()
    [Tb]::SetState(($orig -bor 1)); Start-Sleep 5
    Log "自動非表示を設定 appbar_state=$([Tb]::GetState()) tray=$([Tb]::TrayRect())"
    Move-Mouse ($screen.Width / 2) ($screen.Height / 3); Start-Sleep 3
    Shot 'autohide-hidden'
    Phase 'hidden' 2
    # マウスを下端に寄せてタスクバーを出す
    Move-Mouse ($screen.Width / 2) ($screen.Height - 1); Start-Sleep 3
    Shot 'autohide-hover'
    Phase 'hover' 2
    Move-Mouse ($screen.Width / 2) ($screen.Height / 3); Start-Sleep 4
    Phase 'afterhover' 1
    [Tb]::SetState(($orig -band (-bnot 1))); Start-Sleep 3
    Log "自動非表示を解除 appbar_state=$([Tb]::GetState())"
  }
  { $_ -in 'top','left','right' } {
    $edge = @{ left = 0; top = 1; right = 2 }[$Cond]
    $key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StuckRects3'
    $v = (Get-ItemProperty -Path $key -Name Settings -ErrorAction SilentlyContinue).Settings
    if (-not $v) { Log "StuckRects3 が無い(この OS では位置を変えられない)"; break }
    $orig = $v.Clone(); Log "StuckRects3 byte12(現在の辺)=$($v[12]) 長さ=$($v.Length)"
    $v[12] = $edge
    Set-ItemProperty -Path $key -Name Settings -Value $v
    Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue
    Start-Sleep 8
    if (-not (Get-Process explorer -ErrorAction SilentlyContinue)) { Start-Process explorer.exe; Start-Sleep 8 }
    Log "位置を $Cond に設定 tray=$([Tb]::TrayRect())"
    Shot "edge-$Cond"
    Phase "edge-$Cond" 2
    Set-ItemProperty -Path $key -Name Settings -Value $orig
    Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue; Start-Sleep 6
    if (-not (Get-Process explorer -ErrorAction SilentlyContinue)) { Start-Process explorer.exe; Start-Sleep 6 }
  }
  'toolbar' {
    # トレイの入力インジケーターを右クリックし、メニュー項目(IME ツールバー)を探して押す。メニュー項目の一覧は診断として残す。
    $A=[System.Windows.Automation.AutomationElement]; $T=[System.Windows.Automation.TreeScope]; $tc=[System.Windows.Automation.Condition]::TrueCondition
    $tray = $null
    foreach($x in $A::RootElement.FindAll($T::Children,$tc)){ if($x.Current.ClassName -eq 'Shell_TrayWnd'){ $tray=$x } }
    $ind = $null
    if ($tray) { foreach($e in $tray.FindAll($T::Descendants,$tc)){ if($e.Current.Name -match '^(Tray Input Indicator|トレイ入力インジケーター)'){ $ind=$e; break } } }
    if (-not $ind) { Log 'インジケーターが見つからない'; Phase 'toolbar-noindicator' 1; break }
    $r = $ind.Current.BoundingRectangle; $cx=[int](($r.Left+$r.Right)/2); $cy=[int](($r.Top+$r.Bottom)/2)
    Move-Mouse $cx $cy; Start-Sleep 1
    [Tb]::mouse_event(0x8,0,0,0,[UIntPtr]::Zero); [Tb]::mouse_event(0x10,0,0,0,[UIntPtr]::Zero)   # 右ボタン
    Start-Sleep 2; Shot 'toolbar-menu'
    $items=@()
    foreach($m in $A::RootElement.FindAll($T::Descendants,(New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty,[System.Windows.Automation.ControlType]::MenuItem)))){
      $items += "$($m.Current.Name) | class=$($m.Current.ClassName) | rect=$([int]$m.Current.BoundingRectangle.Left),$([int]$m.Current.BoundingRectangle.Top)"
    }
    $items | Out-File -FilePath "$Out\menu-items.txt" -Encoding utf8
    Log "メニュー項目 $($items.Count) 件"
    $target = $null
    foreach($m in $A::RootElement.FindAll($T::Descendants,(New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty,[System.Windows.Automation.ControlType]::MenuItem)))){
      if($m.Current.Name -match 'toolbar|ツールバー'){ $target=$m; break }
    }
    if ($target) {
      $tr=$target.Current.BoundingRectangle; Move-Mouse ([int](($tr.Left+$tr.Right)/2)) ([int](($tr.Top+$tr.Bottom)/2)); Start-Sleep 1
      [Tb]::mouse_event(0x2,0,0,0,[UIntPtr]::Zero); [Tb]::mouse_event(0x4,0,0,0,[UIntPtr]::Zero)
      Log "ツールバーの項目を押した: $($target.Current.Name)"
    } else { Log 'ツールバーの項目が見つからない(menu-items.txt を参照)'; [System.Windows.Forms.SendKeys]::SendWait('{ESC}') }
    Start-Sleep 3; Shot 'toolbar-shown'; Dump-Tray 'toolbar'
    Move-Mouse ($screen.Width / 2) ($screen.Height / 3); Start-Sleep 2
    Phase 'toolbar' 2
  }
  default { Log "未知の条件 $Cond" }
}
Log "完了 tray=$([Tb]::TrayRect())"
