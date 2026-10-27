# Firefox のアドレスバー(Ctrl+L)へフォーカスを移したとき、前面窓・フォーカス窓のクラスと
# awase.log の FocusTransition/Engine ログがどうなるかを観測する(報告 01M5SATXPZWZ7K1JSKR7TWNW5C)。
# 観測のみ。失敗させない。
$ErrorActionPreference = 'Continue'
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices; using System.Text;
public static class W {
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int l,t,r,b; }
  [StructLayout(LayoutKind.Sequential)] public struct GTI { public int cb; public int flags; public IntPtr active, focus, capture, menu, move, caret; public RECT rc; }
  [DllImport("user32.dll")] public static extern bool GetGUIThreadInfo(uint tid, ref GTI g);
  public static string Cls(IntPtr h){ var s=new StringBuilder(256); GetClassName(h,s,256); return s.ToString(); }
  public static string Info(){
    IntPtr fg=GetForegroundWindow(); uint pid; uint tid=GetWindowThreadProcessId(fg,out pid);
    var g=new GTI(); g.cb=Marshal.SizeOf(typeof(GTI)); GetGUIThreadInfo(tid,ref g);
    return "fg="+Cls(fg)+" pid="+pid+" focus="+(g.focus==IntPtr.Zero?"(none)":Cls(g.focus))+" caret="+(g.caret!=IntPtr.Zero);
  }
  public static void Key(byte vk){ keybd_event(vk,0,0,UIntPtr.Zero); keybd_event(vk,0,2,UIntPtr.Zero); }
  public static void Chord(byte m, byte vk){ keybd_event(m,0,0,UIntPtr.Zero); Key(vk); keybd_event(m,0,2,UIntPtr.Zero); }
}
'@
$ff = (Get-Command firefox -ErrorAction SilentlyContinue).Source
if (-not $ff) { $ff = "$env:ProgramFiles\Mozilla Firefox\firefox.exe" }
Write-Host "firefox: $ff exists=$(Test-Path $ff)"
Start-Process $ff -ArgumentList 'about:blank'
Start-Sleep -Seconds 12
Write-Host "[after launch]    $([W]::Info())"
[W]::Chord(0x11, 0x4C)   # Ctrl+L
Start-Sleep -Seconds 2
Write-Host "[after Ctrl+L]    $([W]::Info())"
foreach ($vk in 0x4B,0x59,0x4F,0x55) { [W]::Key($vk); Start-Sleep -Milliseconds 150 }
Start-Sleep -Seconds 1
Write-Host "[after typing]    $([W]::Info())"
[W]::Key(0x1B); [W]::Key(0x1B)   # Esc: アドレスバーから戻す
Start-Sleep -Seconds 2
Write-Host "[after Esc]       $([W]::Info())"
