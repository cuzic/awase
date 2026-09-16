//! ADR-176 v8のopus round6レビューで残った、紙の設計だけでは決着しない
//! 論点をまとめて実機検証するための決着実験v2。
//!
//! ## 検証したいこと（round6の指摘に対応）
//!
//! 1. **M2**: 較正probe（`ImmGetDefaultIMEWnd`+`WM_IME_CONTROL`の
//!    クロスプロセス呼び出し）を、`WH_KEYBOARD_LL`フックを別スレッドで
//!    持つプロセスから行っても、フックの応答性（コールバックの
//!    取りこぼし）に影響が出ないか。
//!    （コードレビューで`crates/awase-windows/src/hook.rs::install_hook`
//!    を確認した結果、awase本体はフックを"awase-hook"という**専用の
//!    独立スレッド**で動かしており、メインのランタイムスレッドとは
//!    別である——round6 M2の「同一スレッド」という前提はこの時点で
//!    誤りだったと判明している。本スパイクはそれでも「フックスレッドが
//!    別にあっても、メインスレッド側の同期SendMessageTimeoutWで何か
//!    問題が起きないか」を全体として裏取りする）。
//! 2. **M3**: 実際の物理キー押下（無変換/変換）からGJIがIME状態を
//!    実際に変えるまでの実測レイテンシ（ポーリング間隔の実測根拠）。
//! 3. **B2/send_health懸念**: `SendMessageTimeoutW`の`elapsed_ms`が
//!    `send_health::SLOW_THRESHOLD_MS`（100ms）を超える頻度。
//! 4. **m5**: `disable_apps`が実際に`awase-settings.exe`へ適用された
//!    状態で、決着実験（v1、`spike_egui_ime_control_probe.rs`）と同じ
//!    結果が再現するか（v1はこの条件を記録していなかった）。
//!
//! ## 実行方法（Windows実機のみ）
//!
//! 事前に、実機のawase.exe設定（`config.toml`の
//! `app_overrides.disable_apps`）に`awase-settings.exe`を追加し、
//! awase.exeを再起動しておくこと（decision1が要求する条件の再現）。
//!
//! ```powershell
//! cargo run -p awase-windows --example spike_calibration_decisive_v2 --release
//! ```
//!
//! 起動すると非表示の`WH_KEYBOARD_LL`フックスレッドと、可視の
//! メインウィンドウ（ログ表示欄のみ）を1つ作る。`awase-settings.exe`
//! （`--bug-report`等）にフォーカスした状態で、無変換/変換キーで
//! 実際にGJIのIME ON/OFFを切り替えること。
//!
//! ### 見るべきポイント
//!
//! - `[LATENCY]`行: 物理キー押下から観測されたIME状態変化までの
//!   実測ms（M3の実測根拠）。
//! - `[SLOW]`行: `elapsed_ms`が80ms/100msを超えた回（B2/send_health
//!   懸念の裏取り）。
//! - `[HOOK-HEARTBEAT]`行: フックスレッドが生きている証跡（1秒毎の
//!   カウント、極端に間隔が空けばフックスレッドの遅延を疑う）。
//! - 較正中もタイピングが正常に打てる（フック取りこぼしが無い）ことを
//!   目視で確認する。

#![allow(unsafe_code)]

#[cfg(windows)]
mod spike {
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
    use std::sync::OnceLock;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::{
        GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::Input::Ime::ImmGetDefaultIMEWnd;
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClassNameW,
        GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, KillTimer, PostQuitMessage,
        RegisterClassW, SendMessageTimeoutW, SetTimer, SetWindowsHookExW, TranslateMessage,
        UnhookWindowsHookEx, CW_USEDEFAULT, HHOOK, KBDLLHOOKSTRUCT, MSG, SMTO_ABORTIFHUNG,
        WH_KEYBOARD_LL, WM_DESTROY, WM_KEYDOWN, WM_TIMER, WNDCLASSW, WS_OVERLAPPEDWINDOW,
        WS_VISIBLE,
    };

    const WM_IME_CONTROL: u32 = 0x0283;
    const IMC_GETOPENSTATUS: usize = 0x0005;
    const LLKHF_INJECTED: u32 = 0x0000_0010;
    const VK_CONVERT: u16 = 0x1C; // 変換
    const VK_NONCONVERT: u16 = 0x1D; // 無変換

