# Independent WH_KEYBOARD_LL hook logger, unrelated to awase.exe's own code.
# BUG-113 investigation: verify whether the raw hardware event for the
# muhenkan key really is vk=0xF2 (kana key) or something else, without
# going through awase's own hook/processing at all.
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File rawkbd_logger.ps1 -LogPath C:/rawkbd.log
#
# Never consumes/suppresses any key (always calls CallNextHookEx).
# Runs as a separate process unrelated to awase.exe.

param(
    [string]$LogPath = "C:/rawkbd.log"
)

Add-Type -ReferencedAssemblies System.Windows.Forms -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Windows.Forms;
using System.IO;
using System.Globalization;

public class RawKbdLogger
{
    private const int WH_KEYBOARD_LL = 13;
    private const int WM_KEYDOWN = 0x0100;
    private const int WM_KEYUP = 0x0101;
    private const int WM_SYSKEYDOWN = 0x0104;
    private const int WM_SYSKEYUP = 0x0105;
    private const uint LLKHF_INJECTED = 0x10;
    private const uint LLKHF_LOWER_IL_INJECTED = 0x02;
    private const uint LLKHF_EXTENDED = 0x01;

    private static LowLevelKeyboardProc _proc = HookCallback;
    private static IntPtr _hookID = IntPtr.Zero;
    private static StreamWriter _writer;
    private static long _qpcFreq;
    private static long _qpcStart;

    [DllImport("kernel32.dll")]
    private static extern bool QueryPerformanceCounter(out long lpPerformanceCount);

    [DllImport("kernel32.dll")]
    private static extern bool QueryPerformanceFrequency(out long lpFrequency);

    [StructLayout(LayoutKind.Sequential)]
    public struct KBDLLHOOKSTRUCT
    {
        public uint vkCode;
        public uint scanCode;
        public uint flags;
        public uint time;
        public IntPtr dwExtraInfo;
    }

    public delegate IntPtr LowLevelKeyboardProc(int nCode, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll", CharSet = CharSet.Auto, SetLastError = true)]
    public static extern IntPtr SetWindowsHookEx(int idHook, LowLevelKeyboardProc lpfn, IntPtr hMod, uint dwThreadId);

    [DllImport("user32.dll", CharSet = CharSet.Auto, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool UnhookWindowsHookEx(IntPtr hhk);

    [DllImport("user32.dll", CharSet = CharSet.Auto, SetLastError = true)]
    public static extern IntPtr CallNextHookEx(IntPtr hhk, int nCode, IntPtr wParam, IntPtr lParam);

    [DllImport("kernel32.dll", CharSet = CharSet.Auto, SetLastError = true)]
    public static extern IntPtr GetModuleHandle(string lpModuleName);

    public static void Start(string logPath)
    {
        QueryPerformanceFrequency(out _qpcFreq);
        QueryPerformanceCounter(out _qpcStart);
        _writer = new StreamWriter(logPath, true);
        _writer.AutoFlush = true;
        _writer.WriteLine("=== rawkbd_logger started (independent of awase) pid=" + System.Diagnostics.Process.GetCurrentProcess().Id + " at " + DateTime.Now.ToString("o"));
        _hookID = SetHook(_proc);
        Application.Run();
    }

    private static IntPtr SetHook(LowLevelKeyboardProc proc)
    {
        using (var curProcess = System.Diagnostics.Process.GetCurrentProcess())
        using (var curModule = curProcess.MainModule)
        {
            return SetWindowsHookEx(WH_KEYBOARD_LL, proc, GetModuleHandle(curModule.ModuleName), 0);
        }
    }

    private static IntPtr HookCallback(int nCode, IntPtr wParam, IntPtr lParam)
    {
        if (nCode >= 0)
        {
            var hookStruct = (KBDLLHOOKSTRUCT)Marshal.PtrToStructure(lParam, typeof(KBDLLHOOKSTRUCT));
            string action = "";
            int w = (int)wParam;
            if (w == WM_KEYDOWN) action = "DOWN";
            else if (w == WM_KEYUP) action = "UP";
            else if (w == WM_SYSKEYDOWN) action = "SYSDOWN";
            else if (w == WM_SYSKEYUP) action = "SYSUP";
            bool injected = (hookStruct.flags & LLKHF_INJECTED) != 0;
            bool lowerInjected = (hookStruct.flags & LLKHF_LOWER_IL_INJECTED) != 0;
            bool extended = (hookStruct.flags & LLKHF_EXTENDED) != 0;
            long qpcNow;
            QueryPerformanceCounter(out qpcNow);
            long qpcUs = (_qpcFreq > 0) ? (long)((qpcNow - _qpcStart) * 1000000.0 / _qpcFreq) : 0;
            _writer.WriteLine(string.Format(CultureInfo.InvariantCulture,
                "{0} qpc_us={1} {2,-7} vk=0x{3:X2} scan=0x{4:X2} flags=0x{5:X} injected={6} lowerInjected={7} extended={8} os_time_ms={9}",
                DateTime.Now.ToString("HH:mm:ss.fffffff"), qpcUs, action, hookStruct.vkCode, hookStruct.scanCode,
                hookStruct.flags, injected, lowerInjected, extended, hookStruct.time));
        }
        return CallNextHookEx(_hookID, nCode, wParam, lParam);
    }

    public static void Stop()
    {
        if (_hookID != IntPtr.Zero)
        {
            UnhookWindowsHookEx(_hookID);
        }
        if (_writer != null)
        {
            _writer.WriteLine("=== rawkbd_logger stopped at " + DateTime.Now.ToString("o"));
            _writer.Flush();
        }
    }
}
"@

[RawKbdLogger]::Start($LogPath)
