# 旧UI(IMJPUEX.EXE)を UI Automation で操作し、操作の前後でレジストリ全体・ファイルの差分を取るスパイク。観測のみ。
param([string]$Out = 'ui-out', [string]$Phase = 'discover')
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$log = Join-Path $Out 'ui.txt'
function Say([string]$s) { $s | Tee-Object -FilePath $log -Append }
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing, System.Windows.Forms
$AE = [Windows.Automation.AutomationElement]

function Shot([string]$name) {
  try {
    $b = [Windows.Forms.SystemInformation]::VirtualScreen
    $bmp = New-Object Drawing.Bitmap $b.Width, $b.Height
    $g = [Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
    $bmp.Save((Join-Path $Out "$name.png"), [Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
  } catch { Say "shot failed: $_" }
}

function Dump-Tree($el, [int]$depth, [int]$max) {
  if ($depth -gt $max) { return }
  $c = $el.Current
  $pat = ($el.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName -replace 'PatternIdentifiers.Pattern', '' }) -join ','
  Say ("{0}{1} name='{2}' id='{3}' class='{4}' pat=[{5}]" -f ('  ' * $depth), $c.ControlType.ProgrammaticName.Replace('ControlType.', ''), $c.Name, $c.AutomationId, $c.ClassName, $pat)
  $w = [Windows.Automation.TreeWalker]::RawViewWalker
  $ch = $w.GetFirstChild($el)
  while ($ch) { Dump-Tree $ch ($depth + 1) $max; $ch = $w.GetNextSibling($ch) }
}

function Windows-Of([int]$procId) {
  $cond = New-Object Windows.Automation.PropertyCondition ($AE::ProcessIdProperty, $procId)
  $AE::RootElement.FindAll([Windows.Automation.TreeScope]::Children, $cond)
}

function Find-ByName($root, [string]$pattern) {
  $all = $root.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition)
  foreach ($e in $all) { if ($e.Current.Name -match $pattern) { return $e } }
  return $null
}

function Click($el) {
  try { $ip = $el.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern); $ip.Invoke(); return $true } catch {}
  try { $sp = $el.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern); $sp.Select(); return $true } catch {}
  return $false
}

# 互換モード ON(旧UIの前提)
$imejp = 'Software\Microsoft\IME\15.0\IMEJP'
$tsf = 'HKCU:\SOFTWARE\Microsoft\Input\TSF\Tsf3Override\{03b5835f-f03c-411b-9ce2-aa23e1171e36}'
New-Item -Path $tsf -Force | Out-Null
Set-ItemProperty -Path $tsf -Name NoTsf3Override2 -Value 1 -Type DWord
New-Item -Path "HKCU:\$imejp\MSIME" -Force | Out-Null
Set-ItemProperty -Path "HKCU:\$imejp\MSIME" -Name DisableNewIME -Value 1 -Type DWord
Get-Process ctfmon -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2; Start-Process ctfmon.exe; Start-Sleep -Seconds 4

$exe = "$env:windir\System32\IME\IMEJP\IMJPUEX.EXE"
Say "launch $exe"
$p = Start-Process -FilePath $exe -PassThru
Start-Sleep -Seconds 6
Shot 'main'
$ws = Windows-Of $p.Id
Say "top-level windows: $($ws.Count)"
foreach ($w in $ws) { Say "=== window '$($w.Current.Name)' class='$($w.Current.ClassName)'"; Dump-Tree $w 0 6 }
# 他プロセス名の窓(IMJPUEX が別プロセスで UI を出す場合に備え)
Get-Process | Where-Object { $_.ProcessName -match 'IMJP|ime' } | ForEach-Object { Say ("proc " + $_.ProcessName + " pid=" + $_.Id + " title='" + $_.MainWindowTitle + "'") }

# ---------------------------------------------------------------- Win32 操作(UIA では中身が取れないため)
if ($Phase -ne 'diff1') { return }
Add-Type -TypeDefinition @"
using System; using System.Collections.Generic; using System.Runtime.InteropServices; using System.Text;
public static class W {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc p, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc p, IntPtr l);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder sb, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder sb, int n);
  [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetParent(IntPtr h);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, StringBuilder l);
  public static List<IntPtr> Tops(uint pid) {
    var r = new List<IntPtr>();
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p); if (p == pid && IsWindowVisible(h)) r.Add(h); return true; }, IntPtr.Zero);
    return r;
  }
  public static List<IntPtr> Kids(IntPtr parent) {
    var r = new List<IntPtr>();
    EnumChildWindows(parent, (h, l) => { r.Add(h); return true; }, IntPtr.Zero);
    return r;
  }
  public static string Cls(IntPtr h) { var sb = new StringBuilder(256); GetClassName(h, sb, 256); return sb.ToString(); }
  public static string Txt(IntPtr h) { var sb = new StringBuilder(512); GetWindowText(h, sb, 512); return sb.ToString(); }
}
"@

function Dump-Win32([IntPtr]$h, [string]$label) {
  Say "--- Win32 dump: $label hwnd=$h class=$([W]::Cls($h)) text='$([W]::Txt($h))'"
  foreach ($k in [W]::Kids($h)) {
    Say ("   id={0} class={1} text='{2}' hwnd={3}" -f [W]::GetDlgCtrlID($k), [W]::Cls($k), [W]::Txt($k), $k)
  }
}
function Find-Ctl([IntPtr]$top, [int]$id, [string]$cls) {
  foreach ($k in [W]::Kids($top)) { if ([W]::GetDlgCtrlID($k) -eq $id -and ($cls -eq '' -or [W]::Cls($k) -eq $cls)) { return $k } }
  return [IntPtr]::Zero
}

