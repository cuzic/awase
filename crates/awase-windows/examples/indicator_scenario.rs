//! タスクバーの入力インジケーターの字形が、窓の切替・IMM の無い窓・フォーカススレッドと前面スレッドの違いで、どう動くかの測定シナリオ
//! (ADR-261 D0-1/D0-2 の近似/D0-10。使い捨て、awase 非依存)。並行して `ime_indicator_probe`(字形の時系列)が走るので、ログの UTC で突き合わせる。
//!
//! 使い方: `indicator_scenario --scen=switch|noimm|focusthread [--n=<繰り返し>] [--msime] [--log=<path>]`
//! ログ: `SW …`(前面化の時刻)/ `PRESS vk=…` / `TRUTH <名> open=.. conv=..`(窓のスレッドの IME の実状態、IMC で読む)/ `FOCUS fg_tid=.. focus_tid=..` / `=== 完了 ===`。

#[cfg(windows)]
#[allow(dead_code, unused_imports, clippy::all, clippy::pedantic, clippy::nursery, unsafe_code)]
#[path = "text_store_probe.rs"]
mod tsp;

#[cfg(windows)]
mod app {
    use std::io::Write as _;
    use std::sync::mpsc;
    use std::time::Duration;

    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::Ime::{ImmAssociateContextEx, ImmDisableIME, ImmGetDefaultIMEWnd};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        MapVirtualKeyW, SendInput, SetFocus, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
        KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, VIRTUAL_KEY,
    };
    use windows::Win32::UI::TextServices::{
        CLSID_TF_InputProcessorProfiles, CLSID_TF_ThreadMgr, ITfContext, ITfInputProcessorProfileMgr, ITfThreadMgr,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, CreateWindowExW, DefWindowProcW, DispatchMessageW,
        GetForegroundWindow, GetGUIThreadInfo, GetMessageW, GetWindowThreadProcessId, SendMessageW, WM_GETTEXT, WM_SETTEXT,
        RegisterClassW, SendMessageTimeoutW, SetForegroundWindow, TranslateMessage, GUITHREADINFO,
        MSG, SMTO_ABORTIFHUNG, WINDOW_EX_STYLE, WINDOW_STYLE, WM_IME_CONTROL, WNDCLASSW,
        WS_BORDER, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };

    fn arg(prefix: &str) -> Option<String> {
        std::env::args().find_map(|a| a.strip_prefix(prefix).map(str::to_owned))
    }

    fn utc_stamp() -> String {
        let d = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = d.as_secs() % 86_400;
        format!("{:02}:{:02}:{:02}.{:03}Z", secs / 3600, (secs / 60) % 60, secs % 60, d.subsec_millis())
    }

    static LOG_PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    fn log(s: &str) {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(LOG_PATH.get().map_or("indicator_scenario.log", String::as_str))
        {
            let _ = writeln!(f, "[{}] {s}", utc_stamp());
        }
    }
    fn sleep_ms(ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }

    // ---------------------------------------------------------------- プロファイル有効化
    fn activate_profile() {
        let (clsid, profile) = if std::env::args().any(|a| a == "--msime") {
            (
                windows::core::GUID::from_u128(0x03B5835F_F03C_411B_9CE2_AA23E1171E36),
                windows::core::GUID::from_u128(0xA76C93D9_5523_4E90_AAFA_4DB112F9AC76),
            )
        } else {
            (
                windows::core::GUID::from_u128(0xD5A86FD5_5308_47EA_AD16_9C4EB160EC3C),
                windows::core::GUID::from_u128(0x773EB24E_CA1D_4B1B_B420_FA985BB0B80D),
            )
        };
        // SAFETY: 起動直後の単発 COM 初期化とプロファイル有効化。
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let mgr: windows::core::Result<ITfInputProcessorProfileMgr> =
                CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER);
            match mgr {
                Ok(m) => {
                    let r = m.ActivateProfile(
                        1,
                        0x0411,
                        &clsid,
                        &profile,
                        windows::Win32::UI::Input::KeyboardAndMouse::HKL(std::ptr::null_mut()),
                        0x1 | 0x2000_0000,
                    );
                    log(&format!("[init] IMEプロファイルをアクティブ化: {r:?}"));
                }
                Err(e) => log(&format!("[init] ITfInputProcessorProfileMgr取得失敗: {e}")),
            }
        }
        sleep_ms(1500);
    }

    // ---------------------------------------------------------------- 窓(スレッドごと)
    unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        // SAFETY: 既定処理へ渡すだけ。
        unsafe { DefWindowProcW(h, m, wp, lp) }
    }

    fn register_class() {
        // SAFETY: 窓クラスの登録(重複登録は失敗するだけで無害)。
        unsafe {
            let hi = GetModuleHandleW(None).unwrap_or_default();
            let wc = WNDCLASWrap::make(hi.into());
            let _ = RegisterClassW(&wc.0);
        }
    }
    struct WNDCLASWrap(WNDCLASSW);
    impl WNDCLASWrap {
        fn make(hi: windows::Win32::Foundation::HINSTANCE) -> Self {
            Self(WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hi,
                lpszClassName: w!("ScenWnd"),
                ..Default::default()
            })
        }
    }

    fn pump() {
        let mut msg = MSG::default();
        // SAFETY: 標準のメッセージループ。
        unsafe {
            while GetMessageW(&raw mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&raw const msg);
                DispatchMessageW(&raw const msg);
            }
        }
    }

    struct Win {
        name: &'static str,
        top: HWND,
        edit: HWND,
        tid: u32,
        /// TSF ストアの窓なら、ストアのログ(最後の `LockReleased` がストアの現在のテキスト)。
        store: Option<crate::tsp::store::Log>,
    }

    /// 新しいスレッドに、最上位の窓 + EDIT を作る。`disable_ime`=スレッドで ImmDisableIME(0)、`assoc_null`=EDIT の IME コンテキストを外す。
    fn spawn_window(name: &'static str, x: i32, disable_ime: bool, assoc_null: bool) -> Win {
        spawn_window_cls(name, x, disable_ime, assoc_null, false)
    }

    /// `rich`=EDIT の代わりに TSF 対応の RichEdit(`RICHEDIT50W`)を作る。
    fn spawn_window_cls(name: &'static str, x: i32, disable_ime: bool, assoc_null: bool, rich: bool) -> Win {
        let (tx, rx) = mpsc::channel::<(isize, isize, u32)>();
        std::thread::spawn(move || {
            register_class();
            // SAFETY: 窓の作成と ImmXxx。すべてこのスレッドの窓に対する操作。
            unsafe {
                if disable_ime {
                    let _ = ImmDisableIME(0);
                }
                let hi = GetModuleHandleW(None).unwrap_or_default();
                let top = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("ScenWnd"),
                    PCWSTR(w!("scen").as_ptr()),
                    WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                    x,
                    60,
                    320,
                    140,
                    None,
                    None,
                    Some(hi.into()),
                    None,
                )
                .unwrap_or_default();
                if rich {
                    let _ = LoadLibraryW(w!("Msftedit.dll"));
                }
                let edit = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    if rich { w!("RICHEDIT50W") } else { w!("EDIT") },
                    PCWSTR::null(),
                    WS_CHILD | WS_VISIBLE | WS_BORDER | WINDOW_STYLE(0x80),
                    10,
                    10,
                    280,
                    60,
                    Some(top),
                    None,
                    Some(hi.into()),
                    None,
                )
                .unwrap_or_default();
                if assoc_null {
                    let _ = ImmAssociateContextEx(edit, windows::Win32::UI::Input::Ime::HIMC(std::ptr::null_mut()), 0);
                }
                let _ = tx.send((top.0 as isize, edit.0 as isize, GetCurrentThreadId()));
            }
            pump();
        });
        let (t, e, tid) = rx.recv().unwrap_or((0, 0, 0));
        Win {
            name,
            top: HWND(t as *mut _),
            edit: HWND(e as *mut _),
            tid,
            store: None,
        }
    }

    /// 自前の TSF text store を持つ窓(子の入力欄なし)を、新しいスレッドに作る。`imm`: "default"=何もしない、"assoc"=窓の IMM コンテキストを外す、
    /// "disable"=スレッドで ImmDisableIME(0)(UWP などの TSF のみの形)。IME が動くかは `type_probe` の打鍵結果で見る。
    fn spawn_tsf_window(name: &'static str, x: i32, imm: &'static str) -> Win {
        let (tx, rx) = mpsc::channel::<(isize, u32, crate::tsp::store::Log)>();
        std::thread::spawn(move || {
            register_class();
            // SAFETY: 窓・TSF の初期化はすべてこのスレッドで行い、メッセージループもこのスレッドで回す。
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                if imm == "disable" {
                    let _ = ImmDisableIME(0);
                }
                let thread_mgr: ITfThreadMgr = CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER).expect("ThreadMgr");
                let client_id = thread_mgr.Activate().expect("Activate");
                let hi = GetModuleHandleW(None).unwrap_or_default();
                let top = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("ScenWnd"),
                    PCWSTR(w!("scen-tsf").as_ptr()),
                    WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                    x,
                    60,
                    320,
                    140,
                    None,
                    None,
                    Some(hi.into()),
                    None,
                )
                .unwrap_or_default();
                if imm == "assoc" {
                    let _ = ImmAssociateContextEx(top, windows::Win32::UI::Input::Ime::HIMC(std::ptr::null_mut()), 0);
                }
                crate::tsp::store::TOP.store(top.0 as isize, std::sync::atomic::Ordering::SeqCst);
                let t0 = std::time::Instant::now();
                let slog: crate::tsp::store::Log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
                let store: windows::core::IUnknown = crate::tsp::store::Store::new(std::sync::Arc::clone(&slog), t0, 0, false, false).into();
                let doc = thread_mgr.CreateDocumentMgr().expect("CreateDocumentMgr");
                let mut ctx: Option<ITfContext> = None;
                let mut cookie = 0u32;
                doc.CreateContext(client_id, 0, &store, &raw mut ctx, &raw mut cookie).expect("CreateContext");
                doc.Push(&ctx.expect("ctx")).expect("Push");
                let _ = thread_mgr.AssociateFocus(top, &doc);
                let _ = thread_mgr.SetFocus(&doc);
                let _ = tx.send((top.0 as isize, GetCurrentThreadId(), slog));
                pump();
                let _ = thread_mgr.Deactivate();
            }
        });
        let (t, tid, slog) = rx.recv().expect("tsf window");
        Win {
            name,
            top: HWND(t as *mut _),
            edit: HWND(t as *mut _),
            tid,
            store: Some(slog),
        }
    }

    /// 別スレッドが、`parent`(別スレッドの窓)の子として EDIT を作る(フォーカススレッド ≠ 前面スレッドの構成)。
    fn spawn_child_in_other_thread(name: &'static str, parent: HWND) -> Win {
        let (tx, rx) = mpsc::channel::<(isize, u32)>();
        let p = parent.0 as isize;
        std::thread::spawn(move || {
            // SAFETY: 他スレッドの窓を親にした子窓の作成(入力キューが自動で結合される)。
            unsafe {
                let hi = GetModuleHandleW(None).unwrap_or_default();
                let edit = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("EDIT"),
                    PCWSTR::null(),
                    WS_CHILD | WS_VISIBLE | WS_BORDER | WINDOW_STYLE(0x80),
                    10,
                    75,
                    280,
                    40,
                    Some(HWND(p as *mut _)),
                    None,
                    Some(hi.into()),
                    None,
                )
                .unwrap_or_default();
                let _ = tx.send((edit.0 as isize, GetCurrentThreadId()));
            }
            pump();
        });
        let (e, tid) = rx.recv().unwrap_or((0, 0));
        Win {
            name,
            top: parent,
            edit: HWND(e as *mut _),
            tid,
            store: None,
        }
    }

    // ---------------------------------------------------------------- 操作
    fn focus(w: &Win) {
        // SAFETY: 前面化と入力の結合(spike と同じ定番の手順)。
        unsafe {
            let fg = GetForegroundWindow();
            let fg_tid = if fg.0.is_null() { 0 } else { GetWindowThreadProcessId(fg, None) };
            let my = GetCurrentThreadId();
            let attached_fg = fg_tid != 0 && fg_tid != my && AttachThreadInput(my, fg_tid, true).as_bool();
            let target_tid = GetWindowThreadProcessId(w.top, None);
            let attached_t = target_tid != my && AttachThreadInput(my, target_tid, true).as_bool();
            let _ = BringWindowToTop(w.top);
            let _ = SetForegroundWindow(w.top);
            let _ = SetFocus(Some(w.edit));
            if attached_t {
                let _ = AttachThreadInput(my, target_tid, false);
            }
            if attached_fg {
                let _ = AttachThreadInput(my, fg_tid, false);
            }
        }
        log(&format!("SW target={} edit_tid={}", w.name, w.tid));
    }

    fn scan_for(vk: u32) -> u16 {
        match vk {
            0xF2 | 0xF1 => 0x70,
            0xF3 | 0xF4 => 0x29,
            0xF0 => 0x3A,
            _ => 0,
        }
    }

    fn key(vk: u32, down: bool) {
        let scan = {
            let s = scan_for(vk);
            if s != 0 {
                s
            } else {
                // SAFETY: 読み取りのみ。
                u16::try_from(unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC) }).unwrap_or(0)
            }
        };
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(u16::try_from(vk).unwrap_or(0)),
                    wScan: scan,
                    dwFlags: if down { KEYBD_EVENT_FLAGS(0) } else { KEYEVENTF_KEYUP },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        // SAFETY: 1 イベントの注入。
        unsafe {
            let _ = SendInput(&[input], i32::try_from(std::mem::size_of::<INPUT>()).unwrap_or(40));
        }
    }

    fn press(vk: u32) {
        log(&format!("PRESS vk={vk:#04x}"));
        key(vk, true);
        sleep_ms(120);
        key(vk, false);
    }

    fn ime_ctl(ime: HWND, cmd: usize) -> Option<usize> {
        let mut r: usize = 0;
        // SAFETY: 他スレッドの IME 窓への WM_IME_CONTROL(タイムアウト付き)。
        let ok = unsafe {
            SendMessageTimeoutW(ime, WM_IME_CONTROL, WPARAM(cmd), LPARAM(0), SMTO_ABORTIFHUNG, 300, Some(&raw mut r))
        };
        (ok.0 != 0).then_some(r)
    }

    fn ime_set(ime: HWND, cmd: usize, lp: isize) -> Option<usize> {
        let mut r: usize = 0;
        // SAFETY: 他スレッドの IME 窓への WM_IME_CONTROL(タイムアウト付き)。
        let ok = unsafe {
            SendMessageTimeoutW(ime, WM_IME_CONTROL, WPARAM(cmd), LPARAM(lp), SMTO_ABORTIFHUNG, 300, Some(&raw mut r))
        };
        (ok.0 != 0).then_some(r)
    }

    fn truth(w: &Win) {
        // SAFETY: 読み取りのみ。
        let ime = unsafe { ImmGetDefaultIMEWnd(w.edit) };
        if ime.0.is_null() {
            log(&format!("TRUTH {} no-default-ime-wnd", w.name));
            return;
        }
        let open = ime_ctl(ime, 5);
        let conv = ime_ctl(ime, 1);
        log(&format!("TRUTH {} open={open:?} conv={conv:?}", w.name));
    }

    /// 実際の IME の効果を、打った出力で読む(IMM が読めない窓でも使える正解): `ka` + Enter を打って、窓のテキストを読む。
    fn type_probe(w: &Win) {
        for vk in [0x4B_u32, 0x41, 0x0D] {
            key(vk, true);
            sleep_ms(60);
            key(vk, false);
            sleep_ms(250);
        }
        sleep_ms(500);
        if let Some(slog) = &w.store {
            let text = slog
                .lock()
                .ok()
                .and_then(|l| l.iter().rev().find(|r| r.tag == "LockReleased").map(|r| r.detail.clone()))
                .unwrap_or_default();
            log(&format!("TYPED {} store={text:?}", w.name));
            return;
        }
        let mut buf = [0u16; 64];
        // SAFETY: 同一プロセスの窓へのメッセージ。buf はローカル。
        let n = unsafe { SendMessageW(w.edit, WM_GETTEXT, Some(WPARAM(buf.len())), Some(LPARAM(buf.as_mut_ptr() as isize))) }.0;
        let text = String::from_utf16_lossy(&buf[..usize::try_from(n).unwrap_or(0).min(63)]);
        log(&format!("TYPED {} text={text:?}", w.name));
        // SAFETY: 同上。
        unsafe {
            let _ = SendMessageW(w.edit, WM_SETTEXT, Some(WPARAM(0)), Some(LPARAM(w!("").as_ptr() as isize)));
        }
    }

    fn focus_info() {
        // SAFETY: 読み取りのみ。
        unsafe {
            let fg = GetForegroundWindow();
            let fg_tid = if fg.0.is_null() { 0 } else { GetWindowThreadProcessId(fg, None) };
            let mut gti = GUITHREADINFO {
                cbSize: u32::try_from(std::mem::size_of::<GUITHREADINFO>()).unwrap_or(0),
                ..Default::default()
            };
            let focus_tid = if fg_tid != 0 && GetGUIThreadInfo(fg_tid, &raw mut gti).is_ok() && !gti.hwndFocus.0.is_null() {
                GetWindowThreadProcessId(gti.hwndFocus, None)
            } else {
                0
            };
            log(&format!("FOCUS fg_tid={fg_tid} focus_tid={focus_tid}"));
        }
    }

    /// IME を ON にして、`vk`(F2 ひらがな / F0 半角英数 / 1A 閉)の状態にする。
    fn set_state(w: &Win, vk: u32) {
        focus(w);
        sleep_ms(500);
        press(0x16);
        sleep_ms(600);
        if vk != 0x16 {
            press(vk);
        }
        sleep_ms(1500);
        truth(w);
    }

    pub fn run() -> anyhow::Result<()> {
        let _ = LOG_PATH.set(arg("--log=").unwrap_or_else(|| "indicator_scenario.log".to_owned()));
        let scen = arg("--scen=").unwrap_or_else(|| "switch".to_owned());
        let n: u32 = arg("--n=").and_then(|v| v.parse().ok()).unwrap_or(6);
        log(&format!("start scen={scen} n={n}"));
        activate_profile();
        register_class();

        match scen.as_str() {
            "switch" => {
                // 状態の違う 2 窓を行き来する。A=ひらがな、B=半角英数(1周目)→閉(2周目)。
                let a = spawn_window("A", 40, false, false);
                let b = spawn_window("B", 400, false, false);
                set_state(&a, 0xF2);
                set_state(&b, 0xF0);
                for round in 0..2 {
                    if round == 1 {
                        set_state(&b, 0x1A);
                    }
                    for i in 0..n {
                        focus(&a);
                        sleep_ms(1800);
                        truth(&a);
                        log(&format!("ROUND round={round} i={i} shown=A"));
                        focus(&b);
                        sleep_ms(1800);
                        truth(&b);
                        log(&format!("ROUND round={round} i={i} shown=B"));
                    }
                }
            }
            "cycle" => {
                // 1 つの窓で、`--keys=<16進VKをカンマ区切り>` を順に押し、押すたびに実 IME の状態(TRUTH)を記録する。字形の画像は並行する
                // `ime_indicator_probe --raw=<dir>` が保存する(DPI・テーマを替えた画像の収集用)。`--n` は全体の繰り返し数。
                let keys: Vec<u32> = arg("--keys=")
                    .unwrap_or_else(|| "F2,F1,F3,F4,F0,1A".to_owned())
                    .split(',')
                    .filter_map(|v| u32::from_str_radix(v.trim(), 16).ok())
                    .collect();
                // `--win=tsf [--imm=default|assoc|disable]`: IMM で状態を読めない/嘘をつく窓(自前の TSF text store の窓。観測の無い窓の近似)。
                let tsf = arg("--win=").as_deref() == Some("tsf");
                let imm: &'static str = match arg("--imm=").as_deref() {
                    Some("assoc") => "assoc",
                    Some("disable") => "disable",
                    _ => "default",
                };
                let a = if tsf { spawn_tsf_window("A", 40, imm) } else { spawn_window("A", 40, false, false) };
                focus(&a);
                sleep_ms(800);
                press(0x16);
                sleep_ms(800);
                // `--steps=<open>:<conv>,…`(16進): IMM で開閉と変換モードを書く(IMM が効く窓=MS-IME 本体向け)。
                let steps: Vec<(isize, isize)> = arg("--steps=")
                    .map(|v| {
                        v.split(',')
                            .filter_map(|t| {
                                let (o, c) = t.split_once(':')?;
                                Some((isize::from_str_radix(o, 16).ok()?, isize::from_str_radix(c, 16).ok()?))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if !steps.is_empty() {
                    // SAFETY: 既定の IME 窓の取得のみ。
                    let ime = unsafe { ImmGetDefaultIMEWnd(a.edit) };
                    for i in 0..n {
                        for &(o, c) in &steps {
                            // 書き込みの瞬間を記録する(字形の変化との突き合わせ用。GJI の PRESS と同じ役割)。
                            log(&format!("SET open={o:X} conv={c:X}"));
                            let _ = ime_set(ime, 2, c);
                            let _ = ime_set(ime, 6, o);
                            sleep_ms(1500);
                            truth(&a);
                            log(&format!("CYCLE i={i} set open={o:X} conv={c:X}"));
                        }
                    }
                }
                for i in 0..n {
                    if !steps.is_empty() {
                        break;
                    }
                    for &vk in &keys {
                        press(vk);
                        sleep_ms(1500);
                        truth(&a);
                        if tsf || arg("--type=").is_some() {
                            // IMM が読めない窓の実状態は、打った出力(かな/ローマ字)で読む。`--type=1` なら通常の窓でも打って確かめる(GJI は IMM が嘘をつくので正解にならない)。
                            type_probe(&a);
                        }
                        log(&format!("CYCLE i={i} vk={vk:02X}"));
                    }
                }
                // `--hold=<秒>`: 状態を作ったあと、窓にフォーカスを残したまま待つ(別プロセスがインジケーターを右クリックして、メニューを測る用)。
                if let Some(h) = arg("--hold=").and_then(|v| v.parse::<u64>().ok()) {
                    focus(&a);
                    log(&format!("HOLD start {h}s"));
                    sleep_ms(h * 1000);
                    log("HOLD end");
                }
            }
            "noimm" => {
                // A=ひらがな の窓から、IMM の無い窓(C1: ImmAssociateContextEx(NULL)、C2: ImmDisableIME)へ移る。
                let a = spawn_window("A", 40, false, false);
                let c1 = spawn_window("C1", 400, false, true);
                let c2 = spawn_window("C2", 700, true, false);
                set_state(&a, 0xF2);
                for i in 0..n {
                    for c in [&c1, &c2] {
                        focus(c);
                        sleep_ms(2200);
                        truth(c);
                        focus_info();
                        log(&format!("ROUND i={i} shown={}", c.name));
                        focus(&a);
                        sleep_ms(1800);
                        truth(&a);
                        log(&format!("ROUND i={i} shown=A"));
                    }
                }
            }
            "tsfstore" => {
                // 自前の TSF text store の窓 T(IMM との関わりは --imm=default|assoc|disable)。IME が動くかは打鍵結果(ストアのテキスト)で見る。
                // 字形が T 自身の状態を映すか、直前の窓のままかを、直前の窓の状態を替えて区別する(A=ひらがな、B=半角英数。T は 1 段階目=ひらがな、2 段階目=半角英数)。
                let imm: &'static str = match arg("--imm=").as_deref() {
                    Some("assoc") => "assoc",
                    Some("disable") => "disable",
                    _ => "default",
                };
                log(&format!("tsfstore imm={imm}"));
                let a = spawn_window("A", 40, false, false);
                let b = spawn_window("B", 400, false, false);
                let t = spawn_tsf_window("T", 700, imm);
                set_state(&a, 0xF2);
                set_state(&b, 0xF0);
                for phase in 0..2 {
                    set_state(&t, if phase == 0 { 0xF2 } else { 0xF0 });
                    type_probe(&t);
                    for i in 0..n {
                        let prev = if phase == 0 { &b } else { &a };
                        focus(prev);
                        sleep_ms(1800);
                        truth(prev);
                        log(&format!("ROUND phase={phase} i={i} shown={}", prev.name));
                        focus(&t);
                        sleep_ms(2200);
                        type_probe(&t);
                        truth(&t);
                        log(&format!("ROUND phase={phase} i={i} shown=T prev={}", prev.name));
                    }
                }
            }
            "tsfonly" => {
                // IMM を無効にした TSF のみのスレッド T(スレッドで ImmDisableIME + TSF 対応の RichEdit)。UWP などの形。
                // IME が動いていること(打った出力)を確かめたうえで、字形が T 自身の状態を映すか、直前の窓のままかを、
                // 直前の窓の状態を替えて区別する。A=ひらがな、B=半角英数。T は 1 段階目=ひらがな、2 段階目=半角英数。
                let a = spawn_window("A", 40, false, false);
                let b = spawn_window("B", 400, false, false);
                let t = spawn_window_cls("T", 700, true, false, true);
                set_state(&a, 0xF2);
                set_state(&b, 0xF0);
                for phase in 0..2 {
                    // T の状態を作る(1 段階目 F2=ひらがな、2 段階目 F0=半角英数)。打って確かめる。
                    set_state(&t, if phase == 0 { 0xF2 } else { 0xF0 });
                    type_probe(&t);
                    for i in 0..n {
                        let prev = if phase == 0 { &b } else { &a };
                        focus(prev);
                        sleep_ms(1800);
                        truth(prev);
                        log(&format!("ROUND phase={phase} i={i} shown={}", prev.name));
                        focus(&t);
                        sleep_ms(2200);
                        type_probe(&t);
                        log(&format!("ROUND phase={phase} i={i} shown=T prev={}", prev.name));
                    }
                }
            }
            "noimm2" => {
                // ImmDisableIME の窓(C2)の字形が「直前の窓の状態のまま(古い)」か「その窓固有」かを、直前の窓の状態を替えて区別する。
                // A=ひらがな、B=半角英数。B→C2、A→C2 を繰り返す。字形が直前に追随するなら古い証拠、常に同じなら固有。
                let a = spawn_window("A", 40, false, false);
                let b = spawn_window("B", 400, false, false);
                let c2 = spawn_window("C2", 700, true, false);
                set_state(&a, 0xF2);
                set_state(&b, 0xF0);
                for i in 0..n {
                    for prev in [&b, &a] {
                        focus(prev);
                        sleep_ms(1800);
                        truth(prev);
                        log(&format!("ROUND i={i} shown={}", prev.name));
                        focus(&c2);
                        sleep_ms(2200);
                        truth(&c2);
                        log(&format!("ROUND i={i} shown=C2 prev={}", prev.name));
                    }
                }
            }
            "focusthread" => {
                // F(最上位、スレッド TF)の中に、別スレッド TG の EDIT(G)を子として置く。前面=TF、フォーカス=EF または EG。
                let f = spawn_window("F", 40, false, false);
                let g = spawn_child_in_other_thread("G", f.top);
                set_state(&g, 0xF2); // スレッド TG = ひらがな
                set_state(&f, 0xF0); // スレッド TF = 半角英数
                for i in 0..n {
                    focus(&g);
                    sleep_ms(1800);
                    truth(&g);
                    focus_info();
                    log(&format!("ROUND i={i} shown=G"));
                    focus(&f);
                    sleep_ms(1800);
                    truth(&f);
                    focus_info();
                    log(&format!("ROUND i={i} shown=F"));
                }
            }
            other => log(&format!("unknown scen {other}")),
        }
        log("=== 完了 ===");
        Ok(())
    }
}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    app::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows 専用");
}
