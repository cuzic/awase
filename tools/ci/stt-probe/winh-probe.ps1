# 注入した Win+H で音声入力のパネルが出るかを、対照つきで観測する(判定は付けない)。
# Windows PowerShell 5.1 で動かす(UI Automation の .NET Framework 版を使うため)。
# 手順: 窓の一覧 → 対照(Shift のタップ)→ 窓の一覧+画面 → Win+H → 窓の一覧+画面+UI 要素 → もう一度 Win+H → 窓の一覧。
param([string]$OutDir = "stt-probe-out")
$ErrorActionPreference = "Continue"
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class W {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc p, IntPtr l);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  public class Info { public long Hwnd; public bool Visible; public string Cls; public string Title; public uint Pid; public int W, H; }
  public static List<Info> All() {
    var list = new List<Info>();
    EnumWindows(delegate (IntPtr h, IntPtr l) {
      var t = new StringBuilder(256); var c = new StringBuilder(256);
      GetWindowText(h, t, 256); GetClassName(h, c, 256);
      uint pid; GetWindowThreadProcessId(h, out pid);
      RECT r; GetWindowRect(h, out r);
      var i = new Info(); i.Hwnd = h.ToInt64(); i.Visible = IsWindowVisible(h); i.Cls = c.ToString(); i.Title = t.ToString(); i.Pid = pid; i.W = r.R - r.L; i.H = r.B - r.T;
      list.Add(i); return true; }, IntPtr.Zero);
    return list;
  }
}
public static class Kb {
  [StructLayout(LayoutKind.Sequential)] public struct MOUSEINPUT { public int dx; public int dy; public uint mouseData; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
  [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT { public ushort wVk; public ushort wScan; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
  [StructLayout(LayoutKind.Explicit)] public struct INPUT { [FieldOffset(0)] public uint type; [FieldOffset(8)] public MOUSEINPUT mi; [FieldOffset(8)] public KEYBDINPUT ki; }
  [DllImport("user32.dll", SetLastError=true)] static extern uint SendInput(uint n, INPUT[] p, int cb);
  public static uint Key(ushort vk, bool up) {
    var i = new INPUT[1]; i[0].type = 1; i[0].ki.wVk = vk; i[0].ki.dwFlags = up ? 2u : 0u;
    return SendInput(1, i, Marshal.SizeOf(typeof(INPUT)));
  }
}
'@

$procName = @{}
function ProcName([uint32]$id) {
  if (-not $procName.ContainsKey($id)) {
    $p = Get-Process -Id $id -ErrorAction SilentlyContinue
    $procName[$id] = if ($p) { $p.ProcessName } else { "?" }
  }
  return $procName[$id]
}
function Dump([string]$label) {
  $procName.Clear()
  $rows = [W]::All() | Where-Object { $_.Visible -and ($_.Title -ne "" -or $_.Cls -match "Input|Voice|Speech|Xaml|ApplicationFrame") } |
    ForEach-Object { [pscustomobject]@{ Proc = (ProcName $_.Pid); Class = $_.Cls; Title = $_.Title; Size = "$($_.W)x$($_.H)"; Hwnd = ("0x{0:X}" -f $_.Hwnd) } } |
    Sort-Object Proc, Class
  $text = "### $label`n" + (($rows | Format-Table -AutoSize | Out-String -Width 220))
  $text | Out-File -Append -Encoding utf8 "$OutDir\report.md"
  Write-Host $text
  return $rows
}
function Shot([string]$name) {
  try {
    Add-Type -AssemblyName System.Drawing, System.Windows.Forms
    $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
    $bmp.Save("$OutDir\$name.png", [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    Write-Host "screenshot $name.png ($($b.Width)x$($b.Height))"
  } catch { Write-Host "screenshot failed: $_" }
}
function Uia([string]$label) {
  $out = @("### UI 要素: $label")
  try {
    Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
    $targets = [W]::All() | Where-Object { $_.Visible -and (ProcName $_.Pid) -match "TextInputHost|InputApp|Voice|Speech" }
    foreach ($t in $targets) {
      $out += "window $("0x{0:X}" -f $t.Hwnd) class=$($t.Cls) title='$($t.Title)' proc=$(ProcName $t.Pid)"
      $root = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]$t.Hwnd)
      $all = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
      $n = 0
      foreach ($e in $all) {
        $n++; if ($n -gt 60) { $out += "  ... (60 件で打ち切り)"; break }
        $out += "  $($e.Current.ControlType.ProgrammaticName) name='$($e.Current.Name)' id='$($e.Current.AutomationId)'"
      }
      if ($n -eq 0) { $out += "  (子要素なし)" }
    }
    if (@($targets).Count -eq 0) { $out += "(対象のウィンドウなし)" }
  } catch { $out += "UIA failed: $_" }
  $text = ($out -join "`n") + "`n"
  $text | Out-File -Append -Encoding utf8 "$OutDir\report.md"
  Write-Host $text
}

"# 注入した Win+H の観測`n" | Out-File -Encoding utf8 "$OutDir\report.md"
$null = Dump "0. 開始時"
Shot "0-start"

# 対照: 何も起こさないはずの Shift のタップ
$c1 = [Kb]::Key(0x10, $false); $c2 = [Kb]::Key(0x10, $true)
Write-Host "control Shift: $c1 $c2"
Start-Sleep -Seconds 3
$null = Dump "1. 対照(Shift のタップ)3 秒後"
Shot "1-control"

# Win+H
$r = @([Kb]::Key(0x5B, $false), [Kb]::Key(0x48, $false), [Kb]::Key(0x48, $true), [Kb]::Key(0x5B, $true))
Write-Host "Win+H SendInput: $($r -join ' ')"
Start-Sleep -Seconds 4
$null = Dump "2. Win+H 4 秒後"
Shot "2-winh"
Uia "Win+H 4 秒後"

# もう一度 Win+H(トグルで消えるか)
$r = @([Kb]::Key(0x5B, $false), [Kb]::Key(0x48, $false), [Kb]::Key(0x48, $true), [Kb]::Key(0x5B, $true))
Write-Host "Win+H again: $($r -join ' ')"
Start-Sleep -Seconds 3
$null = Dump "3. もう一度 Win+H 3 秒後"
Shot "3-winh-again"
