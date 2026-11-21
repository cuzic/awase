//! スパイク: `IUIAutomation3::AddTextEditTextChangedEventHandler`
//! （`ITextEditProvider`、`TextEditChangeType::Composition`/`CompositionFinalized`/`AutoCorrect`）が、
//! 実際のテキストコントロール上で IME composition に対して発火するかを測る（観測のみ、awase 非依存）。
//!
//! 背景: `crates/awase-ime-core/src/tsf/literal_facts.rs` の literal-detect は、「composition として
//! 確定したか」を IO カウンタ差分・`candidate_visible` 等の**間接証拠**から推測している。この UIA イベントが
//! 実際に(a) 発火し (b) composition の確定/取消を直接運ぶなら、間接推測を置き換える一次証拠になりうる。
//! 一方 ADR-261 の実測（`docs/tasks/textedit-change-event-probe.md` 追記）では UIA 自体の対応状況がアプリ
//! 依存に偏ることが分かっている。
//!
//! 作り: 自前の top-level window に `EDIT`（ITextEditProvider 未対応と予想）と `RICHEDIT50W`
//! （`Msftedit.dll`、対応していると予想）の2種類の子コントロールを順に生成し、どちらにも
//! `AddTextEditTextChangedEventHandler` を登録する。登録後、(1) IME 無しで ASCII を送る健全性チェック、
//! (2) MS-IME をひらがなに切り替えて実際に composition を起こすキー列を送る、の2段をそれぞれのコントロール
//! に対して行い、コールバックが呼ばれた回数・種別・運ばれてきた文字列を記録する。
//!
//! 入力言語設定（`Set-WinUserLanguageList ja-JP` 等）は呼び出し側（CI ワークフロー）が済ませておくこと。
//! キーは `SendInput`（awase 非依存、マーカーなし）。実行中は Windows 機のキーボード・マウスに触らない。
//!
//! 出力: 1行1レコード `TXTEDIT {json}`（`form`・`event_n`・`kind`・`strings`・`t_ms`、コールバックの中から
//! 直接 println! する）。各フォームの開始時に `# form=<edit|rich> register composition=<bool> autocorrect=<bool>`
//! を出す。「発火したか」は、そのフォームのブロックの中に `TXTEDIT {"form":"<edit|rich>",...}` 行が
//! 1つ以上あるかで判定する（register が true でも、provider 未対応なら一度も発火しない）。

#![allow(unsafe_code)]

#[cfg(windows)]
#[allow(clippy::ref_as_ptr, clippy::inline_always)]
mod sink {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;
    use std::time::Instant;

    use windows::core::implement;
    use windows::Win32::System::Com::SAFEARRAY;
    use windows::Win32::System::Ole::{
        SafeArrayGetElement, SafeArrayGetLBound, SafeArrayGetUBound,
    };
    use windows::Win32::UI::Accessibility::{
        IUIAutomationElement, IUIAutomationTextEditTextChangedEventHandler_Impl, TextEditChangeType,
    };
    use windows_core::BSTR;

    /// `SAFEARRAY`（VT_BSTR 要素）を `Vec<String>` に展開する。null や取得失敗は空扱い。
    fn safearray_bstrs(psa: *const SAFEARRAY) -> Vec<String> {
        if psa.is_null() {
            return Vec::new();
        }
        let mut out = Vec::new();
        // SAFETY: `psa` はコールバックが渡した有効な SAFEARRAY（null チェック済み）。1次元を仮定
        // （`AddTextEditTextChangedEventHandler` のドキュメント上、`eventStrings` は常に1次元の BSTR 配列）。
        unsafe {
            let Ok(lbound) = SafeArrayGetLBound(psa, 1) else {
                return out;
            };
            let Ok(ubound) = SafeArrayGetUBound(psa, 1) else {
                return out;
            };
            let mut i = lbound;
            while i <= ubound {
                let mut bstr = BSTR::default();
                let got = SafeArrayGetElement(
                    psa,
                    &raw const i,
                    (&raw mut bstr).cast::<core::ffi::c_void>(),
                );
                if got.is_ok() {
                    out.push(String::try_from(&bstr).unwrap_or_default());
                }
                i += 1;
            }
        }
        out
    }

