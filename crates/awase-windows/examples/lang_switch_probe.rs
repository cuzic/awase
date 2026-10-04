#![allow(clippy::all, clippy::pedantic, clippy::nursery)]
//! BUG-183(入力言語ホットキー経由でロシア語へ切り替えても awase が日本語入力のまま残る)の CI 再現用プローブ。
//!
//! 前面に自前の EDIT 窓を置き、日本語(MS-IME)→ロシア語への切替を方法ごとに注入し、前面スレッドの HKL を
//! 10ms 周期で読んで「実際に切り替わるまでの時間」を測る。awase(別プロセス、`AWASE_TEST_INJECTION=1` の
//! デバッグビルド)の判定は awase.log の `Engine deactivated/activated` と時刻突合する(集計は workflow 側)。
//!
//! 方法: `altshift`(Alt+Shift)、`winspace`(Win+Space)、`hk3`(入力言語ホットキー Alt+Shift+3。`ImmSetHotKey`
//! で IME_HOTKEY_DSWITCH_FIRST=0x100 にロシア語キーボード直接切替を登録してから押す)、`request`
//! (`WM_INPUTLANGCHANGEREQUEST` を前面窓へ送る。キー経路を介さない対照)。
//! 出力: `LS {json}` 行(1 試行 1 行)と、epoch ms の時刻。
//!
//! 実行: `lang_switch_probe [--trials=N] [--methods=altshift,winspace,hk3,request] [--log=<path>]`
//! 前提: 入力言語に ja-JP(Microsoft IME)と ru-RU が入っていること。

#![allow(unsafe_code)]

// `#[implement(...)]`（windows-rs）が生成する内部コードが pedantic/nursery deny に触れるため、マクロ生成部分にまとめて allow する
// （`compartment_notify_probe.rs` と同じ扱い）。
#[cfg(windows)]
#[allow(clippy::ref_as_ptr, clippy::inline_always)]
mod sinks {
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    use windows::core::{implement, BOOL, GUID};
    use windows::Win32::UI::TextServices::{
        ITfActiveLanguageProfileNotifySink, ITfActiveLanguageProfileNotifySink_Impl,
        ITfLanguageProfileNotifySink, ITfLanguageProfileNotifySink_Impl,
    };

    /// 通知イベント(epoch ms, 種別と詳細)。`shell`(HSHELL_*)・`tsf-act`・`tsf-langchange(d)` を共通で溜める。
    pub static EVENTS: Mutex<Vec<(u64, String)>> = Mutex::new(Vec::new());

    pub fn push(text: String) {
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(0))
            .unwrap_or(0);
        if let Ok(mut v) = EVENTS.lock() {
            v.push((at, text));
        }
    }

    #[implement(ITfActiveLanguageProfileNotifySink)]
    pub struct ActSink;

    impl ITfActiveLanguageProfileNotifySink_Impl for ActSink_Impl {
        fn OnActivated(
            &self,
            _rclsid: *const GUID,
            _guidprofile: *const GUID,
            factivated: BOOL,
        ) -> windows::core::Result<()> {
            push(format!("tsf-act activated={}", factivated.as_bool()));
            Ok(())
        }
    }

    #[implement(ITfLanguageProfileNotifySink)]
    pub struct LangSink;

    impl ITfLanguageProfileNotifySink_Impl for LangSink_Impl {
        fn OnLanguageChange(&self, langid: u16) -> windows::core::Result<BOOL> {
            // 変更を拒否しない(TRUE を返す)。
            push(format!("tsf-langchange langid=0x{langid:04X}"));
            Ok(BOOL(1))
        }

        fn OnLanguageChanged(&self) -> windows::core::Result<()> {
            push("tsf-langchanged".to_string());
            Ok(())
        }
    }
}

