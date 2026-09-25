param(
  [string]$Config = 'A',
  [string]$App = 'edit',
  [int]$Runs = 20,
  [string]$AwaseLog = 'awase.log',
  [string]$OutCsv = 'ab1-result.csv'
)
# ADR-191 P1 / タスク09 A/B-1: フォーカス変更時の強制OFF(ime_refresh.rs focus_change_enforce_off)の撤去前後比較。
# 窓2のIMEをONにした状態で、awase(belief=OFF)が動くまま窓1→窓2へフォーカスを移し、+100/+400/+1500msの窓2のIME開閉を読む。
# 物理キーは使わない。Engine OFF は作れないので belief=OFF(窓1のIMEをOFFにして待つ)で代用する(強制OFFの条件は belief のみ)。
Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading;
public class Ab1 {
  [StructLayout(LayoutKind.Sequential)] public struct MSG { public IntPtr hwnd; public uint message; public UIntPtr wParam; public IntPtr lParam; public uint time; public int px, py; }
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr CreateWindowEx(int ex, string cls, string name, int style, int x, int y, int w, int h, IntPtr parent, IntPtr menu, IntPtr inst, IntPtr p);
  [DllImport("user32.dll")] static extern int GetMessage(out MSG m, IntPtr h, uint a, uint b);
  [DllImport("user32.dll")] static extern bool TranslateMessage(ref MSG m);
  [DllImport("user32.dll")] static extern IntPtr DispatchMessage(ref MSG m);
  [DllImport("user32.dll")] static extern IntPtr SendMessage(IntPtr h, int m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] static extern bool BringWindowToTop(IntPtr h);
  [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern bool AttachThreadInput(uint a, uint b, bool attach);
  [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
  [DllImport("imm32.dll")] static extern IntPtr ImmGetDefaultIMEWnd(IntPtr h);

  public static IntPtr Make(int x) {
    IntPtr result = IntPtr.Zero;
    var ready = new ManualResetEvent(false);
    var t = new Thread(() => {
      result = CreateWindowEx(0, "EDIT", "ab1", 0x00CF0000 | 0x10000000 | 0x4, x, 100, 400, 300, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
      ready.Set();
      MSG m;
      while (GetMessage(out m, IntPtr.Zero, 0, 0) > 0) { TranslateMessage(ref m); DispatchMessage(ref m); }
    });
    t.IsBackground = true; t.Start(); ready.WaitOne();
    return result;
  }
  public static bool GetOpen(IntPtr h) {
    IntPtr ime = ImmGetDefaultIMEWnd(h);
    return ime != IntPtr.Zero && SendMessage(ime, 0x283, (IntPtr)5, IntPtr.Zero) != IntPtr.Zero;
  }
  public static void SetOpen(IntPtr h, bool open) {
    IntPtr ime = ImmGetDefaultIMEWnd(h);
    if (ime != IntPtr.Zero) SendMessage(ime, 0x283, (IntPtr)6, open ? (IntPtr)1 : IntPtr.Zero);
  }
  public static bool Front(IntPtr h) {
    for (int i = 0; i < 10; i++) {
      IntPtr fg = GetForegroundWindow(); uint pid; uint ft = fg == IntPtr.Zero ? 0 : GetWindowThreadProcessId(fg, out pid);
      uint me = GetCurrentThreadId(); bool att = ft != 0 && ft != me && AttachThreadInput(me, ft, true);
      BringWindowToTop(h); SetForegroundWindow(h);
      if (att) AttachThreadInput(me, ft, false);
      if (GetForegroundWindow() == h) return true;
      Thread.Sleep(150);
    }
    return false;
  }
  // 窓を前面化した瞬間を0として、+100/+400/+1500msに窓のIME開閉を読む。戻り値は [前面化成否, +100, +400, +1500] (1/0)。
  public static int[] FrontAndSample(IntPtr h) {
    var sw = Stopwatch.StartNew();
    bool ok = Front(h);
    sw.Restart();
    int[] due = new int[] { 100, 400, 1500 };
    int[] r = new int[4]; r[0] = ok ? 1 : 0;
    for (int i = 0; i < 3; i++) {
      while (sw.ElapsedMilliseconds < due[i]) Thread.Sleep(5);
      r[i + 1] = GetOpen(h) ? 1 : 0;
    }
    return r;
  }
}
'@

function Read-LogLines {
  if (-not (Test-Path $AwaseLog)) { return @() }
  $fs = [IO.File]::Open($AwaseLog, 'Open', 'Read', 'ReadWrite')
  $sr = New-Object IO.StreamReader($fs, [Text.Encoding]::UTF8)
  $all = $sr.ReadToEnd(); $sr.Close(); $fs.Close()
  return @($all -split "`r?`n")
}


function Open-App {
  param([int]$x)
  if ($App -eq 'edit') { return [Ab1]::Make($x) }
  $p = Start-Process -FilePath notepad.exe -PassThru
  for ($k = 0; $k -lt 50; $k++) { Start-Sleep -Milliseconds 200; $p.Refresh(); if ($p.MainWindowHandle -ne [IntPtr]::Zero) { break } }
  return $p.MainWindowHandle
}
$w1 = Open-App 50
$w2 = Open-App 500
"app=$App windows: w1=$w1 w2=$w2"
if ($w1 -eq [IntPtr]::Zero -or $w2 -eq [IntPtr]::Zero -or $w1 -eq $w2) { "SUMMARY config=$Config app=$App valid=0/$Runs (窓を2つ作れなかった)"; exit 0 }
Start-Sleep -Seconds 2
# 試行ごとに (窓1のIME状態, 窓2へ移る前の待ち ms) を変えて強制OFFの発火条件を探る
$combos = @(@($false, 800), @($false, 2500), @($false, 6000), @($true, 2500))
$rows = @()
$fullLog = @()
for ($i = 1; $i -le $Runs; $i++) {
  $c = $combos[($i - 1) % $combos.Count]
  $w1on = [bool]$c[0]; $idle = [int]$c[1]
  [void][Ab1]::Front($w1)
  [Ab1]::SetOpen($w1, $w1on)
  [Ab1]::SetOpen($w2, $true)
  Start-Sleep -Milliseconds $idle
  $before = (Read-LogLines).Count
  $s = [Ab1]::FrontAndSample($w2)
  Start-Sleep -Milliseconds 300
  $new = @((Read-LogLines) | Select-Object -Skip $before)
  $fired = @($new | Where-Object { $_ -match 'focus_change_enforce_off|FocusChange: set_ime_open\(false\)' })
  $sent = ($new | Where-Object { $_ -match 'sent=(true|false)' } | ForEach-Object { if ($_ -match 'sent=(true|false)') { $Matches[1] } }) -join '/'
  $wshadow = @($new | Where-Object { $_ -match 'warrant-shadow' }).Count
  $drift = @($new | Where-Object { $_ -match 'Blacklist drift' }).Count
  $row = [pscustomobject]@{
    config = $Config; app = $App; trial = $i; w1_on = [int]$w1on; idle_ms = $idle
    front_ok = $s[0]; open_100ms = $s[1]; open_400ms = $s[2]; open_1500ms = $s[3]
    fired = [int]($fired.Count -gt 0); sent = $sent; warrant_shadow = $wshadow; drift = $drift; new_log_lines = $new.Count
  }
  $row | Format-Table -HideTableHeaders | Out-String -Width 250 | Write-Host
  $rows += $row
  $fullLog += "--- trial $i w1_on=$w1on idle=$idle fired=$($fired.Count)"
  $fullLog += ($new | Where-Object { $_ -match 'FocusChange|focus|enforce|warrant|drift|belief|applied' } | Select-Object -First 25)
}
$rows | Export-Csv -NoTypeInformation -Path $OutCsv -Encoding utf8
$fullLog | Set-Content ($OutCsv -replace '\.csv$', '.trials.log') -Encoding utf8
"=== CSV ($Config/$App) ==="
Get-Content $OutCsv
$valid = @($rows | Where-Object { $_.front_ok -eq 1 })
$fired = @($valid | Where-Object { $_.fired -eq 1 })
function Cnt($set, $col) { @($set | Where-Object { $_.$col -eq 1 }).Count }
"SUMMARY config=$Config app=$App valid=$($valid.Count)/$Runs fired=$($fired.Count)"
"  all:   open@100=$(Cnt $valid 'open_100ms') open@400=$(Cnt $valid 'open_400ms') open@1500=$(Cnt $valid 'open_1500ms')"
"  fired: open@100=$(Cnt $fired 'open_100ms') open@400=$(Cnt $fired 'open_400ms') open@1500=$(Cnt $fired 'open_1500ms')"
foreach ($g in ($valid | Group-Object w1_on, idle_ms)) { "  cond[$($g.Name)] n=$($g.Count) fired=$(Cnt $g.Group 'fired')" }