    fn kind_name(t: TextEditChangeType) -> &'static str {
        match t.0 {
            0 => "None",
            1 => "AutoCorrect",
            2 => "Composition",
            3 => "CompositionFinalized",
            4 => "AutoComplete",
            _ => "Unknown",
        }
    }

    /// 受けたイベントを記録する受け口。`form`（"edit"/"rich"）はラベル用で意味は運ばない。
    // 内部フィールド名はマクロが生成する `_Impl` ラッパー自身の refcount フィールド名
    // （`windows_core::imp::IUnknownImpl` が持つ `count: WeakRefCount`）と衝突しうるため、
    // `count`/`kinds` のような汎用名は避ける（実際に `count` で衝突し `fetch_add` が
    // `WeakRefCount` 上のものに解決されてコンパイルエラーになった）。
    #[implement(windows::Win32::UI::Accessibility::IUIAutomationTextEditTextChangedEventHandler)]
    pub(super) struct TextEditSink {
        pub form: &'static str,
        pub epoch: Instant,
        pub fired_n: AtomicU32,
        pub fired_kinds: Mutex<Vec<&'static str>>,
    }

    impl TextEditSink {
        pub(super) fn new(form: &'static str, epoch: Instant) -> Self {
            Self {
                form,
                epoch,
                fired_n: AtomicU32::new(0),
                fired_kinds: Mutex::new(Vec::new()),
            }
        }
    }

    impl IUIAutomationTextEditTextChangedEventHandler_Impl for TextEditSink_Impl {
        fn HandleTextEditTextChangedEvent(
            &self,
            _sender: windows_core::Ref<IUIAutomationElement>,
            texteditchangetype: TextEditChangeType,
            eventstrings: *const SAFEARRAY,
        ) -> windows_core::Result<()> {
            let n = self.fired_n.fetch_add(1, Ordering::SeqCst) + 1;
            let kind = kind_name(texteditchangetype);
            if let Ok(mut g) = self.fired_kinds.lock() {
                g.push(kind);
            }
            let strings = safearray_bstrs(eventstrings);
            let t_ms = self.epoch.elapsed().as_millis();
            let json = serde_json::json!({
                "form": self.form,
                "event_n": n,
                "kind": kind,
                "strings": strings,
                "t_ms": t_ms,
            });
            println!("TXTEDIT {json}");
            Ok(())
        }
    }
}

#[cfg(windows)]
mod app {
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::time::{Duration, Instant};

    use windows::core::{w, Interface, PCWSTR};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
    use windows::Win32::UI::Accessibility::{
        CUIAutomation8, IUIAutomation, IUIAutomation3, TextEditChangeType_AutoCorrect,
        TextEditChangeType_Composition, TreeScope_Element,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, SetFocus, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_KEYUP, VIRTUAL_KEY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, PeekMessageW,
        PostQuitMessage, RegisterClassExW, ShowWindow, TranslateMessage, CW_USEDEFAULT, MSG,
        PM_REMOVE, SW_SHOW, WINDOW_EX_STYLE, WM_DESTROY, WNDCLASSEXW, WS_BORDER, WS_CHILD,
        WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };

    use super::sink::TextEditSink;

    const ES_AUTOHSCROLL: u32 = 0x0080;
    const VK_IME_OFF: u32 = 0x1A;
    const VK_DBE_HIRAGANA: u32 = 0xF2;
    const VK_RETURN: u32 = 0x0D;

    static TOP: AtomicIsize = AtomicIsize::new(0);

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    unsafe extern "system" fn top_proc(
        hwnd: HWND,
        msg: u32,
        wparam: windows::Win32::Foundation::WPARAM,
        lparam: windows::Win32::Foundation::LPARAM,
    ) -> windows::Win32::Foundation::LRESULT {
        if msg == WM_DESTROY {
            // SAFETY: メッセージループスレッドからの通常の終了通知。
            unsafe { PostQuitMessage(0) };
            return windows::Win32::Foundation::LRESULT(0);
        }
        // SAFETY: 既定処理へ委譲するだけ。
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    fn pump(ms: u64) {
        let until = Instant::now() + Duration::from_millis(ms);
        let mut msg = MSG::default();
        while Instant::now() < until {
            // SAFETY: 標準のメッセージポンプ。
            unsafe {
                while PeekMessageW(&raw mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&raw const msg);
                    DispatchMessageW(&raw const msg);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
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
                    dwExtraInfo: 0,
                },
            },
        };
        // SAFETY: 単発の INPUT 配列を渡すだけ。
        unsafe {
            let _ = SendInput(&[input], i32::try_from(size_of::<INPUT>()).unwrap_or(0));
        }
    }

    fn press(vk: u32, hold_ms: u64) {
        send_key(vk, true);
        std::thread::sleep(Duration::from_millis(hold_ms));
        send_key(vk, false);
        std::thread::sleep(Duration::from_millis(60));
    }

    fn type_ascii(s: &str) {
        for c in s.chars() {
            let vk = u32::from(c.to_ascii_uppercase() as u8);
            press(vk, 40);
            pump(30);
        }
    }

