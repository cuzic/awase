# awase とは完全に独立した、生の WH_KEYBOARD_LL フックのログ取得専用ツール。
# BUG-113 調査用: 無変換キー押下時に本当に vk=0xF2 (かなキー) の
# スキャンコードが届いているのか、awase 自身のフック/処理を経由せずに
# 直接確認する。
#
# 使い方:
#   powershell -NoProfile -ExecutionPolicy Bypass -File rawkbd_logger.ps1 -LogPath C:/rawkbd.log
#
# 何もキーを消費/抑制しない（CallNextHookEx を必ず呼ぶ、Suppress は一切しない）。
# awase.exe のプロセス・コードとは無関係な別プロセスとして動作する。

param(
    [string]$LogPath = "C:/rawkbd.log"
)

Add-Type -ReferencedAssemblies System.Windows.Forms -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Windows.Forms;
using System.IO;
using System.Globalization;
using System.Diagnostics;

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
    // QueryPerformanceCounter 由来、DateTime.Now (~15ms分解能) より遥かに高精度。
    // イベント間の真の間隔（本物のADR-149が確認した「0.5ms間隔」相当）を
    // 判別するために使う。
    private static readonly Stopwatch _sw = Stopwatch.StartNew();

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
        _writer = new StreamWriter(logPath, true);
        _writer.AutoFlush = true;
        _writer.WriteLine("=== rawkbd_logger started (awaseとは無関係の独立プロセス) pid=" + System.Diagnostics.Process.GetCurrentProcess().Id + " at " + DateTime.Now.ToString("o"));
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
            long qpcUs = (long)(_sw.Elapsed.Ticks / (double)(TimeSpan.TicksPerMillisecond) * 1000.0);
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
