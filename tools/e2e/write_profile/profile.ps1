param(
  [Parameter(Mandatory)][ValidateSet('edge','notepad','wt','zoom')][string]$Mode,
  [int[]]$Lens = @(0,12,30),
  [int]$Trials = 3
)
$ErrorActionPreference = 'Stop'
Add-Type @"
using System; using System.Text; using System.Runtime.InteropServices;
public class WP {
  [StructLayout(LayoutKind.Sequential)] public struct IOC { public ulong ReadOps, WriteOps, OtherOps, ReadBytes, WriteBytes, OtherBytes; }
  [DllImport("kernel32.dll")] public static extern IntPtr OpenProcess(uint a, bool i, uint pid);
  [DllImport("kernel32.dll")] public static extern bool GetProcessIoCounters(IntPtr h, out IOC c);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte sc, uint fl, UIntPtr ex);
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc p, IntPtr l);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  public static string Cls(IntPtr h){ var s=new StringBuilder(256); GetClassName(h,s,256); return s.ToString(); }
  public static string Title(IntPtr h){ var s=new StringBuilder(256); GetWindowText(h,s,256); return s.ToString(); }
  public static IntPtr FindCls(string c){ IntPtr r=IntPtr.Zero; EnumWindows((h,l)=>{ if(IsWindowVisible(h) && Cls(h)==c){ r=h; return false;} return true;}, IntPtr.Zero); return r; }
  public static IntPtr FindTitle(string t){ IntPtr r=IntPtr.Zero; EnumWindows((h,l)=>{ if(IsWindowVisible(h) && Title(h).Contains(t)){ r=h; return false;} return true;}, IntPtr.Zero); return r; }
  public static ulong WB(IntPtr h){ IOC c; GetProcessIoCounters(h, out c); return c.WriteBytes; }
  public static ulong WO(IntPtr h){ IOC c; GetProcessIoCounters(h, out c); return c.WriteOps; }
}
"@
$mode = $Mode
$lens = $Lens
$trials = $Trials
function Key($vk,$sc){ [WP]::keybd_event([byte]$vk,[byte]$sc,0,[UIntPtr]::Zero); Start-Sleep -Milliseconds 25; [WP]::keybd_event([byte]$vk,[byte]$sc,2,[UIntPtr]::Zero) }
function AltTap { [WP]::keybd_event(0x12,0,0,[UIntPtr]::Zero); [WP]::keybd_event(0x12,0,2,[UIntPtr]::Zero) }
$script:hs = @()
function Refresh-Handles {
  $ps = @(Get-Process | Where-Object { $_.ProcessName -like 'GoogleIMEJaConverter*' -or $_.ProcessName -like 'GoogleJapaneseInputConverter*' })
  $script:hs = @($ps | ForEach-Object { [WP]::OpenProcess(0x1000, $false, [uint32]$_.Id) })
  return $ps
}
$null = Refresh-Handles
function SumB { $t=[uint64]0; foreach ($x in $script:hs) { $t += [WP]::WB($x) }; $t }
function SumO { $t=[uint64]0; foreach ($x in $script:hs) { $t += [WP]::WO($x) }; $t }
# ---- 窓を開く/前面に ----
$target = [IntPtr]::Zero; $closeTitle = ''
switch ($mode) {
  'zoom'    { $target = [WP]::FindCls('ConfMultiTabContentWndClass') }
  'edge'    { Start-Process msedge -ArgumentList '--new-window','--user-data-dir=C:/Users/cuzic/awase-prof-edge','--no-first-run','data:text/html,<title>awase-prof-edge</title><textarea autofocus rows=5 cols=40></textarea>'; Start-Sleep -Seconds 5; $target = [WP]::FindTitle('awase-prof-edge'); $closeTitle='awase-prof-edge' }
  'notepad' { Start-Process notepad; Start-Sleep -Seconds 3; $target = [WP]::FindCls('Notepad'); $closeTitle='Notepad' }
  'wt'      { Start-Process wt.exe -ArgumentList '-w','new','cmd','/k','title awase-prof-wt'; Start-Sleep -Seconds 5; $target = [WP]::FindTitle('awase-prof-wt'); $closeTitle='awase-prof-wt' }
}
if ($target -eq [IntPtr]::Zero) { 'ABORT: window not found'; exit 1 }
AltTap; [WP]::SetForegroundWindow($target) | Out-Null; Start-Sleep -Milliseconds 1500
$fg = [WP]::GetForegroundWindow()
'mode=' + $mode + ' fg_class=' + [WP]::Cls($fg) + ' fg_is_target=' + ($fg -eq $target)
if ($fg -ne $target) { 'ABORT: target not foreground'; exit 1 }
$sw = [System.Diagnostics.Stopwatch]::new()
function Probe($vk,$sc) {
  $b0 = SumB; $o0 = SumO; $first = -1; $b313 = -1; $sw.Restart(); Key $vk $sc
  while ($sw.ElapsedMilliseconds -lt 500) {
    $e = $sw.ElapsedMilliseconds
    $d = (SumB) - $b0
    if ($first -lt 0 -and $d -gt 0) { $first = $e }
    if ($b313 -lt 0 -and $e -ge 313) { $b313 = $d }
    Start-Sleep -Milliseconds 1
  }
  [pscustomobject]@{ first=$first; b313=$b313; b500=((SumB)-$b0); ops=((SumO)-$o0) }
}
# GJI の変換サーバーは最初の入力で起動することがあるので、見つかるまで待つ(最大 20 秒)。
Key 0x16 0x00; Start-Sleep -Milliseconds 700; Key 0xF2 0x70; Start-Sleep -Milliseconds 800
for ($w = 0; $w -lt 20 -and $script:hs.Count -eq 0; $w++) { Key 0x4B 0x25; Start-Sleep -Milliseconds 500; Key 0x08 0x0E; Start-Sleep -Milliseconds 500; $null = Refresh-Handles }
'converters=' + $script:hs.Count
if ($script:hs.Count -eq 0) { 'ABORT: GJI converter not found'; exit 1 }
# ---- IME ON の確認(書き込みが出るか)。出なければ半角/全角で1回だけ切り替えて再確認 ----
Key 0x16 0x00; Start-Sleep -Milliseconds 700; Key 0xF2 0x70; Start-Sleep -Milliseconds 1200
$r = Probe 0x4B 0x25; Start-Sleep -Milliseconds 600; Key 0x08 0x0E; Start-Sleep -Milliseconds 400
if ($r.b500 -eq 0) {
  Key 0x19 0x29; Start-Sleep -Milliseconds 1200
  $r = Probe 0x4B 0x25; Start-Sleep -Milliseconds 600; Key 0x08 0x0E; Start-Sleep -Milliseconds 400
  'ime_on_retry_with_kanji_key bytes500=' + $r.b500
}
if ($r.b500 -eq 0) { 'ABORT: IME does not respond (no GJI writes)'; if ($closeTitle) { AltTap; Key 0x73 0x3E }; exit 1 }
# ---- 本測定 ----
foreach ($len in $lens) {
  foreach ($gap in 3000, 200) {
    for ($t=1; $t -le $trials; $t++) {
      # 入力中の文字を len 個ためる(「あ」。最後にまとめて消す)
      for ($i=0; $i -lt $len; $i++) { Key 0x41 0x1E; Start-Sleep -Milliseconds 40 }
      Start-Sleep -Milliseconds $gap
      if ([WP]::GetForegroundWindow() -ne $target) { 'ABORT: focus left'; break }
      $a = Probe 0x4B 0x25; Start-Sleep -Milliseconds 600
      $b = Probe 0x49 0x17; Start-Sleep -Milliseconds 600
      '{0},len={1},gap={2},k,first={3},b313={4},b500={5},ops={6}' -f $mode,$len,$gap,$a.first,$a.b313,$a.b500,$a.ops
      '{0},len={1},gap={2},i,first={3},b313={4},b500={5},ops={6}' -f $mode,$len,$gap,$b.first,$b.b313,$b.b500,$b.ops
      for ($i=0; $i -lt ($len + 1); $i++) { Key 0x08 0x0E; Start-Sleep -Milliseconds 40 }
      Start-Sleep -Milliseconds 300
    }
  }
}
if ($closeTitle) {
  $fg = [WP]::GetForegroundWindow()
  if ($fg -eq $target) { AltTap; Key 0x73 0x3E; 'closed test window (Alt+F4)' } else { 'WARN: test window not foreground at the end; left open' }
}
'DONE'
