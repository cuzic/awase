# 実機の半自動測定: 自前の窓(indicator_scenario)で GJI のモードを切り替え、準備ができたら通知音を鳴らす。
# 利用者がインジケーターを右クリックしたら、メニュー(#32768)の MSAA の CHECKED 項目の位置を読み、実状態(TRUTH)と突き合わせる。
# 記録: 状態の数値と項目の位置のみ。他のアプリの内容は読まない。
param([string]$Bin = 'C:\Users\cuzic\awase-menu\target\debug\examples', [string]$Out = 'C:\Users\cuzic\menu-match2-out', [int]$Rounds = 2)
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force -Path $Out | Out-Null
Add-Type -AssemblyName System.Windows.Forms, UIAutomationClient, UIAutomationTypes, Accessibility, PresentationCore
Add-Type -ReferencedAssemblies 'Accessibility' -TypeDefinition @'
using System; using System.Runtime.InteropServices; using System.Text; using Accessibility;
public static class Acc2 {
  [DllImport("oleacc.dll")] static extern int AccessibleObjectFromWindow(IntPtr hwnd, uint id, ref Guid riid, [MarshalAs(UnmanagedType.Interface)] out IAccessible acc);
  [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  public static string Fg() { IntPtr h = GetForegroundWindow(); StringBuilder sb = new StringBuilder(128); GetClassName(h, sb, 128); return sb.ToString(); }
  public static string Checked(IntPtr hwnd) {
    IAccessible a; Guid g = new Guid("618736E0-3C3D-11CF-810C-00AA00389B71");
    if (AccessibleObjectFromWindow(hwnd, 0xFFFFFFFC, ref g, out a) != 0 || a == null) return "n=-1;checked=";
    int n = 0; try { n = a.accChildCount; } catch { }
    StringBuilder sb = new StringBuilder();
    for (int i = 1; i <= n; i++) { try { int st = (int)a.get_accState((object)i); if ((st & 0x10) != 0) { if (sb.Length > 0) sb.Append('+'); sb.Append(i); } } catch { } }
    return "n=" + n + ";checked=" + sb.ToString();
  }
}
'@
$A = [System.Windows.Automation.AutomationElement]; $TS = [System.Windows.Automation.TreeScope]; $tc = [System.Windows.Automation.Condition]::TrueCondition
function Log($m) { $l = "[$([DateTime]::UtcNow.ToString('HH:mm:ss.fffZ'))] $m"; Add-Content -Path "$Out\run.log" -Value $l -Encoding utf8 }
function Beep { try { [System.Media.SystemSounds]::Exclamation.Play() } catch {}; try { [Console]::Beep(880, 250) } catch {} }
function Find-Popup { foreach ($x in $A::RootElement.FindAll($TS::Children, $tc)) { if ($x.Current.ClassName -eq '#32768') { return $x } }; return $null }
Remove-Item "$Out\run.log", "$Out\match2.tsv" -ErrorAction SilentlyContinue
$aw = Get-Process awase -ErrorAction SilentlyContinue | Select-Object -First 1
$awPath = $null; if ($aw) { $awPath = $aw.Path }
Stop-Process -Name awase -Force -ErrorAction SilentlyContinue
Log "awase 停止(再開パス=$awPath)"
"round`tkey`ttruth_open`ttruth_conv`ttyped_kind`tfg_before`tfg_at_read`tmenu" | Set-Content "$Out\match2.tsv" -Encoding utf8
$keys = @('F2', 'F1', 'F3', 'F4', 'F0', '1A')
try {
  for ($r = 1; $r -le $Rounds; $r++) {
    foreach ($k in ($keys | Get-Random -Count $keys.Count)) {
      $d = "$Out\r$r-$k"; New-Item -ItemType Directory -Force -Path $d | Out-Null
      $p = Start-Process -FilePath "$Bin\indicator_scenario.exe" -ArgumentList "--scen=cycle --keys=$k --type=1 --n=1 --hold=40 --log=$d\scen.log" -PassThru -WindowStyle Hidden
      $deadline = [DateTime]::UtcNow.AddSeconds(20)
      while ([DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 300; if ((Get-Content "$d\scen.log" -ErrorAction SilentlyContinue | Where-Object { $_ -match 'CYCLE i=' }).Count -gt 0) { break } }
      $truth = (Get-Content "$d\scen.log" -ErrorAction SilentlyContinue | Where-Object { $_ -match 'TRUTH' } | Select-Object -Last 1)
      $to = ''; $tcv = ''; if ($truth -match 'open=(\S+) conv=(\S+)') { $to = $Matches[1]; $tcv = $Matches[2] }
      $typed = (Get-Content "$d\scen.log" -ErrorAction SilentlyContinue | Where-Object { $_ -match 'TYPED' } | Select-Object -Last 1)
      $kind = 'none'
      if ($typed -match 'text="([^"]*)"') {
        $t = $Matches[1]
        if ($t -match '^[\u3040-\u309F]+$') { $kind = 'hiragana' } elseif ($t -match '^[\u30A0-\u30FF]+$') { $kind = 'katakana-full' } elseif ($t -match '^[\uFF61-\uFF9F]+$') { $kind = 'katakana-half' } elseif ($t -match '^[\uFF21-\uFF5A]+$') { $kind = 'alnum-full' } elseif ($t -match '^[A-Za-z]+$') { $kind = 'alnum-half' } else { $kind = "other(len=$($t.Length))" }
      }
      Log "r$r key=$k 準備完了 truth=open:$to conv:$tcv → 右クリックを待つ"
      Start-Sleep -Milliseconds 1200
      $fgBefore = [Acc2]::Fg()
      Beep
      $res = 'no-menu'; $fgRead = ''; $wait = [DateTime]::UtcNow.AddSeconds(22)
      while ([DateTime]::UtcNow -lt $wait) {
        Start-Sleep -Milliseconds 150
        $pop = Find-Popup
        if ($pop) { $fgRead = [Acc2]::Fg(); Start-Sleep -Milliseconds 400; $res = [Acc2]::Checked([IntPtr]$pop.Current.NativeWindowHandle); break }
      }
      [System.Windows.Forms.SendKeys]::SendWait('{ESC}'); Start-Sleep -Milliseconds 800
      "$r`t$k`t$to`t$tcv`t$kind`t$fgBefore`t$fgRead`t$res" | Add-Content "$Out\match2.tsv" -Encoding utf8
      Log "r$r key=$k truth=open:$to conv:$tcv typed=$kind fg=$fgBefore/$fgRead menu=$res"
      Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; Start-Sleep -Milliseconds 1500
    }
  }
} finally {
  & "$Bin\indicator_scenario.exe" --scen=cycle --n=0 "--log=$Out\restore.log" | Out-Null
  if ($awPath) { Start-Process -FilePath $awPath -WorkingDirectory (Split-Path $awPath) -WindowStyle Hidden; Log 'awase 再開' }
  Log '完了'
}
