param(
  [string]$Config = 'A',
  [int]$Runs = 8,
  [string]$AwaseLog = 'awase.log',
  [string]$OutCsv = 'ab1-result.csv',
  [string]$Serve = ''
)
# ADR-191 P1 / タスク09 A/B-1: フォーカス変更時の強制OFF(ime_refresh.rs focus_change_enforce_off)の撤去前後比較。
# 窓2のIMEをONにした状態で、awase(belief=OFF)が動くまま窓1→窓2へフォーカスを移し、+100/+400/+1500msの窓2のIME開閉を読む。
# アプローチ1: 窓1前面で VK_IME_OFF(0x1A) をテスト注入(物理キー扱い)して belief=OFF(明示意図)を決定的に作る。
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
  [StructLayout(LayoutKind.Explicit, Size=40)] public struct INPUT {
    [FieldOffset(0)] public uint type; [FieldOffset(8)] public ushort wVk; [FieldOffset(10)] public ushort wScan;
    [FieldOffset(12)] public uint dwFlags; [FieldOffset(16)] public uint time; [FieldOffset(24)] public IntPtr dwExtraInfo;
  }
  [DllImport("user32.dll", SetLastError=true)] static extern uint SendInput(uint n, INPUT[] i, int size);
  // awase の AWASE_TEST_INJECTION=1 は dwExtraInfo=TEST_INJECTION_MARKER(0x5350494B)付きの注入を物理キーとして扱う(hook.rs)。
  public static uint PressKey(ushort vk) {
    INPUT[] a = new INPUT[2];
    a[0].type = 1; a[0].wVk = vk; a[0].dwFlags = 0; a[0].dwExtraInfo = (IntPtr)0x5350494B;
    a[1].type = 1; a[1].wVk = vk; a[1].dwFlags = 2; a[1].dwExtraInfo = (IntPtr)0x5350494B;
    return SendInput(2, a, 40);
  }

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

# ime_refresh の FocusChanged は「プロセス変更時」だけ発火するので、窓2は別プロセス(別pwsh)に置く。
if ($Serve) {
  $h = [Ab1]::Make(500)
  Set-Content -Path $Serve -Value ([int64]$h) -Encoding ascii
  Start-Sleep -Seconds 600
  exit
}
$w1 = [Ab1]::Make(50)
$hf = Join-Path $env:TEMP 'ab1-w2.txt'
Remove-Item $hf -ErrorAction SilentlyContinue
$srv = Start-Process pwsh -ArgumentList '-NoProfile','-File',$PSCommandPath,'-Serve',$hf -PassThru
for ($k = 0; $k -lt 60 -and -not (Test-Path $hf); $k++) { Start-Sleep -Milliseconds 500 }
$w2 = [IntPtr][int64](Get-Content $hf)
"windows: w1=$w1 w2=$w2"
Start-Sleep -Seconds 2
$rows = @()
for ($i = 1; $i -le $Runs; $i++) {
  [void][Ab1]::Front($w1)
  $lb = (Read-LogLines).Count
  $sent_keys = [Ab1]::PressKey(0x1A)
  Start-Sleep -Milliseconds 1500
  $pre = @((Read-LogLines) | Select-Object -Skip $lb)
  $premise = @($pre | Where-Object { $_ -match 'ime-key|IME mode key|VK_IME_OFF|0x1a|0x1A|explicit|desired_open|ime-diag' })
  [Ab1]::SetOpen($w2, $true)
  Start-Sleep -Milliseconds 500
  $before = (Read-LogLines).Count
  $s = [Ab1]::FrontAndSample($w2)
  Start-Sleep -Milliseconds 300
  $new = @((Read-LogLines) | Select-Object -Skip $before)
  $hits = @($new | Where-Object { $_ -match 'focus_change_enforce_off|FocusChange: set_ime_open|warrant-shadow' })
  $sent = ($hits | Where-Object { $_ -match 'sent=(true|false)' } | ForEach-Object { if ($_ -match 'sent=(true|false)') { $Matches[1] } }) -join '/'
  $row = [pscustomobject]@{
    config = $Config; trial = $i; front_ok = $s[0]; open_100ms = $s[1]; open_400ms = $s[2]; open_1500ms = $s[3]
    key_sent = $sent_keys; premise_lines = $premise.Count; enforce_off_lines = $hits.Count; sent = $sent
  }
  $row | Format-Table -HideTableHeaders | Out-String -Width 200 | Write-Host
  $rows += $row
  $premise | Select-Object -First 4 | ForEach-Object { "  premise: $_" }
  $new | Where-Object { $_ -match 'ime-diag. label=focus_changed' } | Select-Object -First 1 | ForEach-Object { "  focus_diag: $_" }
  if ($hits.Count -gt 0) { $hits | Select-Object -First 3 | ForEach-Object { "  log: $_" } }
}
$rows | Export-Csv -NoTypeInformation -Path $OutCsv -Encoding utf8
"=== CSV ($Config) ==="
Get-Content $OutCsv
$valid = @($rows | Where-Object { $_.front_ok -eq 1 })
$stay = @($valid | Where-Object { $_.open_1500ms -eq 1 })
"SUMMARY config=$Config valid=$($valid.Count)/$Runs open_at_100ms=$(@($valid | Where-Object { $_.open_100ms -eq 1 }).Count) open_at_400ms=$(@($valid | Where-Object { $_.open_400ms -eq 1 }).Count) open_at_1500ms=$($stay.Count)"

Stop-Process -Id $srv.Id -Force -ErrorAction SilentlyContinue