#[cfg(windows)]
mod p {
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use awase_windows::win32::to_wide;
    use windows::core::{w, Interface as _, BOOL, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardLayout, GetKeyboardLayoutList, SendInput, HKL, INPUT, INPUT_0, INPUT_KEYBOARD,
        KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    };
    use windows::Win32::UI::TextServices::{
        CLSID_TF_InputProcessorProfiles, CLSID_TF_ThreadMgr, ITfActiveLanguageProfileNotifySink,
        ITfLanguageProfileNotifySink, ITfSource, ITfThreadMgr,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, EnumWindows, GetClassNameW,
        GetForegroundWindow, GetGUIThreadInfo, GetWindowTextW, GetWindowThreadProcessId,
        IsWindowVisible, PeekMessageW, PostMessageW, RegisterClassW, RegisterShellHookWindow,
        RegisterWindowMessageW, SetForegroundWindow, ShowWindow, TranslateMessage, CS_HREDRAW,
        CS_VREDRAW, CW_USEDEFAULT, GUITHREADINFO, MSG, PM_REMOVE, SW_SHOW, WNDCLASSW,
        WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };

    const WM_INPUTLANGCHANGEREQUEST: u32 = 0x0050;
    /// awase のデバッグビルド + `AWASE_TEST_INJECTION=1` が「物理キー扱い」にする目印(`hook.rs` の `INJECTED_MARKER` 系)。
    const TEST_MARKER: usize = 0x5350_494B;
    const LANG_JA: u16 = 0x0411;
    const LANG_RU: u16 = 0x0419;

    static LAYOUT_RU_AT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    #[link(name = "imm32")]
    extern "system" {
        fn ImmSetHotKey(dw_hotkey: u32, modifiers: u32, vkey: u32, hkl: HKL) -> i32;
        fn ImmGetHotKey(dw_hotkey: u32, modifiers: *mut u32, vkey: *mut u32, hkl: *mut HKL) -> i32;
    }

