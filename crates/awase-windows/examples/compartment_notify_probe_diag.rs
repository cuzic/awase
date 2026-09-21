//! (診断コピー: compartment_notify_probe.rs は ADR-193 セッションの成果物なので編集せず、CI で前面化に失敗する原因の
//! 切り分けのため別名で置く。前面化の戦略を順に試して結果を DIAG 行に残す。それ以外は本体と同じ。)
//! ADR-193/ADR-191 スパイク: TSF スレッド compartment の**変更通知**（`ITfCompartmentEventSink`）が、
//! キー押下から何 ms で届くかを測る。較正（キー×状態→効果の表を作る）を「押下後 N 秒待って読む」から
//! 「通知が来たら読む」へ置き換えられるかの検証用。
//!
//! 読み取りは ADR-186 の手法T（素の Win32 窓で `CoCreateInstance(CLSID_TF_ThreadMgr)` + `Activate()` して
//! スレッド compartment を読む、実測で全件一致）と同じ。ここに `ITfSource::AdviseSink(ITfCompartmentEventSink)` を足す。
//! 購読対象は `KEYBOARD_OPENCLOSE` / `KEYBOARD_INPUTMODE_CONVERSION` / `KEYBOARD_INPUTMODE_SENTENCE`。
//!
//! 使い方: `compartment_notify_probe [--seq=16,1A,F2,F2,1D,1C] [--gap=1500] [--poll=50] [--richedit] [--log=<path>]`
//!   `--seq`: 注入する VK（16進、カンマ区切り）。既定は IME ON → IME OFF → ひらがな×2 → 無変換 → 変換。
//!   `--gap`: キー間隔(ms)。`--poll`: 比較用に、メインスレッドのタイマーで compartment を読む間隔(ms、0で無効)。
//!   `--richedit`: 入力欄を素の `EDIT` でなく `RICHEDIT50W` にする。
//! キーは `SendInput`（`AWASE_TEST_INJECTION=1` の awase が物理キー扱いする目印付き）で注入する。
//! **awase を止めた状態（IME 単体）で測るのが基本**。前面・フォーカスがプローブ窓にないときは注入しない。
//! 実行中は Windows 機のキーボード・マウスに触らない。

#![allow(unsafe_code)]

#[cfg(windows)]
// `#[implement(...)]`（windows-rs）が生成する内部コードがこのリポジトリの pedantic/nursery deny に触れるため、
// マクロ生成部分にまとめて allow する（`spike_langbar_input_mode.rs` と同じ扱い）。
#[allow(clippy::ref_as_ptr, clippy::inline_always)]
mod sink {
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use windows::core::{implement, GUID};
    use windows::Win32::UI::TextServices::{
        ITfCompartmentEventSink, ITfCompartmentEventSink_Impl, ITfCompartmentMgr,
        GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION,
        GUID_COMPARTMENT_KEYBOARD_INPUTMODE_SENTENCE, GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
    };

    #[derive(Clone, Debug)]
    pub(crate) struct Event {
        pub(crate) at_ms: u128,
        pub(crate) kind: &'static str, // "KEY" | "NOTIFY" | "POLL"
        pub(crate) name: String,
        pub(crate) value: Option<i32>,
    }

    pub(crate) type Events = Arc<Mutex<Vec<Event>>>;

    pub(crate) fn name_of(g: &GUID) -> &'static str {
        if *g == GUID_COMPARTMENT_KEYBOARD_OPENCLOSE {
            "OPENCLOSE"
        } else if *g == GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION {
            "CONVERSION"
        } else if *g == GUID_COMPARTMENT_KEYBOARD_INPUTMODE_SENTENCE {
            "SENTENCE"
        } else {
            "OTHER"
        }
    }

    pub(crate) fn read_value(cmgr: &ITfCompartmentMgr, g: &GUID) -> Option<i32> {
        // SAFETY: cmgr は有効な COM 参照、g は有効な GUID への参照。
        unsafe {
            let c = cmgr.GetCompartment(g).ok()?;
            let v = c.GetValue().ok()?;
            i32::try_from(&v).ok()
        }
    }

    #[implement(ITfCompartmentEventSink)]
    pub(crate) struct CompSink {
        pub(crate) cmgr: ITfCompartmentMgr,
        pub(crate) t0: Instant,
        pub(crate) events: Events,
    }

    impl ITfCompartmentEventSink_Impl for CompSink_Impl {
        fn OnChange(&self, rguid: *const GUID) -> windows::core::Result<()> {
            let at_ms = self.t0.elapsed().as_millis();
            // SAFETY: rguid はこのコールバックの実行中のみ有効な、TSF ランタイムが用意したポインタ。
            let Some(g) = (unsafe { rguid.as_ref() }) else {
                return Ok(());
            };
            let value = read_value(&self.cmgr, g);
            if let Ok(mut ev) = self.events.lock() {
                ev.push(Event {
                    at_ms,
                    kind: "NOTIFY",
                    name: name_of(g).to_string(),
                    value,
                });
            }
            Ok(())
        }
    }
}

