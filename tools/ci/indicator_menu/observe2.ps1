# 実機観察 2: 手動の右クリックで出た #32768 の項目を、MSAA(IAccessible)と UIA の RawView で読む。注入はしない。
# 記録: 役割・状態・名前(IME のメニュー項目)。他の窓は読まない。
param([string]$Out = 'C:\Users\cuzic\menu-diag-out', [int]$Seconds = 75)
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force -Path $Out | Out-Null
Add-Type -AssemblyName System.Windows.Forms, UIAutomationClient, UIAutomationTypes, Accessibility
Add-Type -ReferencedAssemblies 'Accessibility' -TypeDefinition @'
using System; using System.Runtime.InteropServices; using System.Text; using Accessibility;
public static class Acc {
  [DllImport("oleacc.dll")] static extern int AccessibleObjectFromWindow(IntPtr hwnd, uint id, ref Guid riid, [MarshalAs(UnmanagedType.Interface)] out IAccessible acc);
  [DllImport("oleacc.dll")] static extern uint GetStateText(uint state, StringBuilder sb, uint n);
  [DllImport("oleacc.dll")] static extern uint GetRoleText(uint role, StringBuilder sb, uint n);
  public static string Dump(IntPtr hwnd, uint objid) {
    IAccessible a; Guid g = new Guid("618736E0-3C3D-11CF-810C-00AA00389B71");
    int hr = AccessibleObjectFromWindow(hwnd, objid, ref g, out a);
    if (hr != 0 || a == null) return "AccessibleObjectFromWindow hr=0x" + hr.ToString("X");
    StringBuilder sb = new StringBuilder();
    int n = 0; try { n = a.accChildCount; } catch (Exception e) { sb.AppendLine("accChildCount失敗 " + e.Message); }
    sb.AppendLine("objid=" + (int)objid + " childCount=" + n);
    for (int i = 1; i <= n && i <= 40; i++) {
      object c = i;
      string name = "", role = "", state = "";
      try { name = a.get_accName(c); } catch { }
      try { StringBuilder r = new StringBuilder(64); GetRoleText((uint)(int)a.get_accRole(c), r, 64); role = r.ToString(); } catch { }
      try { int st = (int)a.get_accState(c); state = "0x" + st.ToString("X") + ((st & 0x10) != 0 ? " CHECKED" : "") + ((st & 0x1000000) != 0 ? " (SYSTEM_UNAVAILABLE?)" : ""); } catch { }
      sb.AppendLine("  [" + i + "] role=" + role + " state=" + state + " name=" + name);
    }
    return sb.ToString();
  }
}
'@
$A = [System.Windows.Automation.AutomationElement]; $TS = [System.Windows.Automation.TreeScope]; $tc = [System.Windows.Automation.Condition]::TrueCondition
function Log($m) { Add-Content -Path "$Out\observe2.log" -Value ("[" + [DateTime]::UtcNow.ToString('HH:mm:ss.fff') + "Z] " + $m) -Encoding utf8 }
Remove-Item "$Out\observe2.log" -ErrorAction SilentlyContinue
Log 'start'
$seen = @{}; $end = [DateTime]::UtcNow.AddSeconds($Seconds)
while ([DateTime]::UtcNow -lt $end) {
  Start-Sleep -Milliseconds 120
  foreach ($x in $A::RootElement.FindAll($TS::Children, $tc)) {
    if ($x.Current.ClassName -eq '#32768') {
      $k = $x.Current.NativeWindowHandle
      if ($seen.ContainsKey($k)) { continue }
      $seen[$k] = 1; Log "found #32768 hwnd=0x$($k.ToString('X'))"
      Start-Sleep -Milliseconds 400
      foreach ($oid in 4294967292, 4294967295, 0) {   # OBJID_CLIENT(-4), OBJID_MENU(-3 は 4294967293)、OBJID_WINDOW(0)
        try { Log ("MSAA " + [Acc]::Dump([IntPtr]$k, [uint32]$oid)) } catch { Log "MSAA例外 $($_.Exception.Message)" }
      }
      try { Log ("MSAA " + [Acc]::Dump([IntPtr]$k, 4294967293)) } catch { Log "MSAA例外 $($_.Exception.Message)" }
      # UIA RawView
      try {
        $w = [System.Windows.Automation.TreeWalker]::RawViewWalker; $cnt = 0
        $c = $w.GetFirstChild($x)
        while ($c -and $cnt -lt 40) { $cnt++; Log "  raw child ctype=$($c.Current.ControlType.ProgrammaticName) id=$($c.Current.AutomationId) name=$($c.Current.Name)"; $c = $w.GetNextSibling($c) }
        Log "raw children=$cnt"
      } catch { Log "RawView失敗 $($_.Exception.Message)" }
    }
  }
}
Log 'end'