    /// `form`（"edit"/"rich"）の子コントロールを作り、`AddTextEditTextChangedEventHandler` を
    /// `Composition` と `AutoCorrect` の両方で登録し、ASCII → IME 切替 → ローマ字 → 確定 の順にキーを送る。
    /// 発火した `TXTEDIT` 行は登録済みコールバックから直接 println! されるので、この関数自体は何も返さない。
    fn run_form(automation: &IUIAutomation, automation3: &IUIAutomation3, form: &'static str) {
        let epoch = Instant::now();
        // SAFETY: 単純な Win32 ウィンドウ/コントロール作成。すべてこの関数内で作り、終了時に破棄する。
        unsafe {
            let _ = LoadLibraryW(w!("Msftedit.dll"));
            let instance = GetModuleHandleW(None).expect("module");
            let top_class = wide("TextEditProbeTop");
            let wc = WNDCLASSEXW {
                cbSize: u32::try_from(size_of::<WNDCLASSEXW>()).unwrap_or(0),
                lpfnWndProc: Some(top_proc),
                hInstance: instance.into(),
                lpszClassName: PCWSTR(top_class.as_ptr()),
                ..Default::default()
            };
            RegisterClassExW(&raw const wc);
            let top = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                PCWSTR(top_class.as_ptr()),
                w!("textedit probe"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                500,
                200,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .expect("top window");
            TOP.store(top.0 as isize, Ordering::SeqCst);

            let class = wide(if form == "rich" {
                "RICHEDIT50W"
            } else {
                "EDIT"
            });
            let child = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                PCWSTR(class.as_ptr()),
                w!(""),
                windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(
                    WS_CHILD.0 | WS_VISIBLE.0 | WS_BORDER.0 | ES_AUTOHSCROLL,
                ),
                10,
                10,
                400,
                28,
                Some(top),
                None,
                Some(instance.into()),
                None,
            )
            .unwrap_or_else(|e| panic!("[{form}] 入力欄の作成に失敗: {e}"));
            let _ = ShowWindow(top, SW_SHOW);
            let _ = SetFocus(Some(child));
            pump(300);

            let element = match automation.ElementFromHandle(child) {
                Ok(e) => e,
                Err(e) => {
                    println!("# form={form} ElementFromHandle failed: {e:?}");
                    let _ = DestroyWindow(top);
                    return;
                }
            };
            let sink: windows::Win32::UI::Accessibility::IUIAutomationTextEditTextChangedEventHandler =
                TextEditSink::new(form, epoch).into();
            let reg_comp = automation3.AddTextEditTextChangedEventHandler(
                &element,
                TreeScope_Element,
                TextEditChangeType_Composition,
                None,
                &sink,
            );
            let reg_auto = automation3.AddTextEditTextChangedEventHandler(
                &element,
                TreeScope_Element,
                TextEditChangeType_AutoCorrect,
                None,
                &sink,
            );
            println!("# form={form} register composition={reg_comp:?} autocorrect={reg_auto:?}");

            // 1. ASCII 健全性チェック(IME 無し)
            type_ascii("abc");
            pump(500);

            // 2. MS-IME をひらがなへ切り替え、ローマ字を打ち、確定する
            press(VK_IME_OFF, 50);
            std::thread::sleep(Duration::from_millis(600));
            press(VK_DBE_HIRAGANA, 50);
            std::thread::sleep(Duration::from_millis(1500));
            type_ascii("ka");
            pump(500);
            press(VK_RETURN, 50);
            pump(1500);

            let _ = automation3.RemoveTextEditTextChangedEventHandler(&element, &sink);
            let _ = DestroyWindow(top);
            pump(200);
        }
    }

    pub(crate) fn main() {
        // SAFETY: プロセス起動直後に一度だけ呼ぶ。
        unsafe {
            let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            if hr.is_err() {
                println!("# CoInitializeEx failed: {hr:?}");
                return;
            }
        }
        // SAFETY: CoInitializeEx の直後。
        let automation: IUIAutomation =
            match unsafe { CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER) } {
                Ok(a) => a,
                Err(e) => {
                    println!("# CoCreateInstance(CUIAutomation8) failed: {e:?}");
                    return;
                }
            };
        let automation3: IUIAutomation3 = match automation.cast() {
            Ok(a) => a,
            Err(e) => {
                println!("# IUIAutomation3 cast failed: {e:?}");
                return;
            }
        };

        for form in ["edit", "rich"] {
            run_form(&automation, &automation3, form);
        }
        println!("# done");
    }
}

#[cfg(windows)]
fn main() {
    app::main();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("this spike is Windows-only");
}
