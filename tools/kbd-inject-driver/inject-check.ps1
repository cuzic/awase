# ADR-255(使い捨て): awasekbf ドライバ経由のキーが、SendInput の注入キーと違って「物理扱い」(LLKHF_INJECTED なし・dwExtraInfo 0)で
# WH_KEYBOARD_LL に届くかを測る。ドライバ(\\.\AwaseKbInject)に make/break のスキャンコードを渡し、同じ低レベルフックで SendInput の注入も記録して比べる。
Add-Type -AssemblyName System.Windows.Forms
$src = @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class AwaseKb {
  [DllImport("kernel32.dll", SetLastError=true, CharSet=CharSet.Unicode)] public static extern IntPtr CreateFile(string n, uint a, uint s, IntPtr sa, uint d, uint f, IntPtr t);
  [DllImport("kernel32.dll", SetLastError=true)] public static extern bool DeviceIoControl(IntPtr h, uint code, byte[] inb, int inl, byte[] outb, int outl, out int ret, IntPtr ov);
  [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr h);
  [DllImport("user32.dll", SetLastError=true)] static extern uint SendInput(uint n, INPUT[] i, int size);
  [DllImport("user32.dll")] static extern IntPtr SetWindowsHookEx(int id, LowLevelProc p, IntPtr mod, uint tid);
  [DllImport("user32.dll")] static extern IntPtr CallNextHookEx(IntPtr h, int code, IntPtr w, IntPtr l);
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode)] static extern IntPtr GetModuleHandle(string n);
  [StructLayout(LayoutKind.Sequential)] struct KBDLL { public uint vk, scan, flags, time; public UIntPtr extra; }
  [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort vk, scan; public uint flags, time; public UIntPtr extra; }
  [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx, dy; public uint data, flags, time; public UIntPtr extra; }
  [StructLayout(LayoutKind.Explicit)] struct U { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
  [StructLayout(LayoutKind.Sequential)] struct INPUT { public uint type; public U u; }
  delegate IntPtr LowLevelProc(int code, IntPtr w, IntPtr l);
  static LowLevelProc proc = Hook;
  static IntPtr hook = IntPtr.Zero;
  public static List<string> Log = new List<string>();
  static IntPtr Hook(int code, IntPtr w, IntPtr l) {
    if (code >= 0) {
      KBDLL k = (KBDLL)Marshal.PtrToStructure(l, typeof(KBDLL));
      bool down = (int)w == 0x100 || (int)w == 0x104;
      Log.Add(string.Format("vk=0x{0:X2} scan=0x{1:X2} flags=0x{2:X2} injected={3} extra=0x{4:X} down={5}", k.vk, k.scan, k.flags, (k.flags & 0x10) != 0 ? 1 : 0, (ulong)k.extra, down ? 1 : 0));
    }
    return CallNextHookEx(hook, code, w, l);
  }
  public static bool Install() { hook = SetWindowsHookEx(13, proc, GetModuleHandle(null), 0); return hook != IntPtr.Zero; }
  public static void SendKey(ushort vk, ushort scan, bool up, ulong extra) {
    INPUT[] a = new INPUT[1];
    a[0].type = 1; a[0].u.ki.vk = vk; a[0].u.ki.scan = scan; a[0].u.ki.flags = up ? 2u : 0u; a[0].u.ki.extra = new UIntPtr(extra);
    SendInput(1, a, Marshal.SizeOf(typeof(INPUT)));
  }
  public static int Inject(IntPtr h, ushort[] makeAndFlags) {
    byte[] b = new byte[makeAndFlags.Length * 2];
    Buffer.BlockCopy(makeAndFlags, 0, b, 0, b.Length);
    int ret; bool ok = DeviceIoControl(h, 0x0000B2000 | 0, b, b.Length, null, 0, out ret, IntPtr.Zero);
    return ok ? 0 : Marshal.GetLastWin32Error();
  }
}
'@
try { Add-Type -TypeDefinition $src -ErrorAction Stop } catch { Write-Host "Add-Type failed: $($_.Exception.Message)"; $_.Exception | Format-List * -Force | Out-String | Write-Host; if ($_.Exception.InnerException) { Write-Host $_.Exception.InnerException.Message }; exit 4 }
# CTL_CODE(FILE_DEVICE_KEYBOARD=0x0b, 0x800, METHOD_BUFFERED=0, FILE_ANY_ACCESS=0) = (0x0b<<16)|(0<<14)|(0x800<<2)|0 = 0x000B2000
$h = [AwaseKb]::CreateFile('\\.\AwaseKbInject', [uint32]3221225472, 3, [IntPtr]::Zero, 3, 0, [IntPtr]::Zero)
Write-Host "CreateFile handle=$h lasterror=$([Runtime.InteropServices.Marshal]::GetLastWin32Error())"
if ($h -eq [IntPtr]::new(-1)) { Write-Host '::error::制御デバイスを開けない'; exit 3 }
Write-Host "LL hook installed: $([AwaseKb]::Install())"
function Pump($ms) { $end = (Get-Date).AddMilliseconds($ms); while ((Get-Date) -lt $end) { [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 10 } }
Pump 300
# ドライバ経由: A(スキャン 0x1E) の make → break。
$r1 = [AwaseKb]::Inject($h, [uint16[]](0x1E, 0)); Pump 200
$r2 = [AwaseKb]::Inject($h, [uint16[]](0x1E, 1)); Pump 300
Write-Host "driver inject results: make=$r1 break=$r2"
# SendInput(目印なし): S。 SendInput(目印付き 0x5350494B): D。
[AwaseKb]::SendKey(0x53, 0x1F, $false, 0); Pump 100; [AwaseKb]::SendKey(0x53, 0x1F, $true, 0); Pump 300
[AwaseKb]::SendKey(0x44, 0x20, $false, 0x5350494B); Pump 100; [AwaseKb]::SendKey(0x44, 0x20, $true, 0x5350494B); Pump 300
# ドライバ経由で E0 付きのキー(右 Ctrl 相当 0x1D+E0)と、押しっぱなしの自動リピート相当(make を連続 5 回→break)。
$r3 = [AwaseKb]::Inject($h, [uint16[]](0x23, 0)); Pump 100
for ($i = 0; $i -lt 4; $i++) { [void][AwaseKb]::Inject($h, [uint16[]](0x23, 0)); Pump 60 }
$r4 = [AwaseKb]::Inject($h, [uint16[]](0x23, 1)); Pump 300
[void][AwaseKb]::CloseHandle($h)
Write-Host '--- LL hook records ---'
[AwaseKb]::Log | ForEach-Object { Write-Host $_ }
$phys = @([AwaseKb]::Log | Where-Object { $_ -match 'vk=0x41 ' -or $_ -match 'vk=0x48 ' })
$sInj = @([AwaseKb]::Log | Where-Object { $_ -match 'vk=0x53 ' })
$dInj = @([AwaseKb]::Log | Where-Object { $_ -match 'vk=0x44 ' })
$summary = @('| 経路 | 記録 |', '| --- | --- |')
$summary += "| ドライバ経由(A、H) | $($phys -join ' ; ') |"
$summary += "| SendInput 目印なし(S) | $($sInj -join ' ; ') |"
$summary += "| SendInput 目印付き(D) | $($dInj -join ' ; ') |"
$summary | ForEach-Object { Write-Host $_ }
if ($env:GITHUB_STEP_SUMMARY) { $summary | Add-Content -Path $env:GITHUB_STEP_SUMMARY -Encoding utf8 }
$ok = ($phys.Count -ge 2) -and (@($phys | Where-Object { $_ -match 'injected=0' }).Count -eq $phys.Count) -and (@($sInj | Where-Object { $_ -match 'injected=1' }).Count -ge 2)
Write-Host "RESULT driver-injected is physical-like: $ok"
if (-not $ok) { exit 1 }
