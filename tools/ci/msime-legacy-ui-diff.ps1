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