#[cfg(windows)]
mod app {
    use std::cell::RefCell;
    use std::io::Write as _;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use windows::core::{w, Interface, GUID, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, SetFocus, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_KEYUP, VIRTUAL_KEY,
    };
    use windows::Win32::UI::TextServices::{
        CLSID_TF_InputProcessorProfiles, CLSID_TF_ThreadMgr, ITfCompartmentEventSink,
        ITfCompartmentMgr, ITfInputProcessorProfileMgr, ITfSource, ITfThreadMgr,
        GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION,
        GUID_COMPARTMENT_KEYBOARD_INPUTMODE_SENTENCE, GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
        GUID_TFCAT_TIP_KEYBOARD, TF_INPUTPROCESSORPROFILE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, BringWindowToTop, CreateWindowExW, DefWindowProcW,
        DispatchMessageW, GetClassNameW, GetForegroundWindow, GetGUIThreadInfo, GetMessageW,
        GetWindowThreadProcessId, IsWindowVisible, KillTimer, PostMessageW, PostQuitMessage,
        RegisterClassExW, SetForegroundWindow, SetTimer, ShowWindow, TranslateMessage,
        CW_USEDEFAULT, GUITHREADINFO, MSG, SW_SHOW, WM_APP, WM_CLOSE, WM_DESTROY, WM_SETFOCUS,
        WM_TIMER, WNDCLASSEXW, WS_BORDER, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };

    use super::sink::{name_of, read_value, CompSink, Event, Events};

    /// スパイク/chrome_probe と同じ目印。`AWASE_TEST_INJECTION=1` の awase は、この目印の注入を物理キーとして扱う。
    const AUTO_MARKER: usize = awase_windows::hook::TEST_INJECTION_MARKER;
    const TIMER_ID: usize = 1;

    static DIAG: Mutex<Vec<String>> = Mutex::new(Vec::new());
    fn diag(s: String) {
        if let Ok(mut d) = DIAG.lock() {
            d.push(s);
        }
    }
    /// 前面窓・プローブ窓・フォーカスの状態を1行にする。
    fn fg_info() -> String {
        // SAFETY: Win32 の読み取り API。
        unsafe {
            let top = hwnd_of(&TOP);
            let edit = hwnd_of(&EDIT);
            let fg = GetForegroundWindow();
            let mut cls = [0u16; 128];
            let n = if fg.0.is_null() {
                0
            } else {
                GetClassNameW(fg, &mut cls)
            };
            let fgcls = String::from_utf16_lossy(&cls[..n.max(0) as usize]);
            let mut gi = GUITHREADINFO {
                cbSize: size_of::<GUITHREADINFO>() as u32,
                ..Default::default()
            };
            let ok = GetGUIThreadInfo(0, &raw mut gi).is_ok();
            format!(
                "fg={:?}({fgcls}) top={:?} fg==top:{} visible:{} gui_ok:{ok} focus={:?} active={:?} edit={:?}",
                fg.0,
                top.0,
                fg == top,
                IsWindowVisible(top).as_bool(),
                gi.hwndFocus.0,
                gi.hwndActive.0,
                edit.0
            )
        }
    }

    static TOP: AtomicIsize = AtomicIsize::new(0);
    static EDIT: AtomicIsize = AtomicIsize::new(0);

    struct PollState {
        cmgr: ITfCompartmentMgr,
        last: [Option<i32>; 3],
        t0: Instant,
        events: Events,
    }
    thread_local! {
        static POLL: RefCell<Option<PollState>> = const { RefCell::new(None) };
    }

    const GUIDS: [&GUID; 3] = [
        &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
        &GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION,
        &GUID_COMPARTMENT_KEYBOARD_INPUTMODE_SENTENCE,
    ];