# --- スナップショット
function Snapshot([string]$tag) {
  $d = Join-Path $Out "snap-$tag"; New-Item -ItemType Directory -Force -Path $d | Out-Null
  & reg.exe export HKCU (Join-Path $d 'hkcu.reg') /y | Out-Null
  & reg.exe export 'HKLM\SOFTWARE\Microsoft\IME' (Join-Path $d 'hklm-ime.reg') /y 2>&1 | Out-Null
  & reg.exe export 'HKLM\SOFTWARE\Microsoft\CTF' (Join-Path $d 'hklm-ctf.reg') /y 2>&1 | Out-Null
  Say "snapshot $tag: hkcu.reg=$((Get-Item (Join-Path $d 'hkcu.reg')).Length) bytes"
}
function Diff-Reg([string]$file) {
  $a = Get-Content (Join-Path $Out "snap-before\$file") -Encoding Unicode -ErrorAction SilentlyContinue
  $b = Get-Content (Join-Path $Out "snap-after\$file") -Encoding Unicode -ErrorAction SilentlyContinue
  if (-not $a -or -not $b) { Say "(diff $file: missing)"; return }
  $diff = Compare-Object $a $b | Where-Object { $_.InputObject -notmatch '^\s*$' }
  Say "=== reg diff $file : $($diff.Count) lines ==="
  $diff | Select-Object -First 400 | ForEach-Object { Say ("{0} {1}" -f $_.SideIndicator, ($_.InputObject.Substring(0, [Math]::Min(300, $_.InputObject.Length)))) }
}

$top = ([W]::Tops($p.Id))[0]
$kids = [W]::Kids($top)
Say "top hwnd=$top kids=$($kids.Count)"
$combo = Find-Ctl $top 1007 'ComboBox'
$parent = [W]::GetParent($combo)
Say "key template combo=$combo parent=$parent"
$n = [int][W]::SendMessage($combo, 0x146, [IntPtr]::Zero, [IntPtr]::Zero)   # CB_GETCOUNT
$cur = [int][W]::SendMessage($combo, 0x147, [IntPtr]::Zero, [IntPtr]::Zero) # CB_GETCURSEL
$items = @()
for ($i = 0; $i -lt $n; $i++) {
  $sb = New-Object Text.StringBuilder 256
  [void][W]::SendMessage($combo, 0x148, [IntPtr]$i, $sb)                     # CB_GETLBTEXT
  $items += $sb.ToString()
}
Say ("key template items=[" + ($items -join ' | ') + "] cursel=$cur")
$kt0 = (Get-ItemProperty "HKCU:\$imejp\MSIME" -ErrorAction SilentlyContinue).keystyle
Say "keystyle before = $kt0"

Snapshot 'before'
$t0 = Get-Date
$target = -1
for ($i = 0; $i -lt $items.Count; $i++) { if ($items[$i] -match 'ATOK') { $target = $i } }
Say "target index (ATOK) = $target"
if ($target -ge 0) {
  [void][W]::SendMessage($combo, 0x14E, [IntPtr]$target, [IntPtr]::Zero)     # CB_SETCURSEL
  $cid = 1007
  [void][W]::SendMessage($parent, 0x111, [IntPtr]((1 -shl 16) -bor $cid), $combo) # WM_COMMAND, CBN_SELCHANGE
  Start-Sleep -Seconds 1
  $apply = Find-Ctl $top 12321 'Button'
  [void][W]::SendMessage($apply, 0xF5, [IntPtr]::Zero, [IntPtr]::Zero)       # BM_CLICK
  Start-Sleep -Seconds 4
}
Shot 'after-apply'
foreach ($h in [W]::Tops($p.Id)) { Dump-Win32 $h 'after apply' }
Say "keystyle after = $((Get-ItemProperty "HKCU:\$imejp\MSIME" -ErrorAction SilentlyContinue).keystyle)"
Snapshot 'after'
Diff-Reg 'hkcu.reg'
Diff-Reg 'hklm-ime.reg'
Diff-Reg 'hklm-ctf.reg'

# 変更されたファイル(AppData 等)
Say '=== files modified since action ==='
foreach ($root in @($env:APPDATA, $env:LOCALAPPDATA, "$env:USERPROFILE\AppData\LocalLow", $env:ProgramData, "$env:windir\System32\IME")) {
  Get-ChildItem $root -Recurse -File -Force -ErrorAction SilentlyContinue |
    Where-Object { $_.LastWriteTime -ge $t0 -and $_.FullName -notmatch '\\(Temp|Packages|Microsoft\\Windows\\(Explorer|Notifications)|Code Cache|GPUCache)\\' } |
    Select-Object -First 60 | ForEach-Object { Say ("{0}  {1}  {2}" -f $_.LastWriteTime.ToString('HH:mm:ss'), $_.Length, $_.FullName) }
}

# --- Advanced(キー編集)を開いて構造を採取
$adv = Find-Ctl $top 1151 'Button'
Say "advanced button=$adv"
[void][W]::SendMessage($adv, 0xF5, [IntPtr]::Zero, [IntPtr]::Zero)
Start-Sleep -Seconds 3
Shot 'advanced'
$tops = [W]::Tops($p.Id)
Say "windows after Advanced click: $($tops.Count)"
foreach ($h in $tops) { Dump-Win32 $h 'advanced' }
