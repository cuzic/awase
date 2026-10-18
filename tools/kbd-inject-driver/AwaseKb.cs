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
  public static string Status(IntPtr h) {
    byte[] o = new byte[28]; int ret;
    bool ok = DeviceIoControl(h, 0x000B2004, null, 0, o, o.Length, out ret, IntPtr.Zero);
    if (!ok) return "status ioctl failed " + Marshal.GetLastWin32Error();
    return string.Format("jis={0} seenAny={1} seenQuery={2} modified={3} origType={4} origSubtype={5} connected={6}", BitConverter.ToUInt32(o,0), BitConverter.ToUInt32(o,4), BitConverter.ToUInt32(o,8), BitConverter.ToUInt32(o,12), BitConverter.ToUInt32(o,16), BitConverter.ToUInt32(o,20), BitConverter.ToUInt32(o,24));
  }
  public static int Inject(IntPtr h, ushort[] makeAndFlags) {
    byte[] b = new byte[makeAndFlags.Length * 2];
    Buffer.BlockCopy(makeAndFlags, 0, b, 0, b.Length);
    int ret; bool ok = DeviceIoControl(h, 0x0000B2000 | 0, b, b.Length, null, 0, out ret, IntPtr.Zero);
    return ok ? 0 : Marshal.GetLastWin32Error();
  }
}