    /// アクティブなキーボード TIP を「GJI / MS-IME / その他」で返す。
    /// 測定結果がどの IME のものか分からないと、IME を取り違えた結論になる（実際に MS-IME を GJI と誤認した前例がある）。
    fn active_tip() -> String {
        // GJI(Google 日本語入力)と Microsoft IME(日本語)の TIP CLSID。
        const GJI: GUID = GUID::from_u128(0xD5A86FD5_5308_47EA_AD16_9C4EB160EC3C);
        const MSIME: GUID = GUID::from_u128(0x03B5835F_F03C_411B_9CE2_AA23E1171E36);
        // SAFETY: STA スレッドで CoInitializeEx 済みの後に呼ぶ。GetActiveProfile は out 構造体に書き込む。
        unsafe {
            let mgr: windows::core::Result<ITfInputProcessorProfileMgr> =
                CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER);
            let Ok(mgr) = mgr else {
                return "取得失敗(ProfileMgr)".to_string();
            };
            let mut p = TF_INPUTPROCESSORPROFILE::default();
            if mgr
                .GetActiveProfile(&GUID_TFCAT_TIP_KEYBOARD, &raw mut p)
                .is_err()
            {
                return "取得失敗(GetActiveProfile)".to_string();
            }
            let kind = if p.clsid == GJI {
                "GJI"
            } else if p.clsid == MSIME {
                "MS-IME"
            } else {
                "その他"
            };
            format!("{kind} clsid={:?} langid=0x{:X}", p.clsid, p.langid)
        }
    }

    fn hwnd_of(v: &AtomicIsize) -> HWND {
        HWND(v.load(Ordering::SeqCst) as *mut core::ffi::c_void)
    }

    fn sleep(ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn arg_value(args: &[String], key: &str) -> Option<String> {
        args.iter()
            .find_map(|a| a.strip_prefix(key).map(str::to_string))
    }

    fn send_key(vk: u32, down: bool) {
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(u16::try_from(vk).unwrap_or(0)),
                    wScan: 0,
                    dwFlags: if down {
                        KEYBD_EVENT_FLAGS(0)
                    } else {
                        KEYEVENTF_KEYUP
                    },
                    time: 0,
                    dwExtraInfo: AUTO_MARKER,
                },
            },
        };
        // SAFETY: input は有効な INPUT 1件。
        unsafe {
            let _ = SendInput(&[input], size_of::<INPUT>() as i32);
        }
    }

    fn bring_to_front(hwnd: HWND) -> bool {
        // SAFETY: Win32 の前面化 API。hwnd は自プロセスの有効なウィンドウ。
        unsafe {
            let fg = GetForegroundWindow();
            let fg_tid = if fg.0.is_null() {
                0
            } else {
                GetWindowThreadProcessId(fg, None)
            };
            let my_tid = GetCurrentThreadId();
            let attached = fg_tid != 0
                && fg_tid != my_tid
                && AttachThreadInput(my_tid, fg_tid, true).as_bool();
            let _ = BringWindowToTop(hwnd);
            let ok = SetForegroundWindow(hwnd).as_bool();
            if attached {
                let _ = AttachThreadInput(my_tid, fg_tid, false);
            }
            ok || GetForegroundWindow() == hwnd
        }
    }

    /// 前面窓が `top`、かつそのスレッドのフォーカスが `edit` にあるか。取得に失敗したら false。
    fn focus_on_probe(top: HWND, edit: HWND) -> bool {
        // SAFETY: GetGUIThreadInfo は cbSize を設定した GUITHREADINFO を書き込む。
        unsafe {
            if GetForegroundWindow() != top {
                return false;
            }
            let mut gi = GUITHREADINFO {
                cbSize: size_of::<GUITHREADINFO>() as u32,
                ..Default::default()
            };
            GetGUIThreadInfo(0, &raw mut gi).is_ok() && gi.hwndFocus == edit
        }
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        // SAFETY: ウィンドウプロシージャ。DefWindowProcW/PostQuitMessage は任意の引数で安全。
        unsafe {
            match msg {
                WM_DESTROY => {
                    PostQuitMessage(0);
                    LRESULT(0)
                }
                WM_TIMER if wp.0 == TIMER_ID => {
                    poll_once();
                    LRESULT(0)
                }
                WM_SETFOCUS => {
                    // 原因の修正案(スパイクの wndproc と同じ): 前面化されるとトップ窓にフォーカスが移り、子の入力欄から外れる。
                    // 入力欄へフォーカスを戻す(本体には無く、CI の focus_on_probe が false になって ABORT していた)。
                    let edit = hwnd_of(&EDIT);
                    if !edit.0.is_null() {
                        let _ = SetFocus(Some(edit));
                    }
                    LRESULT(0)
                }
                WM_APP => {
                    // UI スレッドでの前面化(スパイクは UI スレッドの WM_TIMER から前面化していて CI で成功している)。
                    let ok = bring_to_front(hwnd);
                    diag(format!(
                        "S3 UIスレッドで bring_to_front → {ok}; {}",
                        fg_info()
                    ));
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, msg, wp, lp),
            }
        }
    }

    /// 比較用: メインスレッドのタイマーで compartment を読み、変化があれば POLL イベントを残す。
    fn poll_once() {
        POLL.with(|p| {
            let mut p = p.borrow_mut();
            let Some(p) = p.as_mut() else { return };
            let at_ms = p.t0.elapsed().as_millis();
            for (i, g) in GUIDS.iter().enumerate() {
                let v = read_value(&p.cmgr, g);
                if v != p.last[i] {
                    p.last[i] = v;
                    if let Ok(mut ev) = p.events.lock() {
                        ev.push(Event {
                            at_ms,
                            kind: "POLL",
                            name: name_of(g).to_string(),
                            value: v,
                        });
                    }
                }
            }
        });
    }

    /// panic フック: sink の `OnChange` など COM コールバック内の panic は非 unwind ABI（`extern "system"`）の境界で
    /// プロセスごと abort し、終了時のタイムライン出力に到達しない。abort の前に走るこのフックで、panic の内容と
    /// 直前までのタイムラインをログファイルへ書き出し、測定データを失わないようにする。
    fn install_panic_hook(path: String, events: Events) {
        std::panic::set_hook(Box::new(move |info| {
            let mut s = format!("PANIC: {info}\n--- panic 直前までのタイムライン ---\n");
            // panic が push 中に起きた場合にデッドロックしないよう try_lock を使う。
            if let Ok(ev) = events.try_lock() {
                for e in ev.iter() {
                    s.push_str(&format!(
                        "+{:>6}ms {:<6} {:<10} {:?}\n",
                        e.at_ms, e.kind, e.name, e.value
                    ));
                }
            } else {
                s.push_str("(イベントのロックを取得できなかった)\n");
            }
            eprintln!("{s}");
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(&path)
            {
                let _ = f.write_all(s.as_bytes());
            }
        }));
    }

    pub(crate) fn main() {
        let args: Vec<String> = std::env::args().collect();
        // 不正な要素や空の列は、測定と誤読されないよう黙って捨てずに終了する。
        let seq_arg = arg_value(&args, "--seq=").unwrap_or_else(|| "16,1A,F2,F2,1D,1C".to_string());
        let mut seq: Vec<u32> = Vec::new();
        for tok in seq_arg.split(',') {
            match u32::from_str_radix(tok.trim(), 16) {
                Ok(v) => seq.push(v),
                Err(_) => {
                    eprintln!(
                        "--seq の要素 {tok:?} は16進のVKとして解釈できません(--seq={seq_arg})"
                    );
                    std::process::exit(2);
                }
            }
        }
        if seq.is_empty() {
            eprintln!("--seq が空です");
            std::process::exit(2);
        }
        let gap_ms: u64 = arg_value(&args, "--gap=")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1500);
        let poll_ms: u32 = arg_value(&args, "--poll=")
            .and_then(|v| v.parse().ok())
            .unwrap_or(50);
        let use_rich = args.iter().any(|a| a == "--richedit");
        let log_path = arg_value(&args, "--log=")
            .unwrap_or_else(|| "compartment_notify_probe_diag.log".into());
        let mut log = std::fs::File::create(&log_path).expect("log");
        let mut out = |s: &str| {
            println!("{s}");
            let _ = writeln!(log, "{s}");
        };

        let t0 = Instant::now();
        let events: Events = Arc::new(Mutex::new(Vec::new()));
        install_panic_hook(log_path.clone(), Arc::clone(&events));

        // SAFETY: メインスレッド(STA)で COM/TSF と窓を初期化する。以降の COM 呼び出しは同じスレッドから行う。
        let (cmgr, thread_mgr, cookies) = unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .expect("CoInitializeEx");
            let thread_mgr: ITfThreadMgr =
                CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)
                    .expect("ThreadMgr");
            let _client = thread_mgr.Activate().expect("Activate");
            let cmgr = thread_mgr
                .cast::<ITfCompartmentMgr>()
                .expect("ITfCompartmentMgr");

            let instance = GetModuleHandleW(None).expect("module");
            if use_rich {
                let _ = LoadLibraryW(w!("Msftedit.dll"));
            }
            let cls = wide("CompartmentNotifyProbeTop");
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                lpszClassName: PCWSTR(cls.as_ptr()),
                ..Default::default()
            };
            RegisterClassExW(&raw const wc);
            let top = CreateWindowExW(
                windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
                PCWSTR(cls.as_ptr()),
                w!("compartment notify probe"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                600,
                200,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .expect("top");
            let edit_class = if use_rich {
                w!("RICHEDIT50W")
            } else {
                w!("EDIT")
            };
            let edit = CreateWindowExW(
                windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
                edit_class,
                w!(""),
                WS_CHILD | WS_VISIBLE | WS_BORDER,
                10,
                10,
                560,
                100,
                Some(top),
                None,
                Some(instance.into()),
                None,
            )
            .expect("edit");
            let _ = ShowWindow(top, SW_SHOW);
            let _ = SetFocus(Some(edit));
            TOP.store(top.0 as isize, Ordering::SeqCst);
            EDIT.store(edit.0 as isize, Ordering::SeqCst);

            // 3つの compartment それぞれに通知 sink を購読する。
            let mut cookies = Vec::new();
            for g in GUIDS {
                let comp = match cmgr.GetCompartment(g) {
                    Ok(c) => c,
                    Err(e) => {
                        out(&format!("GetCompartment({}) 失敗: {e:?}", name_of(g)));
                        continue;
                    }
                };
                let source = match comp.cast::<ITfSource>() {
                    Ok(s) => s,
                    Err(e) => {
                        out(&format!("ITfSource({}) 失敗: {e:?}", name_of(g)));
                        continue;
                    }
                };
                let sink: ITfCompartmentEventSink = CompSink {
                    cmgr: cmgr.clone(),
                    t0,
                    events: Arc::clone(&events),
                }
                .into();
                match source.AdviseSink(&<ITfCompartmentEventSink as Interface>::IID, &sink) {
                    Ok(cookie) => {
                        out(&format!("AdviseSink({}) ok cookie={cookie}", name_of(g)));
                        cookies.push((source, cookie, sink));
                    }
                    Err(e) => out(&format!("AdviseSink({}) 失敗: {e:?}", name_of(g))),
                }
            }
            (cmgr, thread_mgr, cookies)
        };

        out(&format!("アクティブTIP: {}", active_tip()));
        let initial: Vec<String> = GUIDS
            .iter()
            .map(|g| format!("{}={:?}", name_of(g), read_value(&cmgr, g)))
            .collect();
        out(&format!("初期値: {}", initial.join(" ")));

        if poll_ms > 0 {
            POLL.with(|p| {
                *p.borrow_mut() = Some(PollState {
                    cmgr: cmgr.clone(),
                    last: [
                        read_value(&cmgr, GUIDS[0]),
                        read_value(&cmgr, GUIDS[1]),
                        read_value(&cmgr, GUIDS[2]),
                    ],
                    t0,
                    events: Arc::clone(&events),
                });
            });
            // SAFETY: TOP は上で作成した有効な窓。
            unsafe {
                SetTimer(Some(hwnd_of(&TOP)), TIMER_ID, poll_ms, None);
            }
        }

        let worker_events = Arc::clone(&events);
        let worker_seq = seq.clone();
        let worker = std::thread::spawn(move || {
            let top = hwnd_of(&TOP);
            let edit = hwnd_of(&EDIT);
            sleep(1500);
            diag(format!("S-1 起動直後(前面化前); {}", fg_info()));
            let mut fronted = bring_to_front(top);
            sleep(300);
            diag(format!(
                "S0 ワーカースレッドで bring_to_front → {fronted}; {}",
                fg_info()
            ));
            if !focus_on_probe(top, edit) {
                // S1: AllowSetForegroundWindow(ASFW_ANY) の後にもう一度。
                unsafe {
                    let _ = AllowSetForegroundWindow(u32::MAX);
                }
                fronted = bring_to_front(top);
                sleep(300);
                diag(format!("S1 ASFW_ANY 後 → {fronted}; {}", fg_info()));
            }
            if !focus_on_probe(top, edit) {
                // S2: ALT の押下/解放を注入してから(前面化ロックの定番の回避策)。
                send_key(0x12, true);
                send_key(0x12, false);
                sleep(100);
                fronted = bring_to_front(top);
                sleep(300);
                diag(format!("S2 ALT 注入後 → {fronted}; {}", fg_info()));
            }
            if !focus_on_probe(top, edit) {
                // S3: UI スレッド(窓を作ったスレッド)で前面化する。
                unsafe {
                    let _ = PostMessageW(Some(top), WM_APP, WPARAM(0), LPARAM(0));
                }
                sleep(500);
                fronted = focus_on_probe(top, edit);
                diag(format!(
                    "S3 後の判定 focus_on_probe={fronted}; {}",
                    fg_info()
                ));
            }
            sleep(800);
            let push = |kind: &'static str, name: String| {
                if let Ok(mut ev) = worker_events.lock() {
                    ev.push(Event {
                        at_ms: t0.elapsed().as_millis(),
                        kind,
                        name,
                        value: None,
                    });
                }
            };
            'run: {
                // キー注入の前に、前面かつフォーカスがプローブ窓にあることを確かめる。
                if !fronted || !focus_on_probe(top, edit) {
                    push("ABORT", "前面化またはフォーカスに失敗".to_string());
                    break 'run;
                }
                for vk in &worker_seq {
                    if !focus_on_probe(top, edit) {
                        push("ABORT", format!("vk=0x{vk:02X} の前にフォーカスが外れた"));
                        break 'run;
                    }
                    push("KEY", format!("0x{vk:02X}"));
                    send_key(*vk, true);
                    sleep(40);
                    send_key(*vk, false);
                    sleep(gap_ms);
                }
                sleep(600);
            }
            // SAFETY: top は有効な窓。WM_CLOSE でメインのメッセージループを終わらせる。
            unsafe {
                let _ = PostMessageW(Some(top), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        });

        // SAFETY: メインスレッドのメッセージループ。sink の OnChange はここで配送される。
        unsafe {
            let mut msg = MSG::default();
            while GetMessageW(&raw mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&raw const msg);
                DispatchMessageW(&raw const msg);
            }
            let _ = KillTimer(Some(hwnd_of(&TOP)), TIMER_ID);
        }
        let _ = worker.join();
        for (source, cookie, _sink) in &cookies {
            // SAFETY: cookie は AdviseSink が返した有効な cookie。
            let _ = unsafe { source.UnadviseSink(*cookie) };
        }
        // SAFETY: 上の Activate と対で、同じスレッドから呼ぶ。
        let _ = unsafe { thread_mgr.Deactivate() };

        // 集計: 各 KEY の直後に最初に来た NOTIFY / POLL までの遅延。
        let ev = events.lock().map(|e| e.clone()).unwrap_or_default();
        out("--- タイムライン(ms は起動からの経過) ---");
        for e in &ev {
            out(&format!(
                "+{:>6}ms {:<6} {:<10} {}",
                e.at_ms,
                e.kind,
                e.name,
                e.value
                    .map_or(String::new(), |v| format!("= {v} (0x{v:X})"))
            ));
        }
        out("--- 前面化の診断(DIAG) ---");
        if let Ok(d) = DIAG.lock() {
            for l in d.iter() {
                out(&format!("DIAG {l}"));
            }
        }
        out("--- キーごとの遅延(押下→最初の通知/ポーリング検出) ---");
        for (i, e) in ev.iter().enumerate().filter(|(_, e)| e.kind == "KEY") {
            let next_key = ev[i + 1..]
                .iter()
                .find(|x| x.kind == "KEY")
                .map_or(u128::MAX, |x| x.at_ms);
            let first = |kind: &str| {
                ev[i + 1..]
                    .iter()
                    .find(|x| x.kind == kind && x.at_ms < next_key)
                    .map(|x| format!("{}ms ({} {:?})", x.at_ms - e.at_ms, x.name, x.value))
            };
            out(&format!(
                "KEY {}: NOTIFY={} POLL={}",
                e.name,
                first("NOTIFY").unwrap_or_else(|| "なし".to_string()),
                first("POLL").unwrap_or_else(|| "なし".to_string())
            ));
        }
        out("=== 完了 ===");
    }
}

#[cfg(windows)]
fn main() {
    app::main();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows 専用のプローブです。");
}