    const TARGET_PROCESS: &str = "awase-settings.exe";
    const WINDOW_CLASS_NAME: &str = "spike_calibration_decisive_v2_window";
    const TIMER_ID: usize = 1;
    /// M3のレイテンシ実測のため、決着実験v1(100ms)よりさらに細かく取る。
    const POLL_INTERVAL_MS: u32 = 20;
    const SEND_IME_CONTROL_TIMEOUT_MS: u32 = 50;
    /// `send_health::SLOW_THRESHOLD_MS`と同じ値（B2裏取り用）。
    const SLOW_THRESHOLD_MS: u128 = 100;

    static START: OnceLock<std::time::Instant> = OnceLock::new();
    /// 直近の無変換/変換キー物理押下からの経過ms（0=保留中の押下無し）。
    /// フックスレッドがセットし、メインスレッドが遷移検出時に読んで消費する。
    static LAST_MODE_KEY_DOWN_MS: AtomicU64 = AtomicU64::new(0);
    static HOOK_TICK_COUNT: AtomicU32 = AtomicU32::new(0);
    static HOOK_HANDLE_RAW: AtomicU64 = AtomicU64::new(0);

    fn now_ms() -> u64 {
        u64::try_from(START.get().expect("START not set").elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn log(msg: &str) {
        use std::io::Write as _;
        let t = now_ms();
        println!("[{t:>8}ms] {msg}");
        let _ = std::io::stdout().flush();
    }

    fn process_name_of(hwnd: HWND) -> String {
        let mut pid: u32 = 0;
        // SAFETY: hwnd は呼出元が GetForegroundWindow から得た値。
        unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
        if pid == 0 {
            return "?".to_string();
        }
        // SAFETY: pid は直前に取得した有効な PID。
        let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
        else {
            return "?".to_string();
        };
        let mut buf = [0u16; 260];
        let mut len = u32::try_from(buf.len()).unwrap_or(0);
        // SAFETY: handle は有効なプロセスハンドル、buf はスタック上有効。
        let ok = unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(buf.as_mut_ptr()),
                &raw mut len,
            )
        };
        // SAFETY: handle はここでのみ閉じる。
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(handle) };
        if ok.is_err() {
            return "?".to_string();
        }
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        full.rsplit('\\').next().unwrap_or(&full).to_string()
    }

    /// `crates/awase-windows/src/imm.rs::get_ime_wnd`+`probe_ime_control`と同型。
    fn probe_open_status(hwnd: HWND) -> (Option<bool>, u128) {
        // SAFETY: hwnd は GetForegroundWindow が返した値、クロスプロセスで安全。
        let ime_wnd = unsafe { ImmGetDefaultIMEWnd(hwnd) };
        if ime_wnd.0.is_null() {
            return (None, 0);
        }
        let mut result: usize = 0;
        let start = std::time::Instant::now();
        // SAFETY: ime_wnd は直前に取得した有効な IME ウィンドウ。
        let ok = unsafe {
            SendMessageTimeoutW(
                ime_wnd,
                WM_IME_CONTROL,
                WPARAM(IMC_GETOPENSTATUS),
                LPARAM(0),
                SMTO_ABORTIFHUNG,
                SEND_IME_CONTROL_TIMEOUT_MS,
                Some(&raw mut result),
            )
        };
        let elapsed_ms = start.elapsed().as_millis();
        if ok.0 == 0 {
            (None, elapsed_ms)
        } else {
            (Some(result != 0), elapsed_ms)
        }
    }

    // ─── WH_KEYBOARD_LL フックスレッド（hook.rs::install_hook を簡略化して再現） ───

    unsafe extern "system" fn hook_callback(ncode: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        HOOK_TICK_COUNT.fetch_add(1, Ordering::Relaxed);
        let hook_handle = HHOOK(HOOK_HANDLE_RAW.load(Ordering::Relaxed) as *mut _);
        if ncode < 0 {
            // SAFETY: OS から渡されたそのままの引数を CallNextHookEx へ転送する。
            return unsafe { CallNextHookEx(Some(hook_handle), ncode, wparam, lparam) };
        }
        // SAFETY: lparam は WH_KEYBOARD_LL コールバックで OS が渡す
        //         KBDLLHOOKSTRUCT へのポインタ。
        let kb = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let is_keydown = u32::try_from(wparam.0) == Ok(WM_KEYDOWN);
        let is_injected = (kb.flags.0 & LLKHF_INJECTED) != 0;
        let vk = u16::try_from(kb.vkCode).unwrap_or(0);

        if is_keydown && !is_injected && (vk == VK_CONVERT || vk == VK_NONCONVERT) {
            let t = now_ms();
            LAST_MODE_KEY_DOWN_MS.store(t, Ordering::Relaxed);
            log(&format!(
                "[KEYDOWN] vk=0x{vk:02X} ({}) physical (non-injected)",
                if vk == VK_CONVERT {
                    "変換"
                } else {
                    "無変換"
                }
            ));
        }

        // SAFETY: OS から渡されたそのままの引数を CallNextHookEx へ転送する。
        unsafe { CallNextHookEx(Some(hook_handle), ncode, wparam, lparam) }
    }

    fn install_hook_thread() -> std::thread::JoinHandle<()> {
        std::thread::Builder::new()
            .name("spike-hook".into())
            .spawn(|| {
                // SAFETY: プロセス内で1度だけ、専用スレッド上で呼ぶ。
                let hook =
                    unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_callback), None, 0) };
                let hook = match hook {
                    Ok(h) => h,
                    Err(e) => {
                        log(&format!("[FATAL] SetWindowsHookExW failed: {e}"));
                        return;
                    }
                };
                HOOK_HANDLE_RAW.store(hook.0 as u64, Ordering::Relaxed);
                let tid = unsafe { GetCurrentThreadId() };
                log(&format!("[hook-thread] installed, tid={tid}"));

                let mut msg = MSG::default();
                loop {
                    // SAFETY: msg はスタック上の有効な MSG バッファ。
                    let ret = unsafe { GetMessageW(&raw mut msg, None, 0, 0) };
                    if ret.0 <= 0 {
                        break;
                    }
                    // SAFETY: msg は GetMessageW が充填した有効な値。
                    unsafe { DispatchMessageW(&raw const msg) };
                }
                // SAFETY: hook は上で取得した有効なハンドル、このスレッドでのみ解除。
                let _ = unsafe { UnhookWindowsHookEx(hook) };
            })
            .expect("failed to spawn spike-hook thread")
    }

    // ─── メインウィンドウ（可視、ログ表示欄のみ。ポーリングタイマーを保持） ───

    thread_local! {
        static LAST_OPEN: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
        static LAST_FOCUS_HWND: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
        static HEARTBEAT_TICKS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    fn on_timer() {
        let hwnd = unsafe { GetForegroundWindow() };
        let hwnd_raw = hwnd.0 as isize;
        let prev_focus = LAST_FOCUS_HWND.get();
        if hwnd_raw != prev_focus {
            LAST_FOCUS_HWND.set(hwnd_raw);
            let process = if hwnd.0.is_null() {
                "?".to_string()
            } else {
                process_name_of(hwnd)
            };
            let mut buf = [0u16; 256];
            // SAFETY: hwnd は GetForegroundWindow が返した値、buf はスタック上有効。
            let len = unsafe { GetClassNameW(hwnd, &mut buf) };
            let class = usize::try_from(len)
                .ok()
                .map_or_else(|| "?".to_string(), |n| String::from_utf16_lossy(&buf[..n]));
            log(&format!(
                "[FOCUS] hwnd=0x{hwnd_raw:X} class={class} process={process}"
            ));
        }

        let process = if hwnd.0.is_null() {
            String::new()
        } else {
            process_name_of(hwnd)
        };
        if !process.eq_ignore_ascii_case(TARGET_PROCESS) {
            return;
        }

        let (open, elapsed_ms) = probe_open_status(hwnd);
        if elapsed_ms > SLOW_THRESHOLD_MS {
            log(&format!(
                "[SLOW] elapsed_ms={elapsed_ms} > SLOW_THRESHOLD_MS(100) -- send_health懸念(B2)の実例"
            ));
        } else if elapsed_ms > 80 {
            log(&format!(
                "[SLOW-WARN] elapsed_ms={elapsed_ms} (80ms超、100ms未満)"
            ));
        }

        let prev = LAST_OPEN.get();
        if open != prev && open.is_some() {
            log(&format!(
                "[TRANSITION] open: {prev:?} -> {open:?} elapsed_ms={elapsed_ms}"
            ));
            let pending = LAST_MODE_KEY_DOWN_MS.swap(0, Ordering::Relaxed);
            if pending != 0 {
                let latency = now_ms().saturating_sub(pending);
                log(&format!(
                    "[LATENCY] 物理キー押下から観測までの実測: {latency}ms"
                ));
            } else {
                log("[LATENCY] 対応する物理キー押下が記録されていない遷移(手動UI操作等)");
            }
        }
        LAST_OPEN.set(open);

        HEARTBEAT_TICKS.set(HEARTBEAT_TICKS.get() + 1);
        if HEARTBEAT_TICKS.get().is_multiple_of(50) {
            // 50 tick * 20ms = 1秒毎
            let hook_ticks = HOOK_TICK_COUNT.load(Ordering::Relaxed);
            log(&format!(
                "[HOOK-HEARTBEAT] hook_thread cumulative key events={hook_ticks}"
            ));
        }
    }

    extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match msg {
            WM_TIMER => {
                on_timer();
                LRESULT(0)
            }
            WM_DESTROY => {
                // SAFETY: hwnd はこのウィンドウ自身、TIMER_ID は SetTimer と同一。
                let _ = unsafe { KillTimer(Some(hwnd), TIMER_ID) };
                // SAFETY: メッセージループを持つスレッドから呼ぶ限り常に安全。
                unsafe { PostQuitMessage(0) };
                LRESULT(0)
            }
            // SAFETY: DefWindowProcW はどんな引数でも安全に呼べる既定ハンドラ。
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub(super) fn run() -> windows::core::Result<()> {
        START.set(std::time::Instant::now()).ok();

        let _hook_thread = install_hook_thread();

        let class_name_wide = to_wide(WINDOW_CLASS_NAME);
        // SAFETY: プロセス起動直後、他スレッドがまだウィンドウを作っていない時点。
        let hinstance = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: PCWSTR(class_name_wide.as_ptr()),
            ..Default::default()
        };
        // SAFETY: wc はスタック上の有効な WNDCLASSW、呼び出しは起動時に1回のみ。
        let atom = unsafe { RegisterClassW(&raw const wc) };
        if atom == 0 {
            return Err(windows::core::Error::from_thread());
        }

        let title_wide = to_wide("ADR-176 v8 decisive spike v2");
        // SAFETY: 各 wide 文字列・hinstance はこのスコープで有効。
        let hwnd = unsafe {
            CreateWindowExW(
                windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
                PCWSTR(class_name_wide.as_ptr()),
                PCWSTR(title_wide.as_ptr()),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                480,
                200,
                None,
                None,
                Some(hinstance.into()),
                None,
            )
        }?;

        log("=== ADR-176 v8 decisive spike v2 起動 ===");
        log(&format!(
            "M2: WH_KEYBOARD_LLは専用スレッド(spike-hook)、ポーリングはこのメインスレッド。POLL_INTERVAL_MS={POLL_INTERVAL_MS}"
        ));
        log("awase-settings.exeにフォーカスし、無変換/変換キーで実際にGJIのIME ON/OFFを切り替えてください。");
        log("事前にconfig.tomlのdisable_appsにawase-settings.exeを入れてawase.exeを再起動しておくこと(m5対応)。");

        // SAFETY: hwnd は直前に作成した有効なウィンドウ。
        unsafe { SetTimer(Some(hwnd), TIMER_ID, POLL_INTERVAL_MS, None) };

        let mut msg = MSG::default();
        // SAFETY: msg はスタック上の有効な MSG バッファ。
        while unsafe { GetMessageW(&raw mut msg, None, 0, 0) }.as_bool() {
            let _ = unsafe { TranslateMessage(&raw const msg) };
            unsafe { DispatchMessageW(&raw const msg) };
        }
        Ok(())
    }
}

#[cfg(windows)]
fn main() {
    if let Err(e) = spike::run() {
        eprintln!("fatal: {e}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("このスパイクは Windows 専用です（cfg(windows) ガード）。");
}