    static SHELL_MSG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        let shell = SHELL_MSG.load(std::sync::atomic::Ordering::SeqCst);
        if shell != 0 && msg == shell {
            // wparam: HSHELL_LANGUAGE=8、HSHELL_WINDOWACTIVATED=4 など。lparam は HKL またはウィンドウ。
            super::sinks::push(format!("shell wparam={} lparam=0x{:X}", wp.0, lp.0));
        }
        unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
    }

    fn epoch_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(0))
            .unwrap_or(0)
    }

    fn log(path: &str, msg: &str) {
        use std::io::Write as _;
        println!("{msg}");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(f, "{msg}");
        }
    }

    fn pump(d: Duration) {
        let end = Instant::now() + d;
        while Instant::now() < end {
            let mut msg = MSG::default();
            while unsafe { PeekMessageW(&raw mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
                let _ = unsafe { TranslateMessage(&raw const msg) };
                unsafe { DispatchMessageW(&raw const msg) };
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn key(vk: u16, up: bool) {
        key_ex(vk, up, TEST_MARKER);
    }

    /// `extra` を dwExtraInfo に付けて注入する(0 = マーカーなし = 外部ソフト(PowerToys 等)の注入と同じ扱い)。
    fn key_ex(vk: u16, up: bool, extra: usize) {
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    // 文字キー(A=0x1E)はスキャンコードを付ける(awase が Char キーとして NICOLA 変換するため。typing_stress と同じ)。
                    // スキャンコード 0 だと `key_class=Passthrough` になり、日本語のままでも変換されない。
                    wScan: if vk == 0x41 { 0x1E } else { 0 },
                    dwFlags: if up {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    },
                    time: 0,
                    dwExtraInfo: extra,
                },
            },
        };
        let size = i32::try_from(size_of::<INPUT>()).expect("INPUT size");
        unsafe { SendInput(&[input], size) };
    }

    fn chord(mods: &[u16], vk: Option<u16>) {
        for &m in mods {
            key(m, false);
            pump(Duration::from_millis(30));
        }
        if let Some(v) = vk {
            key(v, false);
            pump(Duration::from_millis(30));
            key(v, true);
            pump(Duration::from_millis(30));
        }
        for &m in mods.iter().rev() {
            key(m, true);
            pump(Duration::from_millis(30));
        }
    }

    fn fg_lang() -> u16 {
        let fg = unsafe { GetForegroundWindow() };
        let tid = unsafe { GetWindowThreadProcessId(fg, None) };
        let hkl = unsafe { GetKeyboardLayout(tid) };
        (hkl.0 as usize & 0xFFFF) as u16
    }

    fn hkls() -> Vec<HKL> {
        let n = unsafe { GetKeyboardLayoutList(None) };
        let mut v = vec![HKL::default(); usize::try_from(n).unwrap_or(0)];
        let n2 = unsafe { GetKeyboardLayoutList(Some(&mut v)) };
        v.truncate(usize::try_from(n2).unwrap_or(0));
        v
    }

    fn hkl_of(list: &[HKL], lang: u16) -> Option<HKL> {
        list.iter()
            .copied()
            .find(|h| (h.0 as usize & 0xFFFF) as u16 == lang)
    }

    fn request(hkl: HKL) {
        let fg = unsafe { GetForegroundWindow() };
        let _ = unsafe {
            PostMessageW(
                Some(fg),
                WM_INPUTLANGCHANGEREQUEST,
                WPARAM(0),
                LPARAM(hkl.0 as isize),
            )
        };
    }

    /// OS が実行時に持つ IME ホットキー(`ImmGetHotKey`)を、既知の ID 範囲で列挙してログに出す。レジストリの値と突き合わせる。
    fn dump_hotkeys(lp: &str, tag: &str) {
        let ids: Vec<u32> = [0x10u32, 0x11, 0x12, 0x30, 0x31, 0x32, 0x70, 0x71, 0x72]
            .into_iter()
            .chain(0x100..0x120)
            .chain(0x200..0x210)
            .collect();
        for id in ids {
            let (mut m, mut v, mut h) = (0u32, 0u32, HKL::default());
            let ok = unsafe { ImmGetHotKey(id, &raw mut m, &raw mut v, &raw mut h) };
            if ok != 0 {
                log(
                    lp,
                    &format!(
                        "HK {tag} id=0x{id:03X} mods=0x{m:08X} vk=0x{v:02X} hkl=0x{:08X}",
                        h.0 as usize
                    ),
                );
            }
        }
    }

    // ── ADR-223 段階 0(窓の種類を増やした測定)────────────────────────────────

    fn class_of(hwnd: HWND) -> String {
        let mut buf = [0u16; 256];
        let n = unsafe { GetClassNameW(hwnd, &mut buf) };
        usize::try_from(n)
            .ok()
            .filter(|n| *n > 0)
            .map_or_else(String::new, |n| String::from_utf16_lossy(&buf[..n]))
    }

    fn title_of(hwnd: HWND) -> String {
        let mut buf = [0u16; 256];
        let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
        usize::try_from(n)
            .ok()
            .filter(|n| *n > 0)
            .map_or_else(String::new, |n| String::from_utf16_lossy(&buf[..n]))
    }

    unsafe extern "system" fn enum_cb(hwnd: HWND, lp: LPARAM) -> BOOL {
        let v = unsafe { &mut *(lp.0 as *mut Vec<isize>) };
        v.push(hwnd.0 as isize);
        BOOL(1)
    }

    fn top_windows() -> Vec<HWND> {
        let mut v: Vec<isize> = Vec::new();
        let _ = unsafe { EnumWindows(Some(enum_cb), LPARAM(&raw mut v as isize)) };
        v.into_iter()
            .map(|h| HWND(h as *mut core::ffi::c_void))
            .collect()
    }

    /// OS が持つ「実際の入力先」の言語(真値)。`GetGUIThreadInfo` のフォーカス窓(無ければアクティブ窓)のスレッドと、前面窓のスレッドの両方を読む。
    struct FocusInfo {
        fg_class: String,
        fg_tid: u32,
        fg_lang: u16,
        focus_class: String,
        focus_tid: u32,
        focus_lang: u16,
    }

    fn lang_of_tid(tid: u32) -> u16 {
        if tid == 0 {
            return 0;
        }
        let hkl = unsafe { GetKeyboardLayout(tid) };
        (hkl.0 as usize & 0xFFFF) as u16
    }

    fn focus_info() -> FocusInfo {
        let fg = unsafe { GetForegroundWindow() };
        let fg_tid = unsafe { GetWindowThreadProcessId(fg, None) };
        let mut gti = GUITHREADINFO {
            cbSize: u32::try_from(size_of::<GUITHREADINFO>()).unwrap_or(0),
            ..Default::default()
        };
        let ok = unsafe { GetGUIThreadInfo(0, &raw mut gti) }.is_ok();
        let focus = if ok && !gti.hwndFocus.0.is_null() {
            gti.hwndFocus
        } else if ok {
            gti.hwndActive
        } else {
            HWND::default()
        };
        let focus_tid = if focus.0.is_null() {
            0
        } else {
            unsafe { GetWindowThreadProcessId(focus, None) }
        };
        FocusInfo {
            fg_class: class_of(fg),
            fg_tid,
            fg_lang: lang_of_tid(fg_tid),
            focus_class: if focus.0.is_null() {
                String::new()
            } else {
                class_of(focus)
            },
            focus_tid,
            focus_lang: lang_of_tid(focus_tid),
        }
    }

    fn force_fg(hwnd: HWND) {
        let fg = unsafe { GetForegroundWindow() };
        let fg_tid = unsafe { GetWindowThreadProcessId(fg, None) };
        let me = unsafe { GetCurrentThreadId() };
        let attached =
            fg_tid != 0 && fg_tid != me && unsafe { AttachThreadInput(me, fg_tid, true) }.as_bool();
        let _ = unsafe { ShowWindow(hwnd, SW_SHOW) };
        let _ = unsafe { SetForegroundWindow(hwnd) };
        if attached {
            let _ = unsafe { AttachThreadInput(me, fg_tid, false) };
        }
    }

    /// 対象アプリを起動して、前面にした窓を返す。`console`(conhost の ConsoleWindowClass)・`uwp`(設定アプリ = ApplicationFrameWindow)・`chrome`。
    fn spawn_target(kind: &str, lp: &str) -> Option<HWND> {
        let before: Vec<isize> = top_windows().into_iter().map(|h| h.0 as isize).collect();
        match kind {
            "console" => {
                let _ = std::process::Command::new("conhost.exe")
                    .args(["cmd.exe", "/k", "title LSPROBE_CONSOLE"])
                    .spawn();
            }
            "uwp" => {
                let _ = std::process::Command::new("cmd.exe")
                    .args(["/c", "start", "", "ms-settings:"])
                    .spawn();
            }
            "chrome" => {
                let dir = std::env::temp_dir().join("lsprobe-chrome");
                let exe = [
                    "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
                    "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
                ]
                .into_iter()
                .find(|p| std::path::Path::new(p).exists())
                .unwrap_or("chrome.exe");
                let _ = std::process::Command::new(exe)
                    .arg(format!("--user-data-dir={}", dir.display()))
                    .args([
                        "--no-first-run",
                        "--no-default-browser-check",
                        "about:blank",
                    ])
                    .spawn();
            }
            _ => return None,
        }
        let want_class = match kind {
            "console" => "ConsoleWindowClass",
            "uwp" => "ApplicationFrameWindow",
            _ => "Chrome_WidgetWin_1",
        };
        for _ in 0..60 {
            pump(Duration::from_millis(250));
            for h in top_windows() {
                if before.contains(&(h.0 as isize)) || !unsafe { IsWindowVisible(h) }.as_bool() {
                    continue;
                }
                let class = class_of(h);
                let title = title_of(h);
                let hit =
                    class == want_class && (kind != "console" || title.contains("LSPROBE_CONSOLE"));
                if hit {
                    log(
                        lp,
                        &format!(
                            "[ls] target={kind} hwnd=0x{:X} class={class} title={title:?}",
                            h.0 as usize
                        ),
                    );
                    return Some(h);
                }
            }
        }
        log(lp, &format!("[ls] ABORT target={kind} の窓が見つからない"));
        None
    }

    /// `--stage0-ext=<console|uwp|chrome>`: 対象アプリを前面にして、切替前後の打鍵を送り、OS が持つ真値(フォーカススレッドと前面スレッドの言語)を記録する。
    fn run_ext(lp: &str, kind: &str, trials: usize, ja: HKL, ru: HKL) {
        let Some(target) = spawn_target(kind, lp) else {
            return;
        };
        pump(Duration::from_millis(2500));
        let tap = |extra: usize| {
            key_ex(0x41, false, extra);
            key_ex(0x41, true, extra);
        };
        for method in ["request", "altshift"] {
            for n in 0..trials {
                force_fg(target);
                pump(Duration::from_millis(600));
                request(ja);
                let t0 = Instant::now();
                while focus_info().focus_lang != LANG_JA
                    && t0.elapsed() < Duration::from_millis(2500)
                {
                    pump(Duration::from_millis(20));
                }
                pump(Duration::from_millis(400));
                let before = focus_info();
                tap(TEST_MARKER);
                let t_ctrl = epoch_ms();
                pump(Duration::from_millis(300));
                let t_inject = epoch_ms();
                match method {
                    "altshift" => chord(&[0xA4, 0xA0], None),
                    _ => request(ru),
                }
                pump(Duration::from_millis(500));
                let after = focus_info();
                tap(TEST_MARKER);
                let t_key1 = epoch_ms();
                pump(Duration::from_millis(300));
                tap(0);
                let t_key2 = epoch_ms();
                pump(Duration::from_millis(300));
                let rec = serde_json::json!({
                    "target": kind, "method": method, "n": n,
                    "t_ctrl_epoch_ms": t_ctrl, "t_inject_epoch_ms": t_inject, "t_key1_epoch_ms": t_key1, "t_key2_epoch_ms": t_key2,
                    "before": {"fg_class": before.fg_class, "fg_tid": before.fg_tid, "fg_lang": format!("0x{:04X}", before.fg_lang),
                               "focus_class": before.focus_class, "focus_tid": before.focus_tid, "focus_lang": format!("0x{:04X}", before.focus_lang)},
                    "after": {"fg_class": after.fg_class, "fg_tid": after.fg_tid, "fg_lang": format!("0x{:04X}", after.fg_lang),
                              "focus_class": after.focus_class, "focus_tid": after.focus_tid, "focus_lang": format!("0x{:04X}", after.focus_lang)},
                });
                log(lp, &format!("LS0X {rec}"));
            }
        }
        log(lp, "[ls] done");
    }

    pub fn run() -> anyhow::Result<()> {
        let args: Vec<String> = std::env::args().collect();
        let arg = |k: &str| {
            args.iter()
                .find_map(|a| a.strip_prefix(k))
                .map(str::to_string)
        };
        let trials: usize = arg("--trials=").and_then(|v| v.parse().ok()).unwrap_or(6);
        let methods: Vec<String> = arg("--methods=")
            .unwrap_or_else(|| "altshift,winspace,hk3,request,hk3_focus,request_focus".into())
            .split(',')
            .map(str::to_string)
            .collect();
        let lp = arg("--log=").unwrap_or_else(|| "lang_switch_probe.log".into());
        // `--listen=<秒>`: 前面にならない背景プロセスとして通知だけを受ける(awase と同じ条件)。通知は `EV` 行でログへ出す。
        let listen_secs: Option<u64> = arg("--listen=").and_then(|v| v.parse().ok());
        let listen = listen_secs.is_some();
        let observe_ms: u64 = arg("--observe-ms=")
            .and_then(|v| v.parse().ok())
            .unwrap_or(10_000);
        // 前面スレッドの言語が ru になった最初の時刻(epoch ms)を 5ms 周期で記録する(キー注入中に切り替わっても拾えるように別スレッド)。
        std::thread::spawn(|| loop {
            if fg_lang() == LANG_RU && LAYOUT_RU_AT.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                LAYOUT_RU_AT.store(epoch_ms(), std::sync::atomic::Ordering::SeqCst);
            }
            std::thread::sleep(Duration::from_millis(5));
        });

        let class = to_wide("lang_switch_probe_window");
        let hinst = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }
            .unwrap_or_default()
            .into();
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinst,
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        anyhow::ensure!(
            unsafe { RegisterClassW(&raw const wc) } != 0,
            "RegisterClassW"
        );
        let title = to_wide("lang switch probe");
        let parent = unsafe {
            CreateWindowExW(
                Default::default(),
                PCWSTR(class.as_ptr()),
                PCWSTR(title.as_ptr()),
                if listen {
                    WS_OVERLAPPEDWINDOW
                } else {
                    WS_OVERLAPPEDWINDOW | WS_VISIBLE
                },
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                600,
                300,
                None,
                None,
                Some(hinst),
                None,
            )
        }?;
        let edit_class = to_wide("EDIT");
        let _edit = unsafe {
            CreateWindowExW(
                Default::default(),
                PCWSTR(edit_class.as_ptr()),
                PCWSTR::null(),
                windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(
                    0x4000_0000 | WS_VISIBLE.0 | 0x0080_0000 | 0x0004 | 0x0040,
                ),
                0,
                0,
                580,
                260,
                Some(parent),
                None,
                Some(hinst),
                None,
            )
        }?;
        let title2 = to_wide("lang switch probe 2");
        let parent2 = unsafe {
            CreateWindowExW(
                Default::default(),
                PCWSTR(class.as_ptr()),
                PCWSTR(title2.as_ptr()),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                400,
                200,
                None,
                None,
                Some(hinst),
                None,
            )
        }?;
        if !listen {
            let _ = unsafe { ShowWindow(parent, SW_SHOW) };
            let _ = unsafe { SetForegroundWindow(parent) };
        }
        pump(Duration::from_millis(800));

        // 購読(イベント)候補: ①シェルフック HSHELL_LANGUAGE ②TSF の ITfActiveLanguageProfileNotifySink(ThreadMgr)
        // ③TSF の ITfLanguageProfileNotifySink(InputProcessorProfiles)。どれが言語切替で通知を出すかを測る。
        let shell_msg = unsafe { RegisterWindowMessageW(w!("SHELLHOOK")) };
        SHELL_MSG.store(shell_msg, std::sync::atomic::Ordering::SeqCst);
        let shell_ok = unsafe { RegisterShellHookWindow(parent) }.as_bool();
        log(
            &lp,
            &format!("[ls] RegisterShellHookWindow={shell_ok} msg=0x{shell_msg:X}"),
        );
        let _com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        let mut _keep: Vec<(ITfSource, u32, windows::core::IUnknown)> = Vec::new();
        let thread_mgr: Option<ITfThreadMgr> =
            unsafe { CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER) }.ok();
        if let Some(tm) = &thread_mgr {
            let _ = unsafe { tm.Activate() };
            if let Ok(src) = tm.cast::<ITfSource>() {
                let sink: windows::core::IUnknown = super::sinks::ActSink.into();
                match unsafe {
                    src.AdviseSink(
                        &<ITfActiveLanguageProfileNotifySink as windows::core::Interface>::IID,
                        &sink,
                    )
                } {
                    Ok(c) => {
                        log(
                            &lp,
                            &format!(
                                "[ls] Advise ITfActiveLanguageProfileNotifySink ok cookie={c}"
                            ),
                        );
                        _keep.push((src, c, sink));
                    }
                    Err(e) => log(
                        &lp,
                        &format!("[ls] Advise ITfActiveLanguageProfileNotifySink 失敗: {e:?}"),
                    ),
                }
            }
        }
        let profiles: Option<windows::core::IUnknown> = unsafe {
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)
        }
        .ok();
        if let Some(pr) = &profiles {
            if let Ok(src) = pr.cast::<ITfSource>() {
                let sink: windows::core::IUnknown = super::sinks::LangSink.into();
                match unsafe {
                    src.AdviseSink(
                        &<ITfLanguageProfileNotifySink as windows::core::Interface>::IID,
                        &sink,
                    )
                } {
                    Ok(c) => {
                        log(
                            &lp,
                            &format!("[ls] Advise ITfLanguageProfileNotifySink ok cookie={c}"),
                        );
                        _keep.push((src, c, sink));
                    }
                    Err(e) => log(
                        &lp,
                        &format!("[ls] Advise ITfLanguageProfileNotifySink 失敗: {e:?}"),
                    ),
                }
            }
        }

        if let Some(secs) = listen_secs {
            log(&lp, &format!("[ls] listener started secs={secs}"));
            let end = Instant::now() + Duration::from_secs(secs);
            while Instant::now() < end {
                pump(Duration::from_millis(100));
                let drained: Vec<(u64, String)> = super::sinks::EVENTS
                    .lock()
                    .map(|mut v| std::mem::take(&mut *v))
                    .unwrap_or_default();
                for (t, e) in drained {
                    log(&lp, &format!("EV {t} {e}"));
                }
            }
            return Ok(());
        }
        let list = hkls();
        if args.iter().any(|a| a == "--dump-hotkeys") {
            dump_hotkeys(&lp, "before");
        }
        log(
            &lp,
            &format!(
                "[ls] hkls={:?} fg_lang=0x{:04X}",
                list.iter()
                    .map(|h| format!("{:08X}", h.0 as usize))
                    .collect::<Vec<_>>(),
                fg_lang()
            ),
        );
        let (Some(ja), Some(ru)) = (hkl_of(&list, LANG_JA), hkl_of(&list, LANG_RU)) else {
            log(&lp, "[ls] ABORT ja または ru の HKL が無い");
            return Ok(());
        };
        // 入力言語のホットキー(ロシア語直接切替 = Alt+Shift+3、日本語 = Alt+Shift+1)。MOD_ALT=1, MOD_SHIFT=4, MOD_LEFT=0x4000。
        let r1 = unsafe { ImmSetHotKey(0x100, 0x4005, 0x33, ru) };
        let r2 = unsafe { ImmSetHotKey(0x101, 0x4005, 0x31, ja) };
        log(&lp, &format!("[ls] ImmSetHotKey ru={r1} ja={r2}"));
        if let Some(kind) = arg("--stage0-ext=") {
            run_ext(&lp, &kind, trials, ja, ru);
            return Ok(());
        }
        if args.iter().any(|a| a == "--dump-hotkeys") {
            dump_hotkeys(&lp, "after-set");
        }

        let set_ja = || {
            request(ja);
            let t0 = Instant::now();
            while fg_lang() != LANG_JA && t0.elapsed() < Duration::from_millis(2500) {
                pump(Duration::from_millis(20));
            }
            pump(Duration::from_millis(300));
            // IME ON(awase の Engine を活性にする)。
            key(0x16, false);
            key(0x16, true);
            pump(Duration::from_millis(2500));
        };

        // ADR-223 段階 0: 打鍵時の入力言語の記録(`[lang-check:key]`)を、プローブ側の真値と突き合わせるための試行。
        // 各試行: ja に戻す → 対照の文字キー(ja のまま = 偽陽性の検出)→ 切替 → 切替後の最初の文字キー(マーカーあり=物理扱い)
        // → 2 打鍵目(マーカーなし=外部注入)。`focus_key` は切替の直後にフォーカスを別窓へ移して即座に打鍵する(フォーカス直後 0〜100ms)。
        if args.iter().any(|a| a == "--stage0") {
            let tap = |extra: usize| {
                key_ex(0x41, false, extra);
                key_ex(0x41, true, extra);
            };
            for m in &methods {
                for n in 0..trials {
                    let _ = unsafe { SetForegroundWindow(parent) };
                    set_ja();
                    tap(TEST_MARKER);
                    let t_ctrl = epoch_ms();
                    pump(Duration::from_millis(300));
                    LAYOUT_RU_AT.store(0, std::sync::atomic::Ordering::SeqCst);
                    let t_inject = epoch_ms();
                    match m.as_str() {
                        "altshift" => chord(&[0xA4, 0xA0], None),
                        "winspace" => chord(&[0x5B], Some(0x20)),
                        "hk3" => chord(&[0xA4, 0xA0], Some(0x33)),
                        "request" | "focus_key" => request(ru),
                        _ => {}
                    }
                    if m == "focus_key" {
                        pump(Duration::from_millis(150));
                        let _ = unsafe { SetForegroundWindow(parent2) };
                        pump(Duration::from_millis(20));
                    } else {
                        pump(Duration::from_millis(500));
                    }
                    let t_switch = LAYOUT_RU_AT.load(std::sync::atomic::Ordering::SeqCst);
                    tap(TEST_MARKER);
                    let t_key1 = epoch_ms();
                    pump(Duration::from_millis(300));
                    tap(0);
                    let t_key2 = epoch_ms();
                    pump(Duration::from_millis(300));
                    let rec = serde_json::json!({
                        "method": m, "n": n, "t_ctrl_epoch_ms": t_ctrl, "t_inject_epoch_ms": t_inject,
                        "t_switch_epoch_ms": t_switch, "t_key1_epoch_ms": t_key1, "t_key2_epoch_ms": t_key2,
                        "lang_end": format!("0x{:04X}", fg_lang()),
                    });
                    log(&lp, &format!("LS0 {rec}"));
                }
            }
            log(&lp, "[ls] done");
            return Ok(());
        }

        for m in &methods {
            for n in 0..trials {
                let _ = unsafe { SetForegroundWindow(parent) };
                set_ja();
                let before = fg_lang();
                LAYOUT_RU_AT.store(0, std::sync::atomic::Ordering::SeqCst);
                if let Ok(mut v) = super::sinks::EVENTS.lock() {
                    v.clear();
                }
                let t_inject = epoch_ms();
                let mut t_focus: Option<u64> = None;
                match m.as_str() {
                    "altshift" => chord(&[0xA4, 0xA0], None),
                    "ctrlshift" => chord(&[0xA2, 0xA0], None),
                    "winspace" => chord(&[0x5B], Some(0x20)),
                    "hk3" | "hk3_focus" => chord(&[0xA4, 0xA0], Some(0x33)),
                    "request" | "request_focus" => request(ru),
                    _ => {}
                }
                let t0 = Instant::now();
                let mut focus_done = false;
                while t0.elapsed() < Duration::from_millis(observe_ms) {
                    // `*_focus`: 切替の 1.5 秒後にフォーカスを別窓へ移す(フォーカス変更が awase の再判定の契機になるかを見る)。
                    if m.ends_with("_focus")
                        && !focus_done
                        && t0.elapsed() >= Duration::from_millis(1500)
                    {
                        let _ = unsafe { SetForegroundWindow(parent2) };
                        t_focus = Some(epoch_ms());
                        focus_done = true;
                    }
                    pump(Duration::from_millis(10));
                }
                let ru_at = LAYOUT_RU_AT.load(std::sync::atomic::Ordering::SeqCst);
                let layout_ms: Option<u64> = if ru_at == 0 {
                    None
                } else {
                    Some(ru_at.saturating_sub(t_inject))
                };
                let events: Vec<(i64, String)> = super::sinks::EVENTS
                    .lock()
                    .map(|v| {
                        v.iter()
                            .map(|(t, e)| {
                                (
                                    i64::try_from(*t).unwrap_or(0)
                                        - i64::try_from(t_inject).unwrap_or(0),
                                    e.clone(),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let rec = serde_json::json!({
                    "method": m, "n": n, "lang_before": format!("0x{before:04X}"),
                    "t_inject_epoch_ms": t_inject, "t_focus_epoch_ms": t_focus, "layout_ms_after_inject": layout_ms,
                    "lang_end": format!("0x{:04X}", fg_lang()), "events": events,
                });
                log(&lp, &format!("LS {rec}"));
            }
        }
        log(&lp, "[ls] done");
        Ok(())
    }
}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    p::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("windows only");
}
