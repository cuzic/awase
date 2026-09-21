//! ADR-176 技術スパイク: 単一のWin32ウィンドウ（コンソール無し）に対して、
//! IME open/close状態を観測する3手法を同時に監視し、どれが確実に機能するかを
//! 1回の実機操作で確定する。
//!
//! **コンソールを使わない理由**: 最初の版はコンソールにログを出力していたが、
//! ユーザー指摘により「コンソール（別ウィンドウ、Windows Terminalホスト＝
//! TsfNativeアプリの可能性がある）を使う設計自体が紛らわしい」と判明した。
//! `#![windows_subsystem = "windows"]`でコンソール自体を作らず、単一の
//! Win32ウィンドウの中にテキスト入力欄とログ表示欄を両方置く。
//!
//! - **手法A**: `ImmGetContext(hwnd)` → `ImmGetOpenStatus(himc)`。
//!   ADR-125（BUG-107調査）がawase-settings.exeでは`himc=0x0`固定と実測した経路。
//! - **手法B**: `ImmGetDefaultIMEWnd(hwnd)` → `SendMessageTimeoutW(WM_IME_CONTROL,
//!   IMC_GETOPENSTATUS)`。awase本体が実際に使っている経路
//!   （`crates/awase-windows/src/imm.rs::probe_ime_control`と同型）。
//! - **手法C**: TSF `ITfThreadMgr::GetGlobalCompartment()` →
//!   `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE`の`ITfCompartment::GetValue()`。
//!   HIMCにもクロスプロセスメッセージにも依存しない、COMベースの第3の経路。
//!
//! ## 使い方
//! 1. Windows実機でビルド: `cargo build --example ime_observation_spike -p awase-windows`
//! 2. `target/debug/examples/ime_observation_spike.exe`を実行する
//!    （コンソールは開かない、ウィンドウが1つだけ表示される）。
//! 3. 上段のテキスト入力欄（起動直後は自動でフォーカスされている）に
//!    フォーカスした状態で、GJIまたはMS-IMEを手動でON/OFF切り替える
//!    （半角/全角キー等）。ついでに何か文字を入力してみてもよい。
//! 4. 下段のログ表示欄に、3手法それぞれの値が変化したときだけ1行追加される。
//!    ログ欄の内容はそのまま選択・コピーできる。

#![windows_subsystem = "windows"]
#![allow(unsafe_code)]

use std::cell::RefCell;
use std::fmt::Write as _;

use windows::core::{w, Result as WinResult, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::Ime::{
    ImmGetContext, ImmGetDefaultIMEWnd, ImmGetOpenStatus, ImmReleaseContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
use windows::Win32::UI::TextServices::{
    CLSID_TF_ThreadMgr, ITfCompartmentMgr, ITfThreadMgr, GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, MessageBoxW,
    PostQuitMessage, RegisterClassW, SendMessageTimeoutW, SendMessageW, SetTimer, ShowWindow,
    TranslateMessage, CW_USEDEFAULT, MB_ICONERROR, MB_OK, MSG, SMTO_ABORTIFHUNG, SW_SHOW,
    WINDOW_STYLE, WM_DESTROY, WM_SETFOCUS, WM_TIMER, WNDCLASSW, WS_BORDER, WS_CHILD,
    WS_OVERLAPPEDWINDOW, WS_VISIBLE, WS_VSCROLL,
};

const WM_IME_CONTROL: u32 = 0x0283;
const IMC_GETOPENSTATUS: usize = 0x0005;
const TIMER_ID: usize = 1;
const TIMER_INTERVAL_MS: u32 = 250;
const ES_MULTILINE: u32 = 0x0004;
const ES_READONLY: u32 = 0x0800;
const ES_AUTOVSCROLL: u32 = 0x0040;
const MAX_LOG_CHARS: usize = 12_000;
// `Win32_UI_Controls` featureを新たに有効化せずに済むよう、EMメッセージは
// 定数値を直書きする（標準Win32ヘッダの既知の固定値）。
const EM_SETSEL: u32 = 177;
const EM_REPLACESEL: u32 = 194;

struct TsfState {
    // ITfThreadMgr自体は保持し続けないとCOMオブジェクトが解放される。
    _thread_mgr: ITfThreadMgr,
    compartment_mgr: ITfCompartmentMgr,
}

/// 3手法それぞれの観測値（`None`=取得失敗）。
type Observation = (Option<bool>, Option<bool>, Option<bool>);

thread_local! {
    static TSF_STATE: RefCell<Option<TsfState>> = const { RefCell::new(None) };
    static LAST_SEEN: RefCell<Option<Observation>> = const { RefCell::new(None) };
    static TICK_COUNT: RefCell<u64> = const { RefCell::new(0) };
    static EDIT_HWND: RefCell<Option<HWND>> = const { RefCell::new(None) };
    static LOG_HWND: RefCell<Option<HWND>> = const { RefCell::new(None) };
    static LOG_BUF: RefCell<String> = const { RefCell::new(String::new()) };
}

/// 手法A: `ImmGetContext` + `ImmGetOpenStatus`。
fn method_a_imm_get_open_status(hwnd: HWND) -> Option<bool> {
    unsafe {
        let himc = ImmGetContext(hwnd);
        if himc.is_invalid() {
            return None;
        }
        let open = ImmGetOpenStatus(himc);
        let _ = ImmReleaseContext(hwnd, himc);
        Some(open.as_bool())
    }
}

/// 手法B: `ImmGetDefaultIMEWnd` + `WM_IME_CONTROL`/`IMC_GETOPENSTATUS`
/// （awase本体の`imm::probe_ime_control`と同型）。
fn method_b_wm_ime_control(hwnd: HWND) -> Option<bool> {
    unsafe {
        let ime_wnd = ImmGetDefaultIMEWnd(hwnd);
        if ime_wnd.0.is_null() {
            return None;
        }
        let mut result: usize = 0;
        let ok = SendMessageTimeoutW(
            ime_wnd,
            WM_IME_CONTROL,
            WPARAM(IMC_GETOPENSTATUS),
            LPARAM(0),
            SMTO_ABORTIFHUNG,
            TIMER_INTERVAL_MS,
            Some(&raw mut result),
        );
        if ok.0 == 0 {
            return None;
        }
        Some(result != 0)
    }
}

/// 手法C: TSF `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE`（COM、HIMC非依存）。
fn method_c_tsf_compartment() -> Option<bool> {
    TSF_STATE.with(|state| {
        let state = state.borrow();
        let state = state.as_ref()?;
        unsafe {
            let compartment = state
                .compartment_mgr
                .GetCompartment(&GUID_COMPARTMENT_KEYBOARD_OPENCLOSE)
                .ok()?;
            let variant = compartment.GetValue().ok()?;
            let v = i32::try_from(&variant).ok()?;
            Some(v != 0)
        }
    })
}

fn init_tsf() -> WinResult<()> {
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let thread_mgr: ITfThreadMgr =
            CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)?;
        let _client_id = thread_mgr.Activate()?;
        let compartment_mgr = thread_mgr.GetGlobalCompartment()?;
        TSF_STATE.with(|s| {
            *s.borrow_mut() = Some(TsfState {
                _thread_mgr: thread_mgr,
                compartment_mgr,
            });
        });
    }
    Ok(())
}

fn fmt(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "true ",
        Some(false) => "false",
        None => "None ",
    }
}

/// ログ表示欄（下段のEDIT）へ1行追記する。バッファが大きくなりすぎたら
/// 先頭側を捨てる（`EM_REPLACESEL`で末尾に追記、`SetWindowTextW`は使わない
/// ——毎回全体を再セットするとスクロール位置が飛ぶ）。
fn append_log(line: &str) {
    LOG_BUF.with(|buf| {
        let mut buf = buf.borrow_mut();
        buf.push_str(line);
        buf.push_str("\r\n");
        if buf.len() > MAX_LOG_CHARS {
            let cut = buf.len() - MAX_LOG_CHARS;
            let cut = buf
                .char_indices()
                .map(|(i, _)| i)
                .find(|&i| i >= cut)
                .unwrap_or(0);
            buf.drain(0..cut);
        }
    });
    let Some(log_hwnd) = LOG_HWND.with(|h| *h.borrow()) else {
        return;
    };
    unsafe {
        // 末尾へキャレットを移動してから追記する（EM_SETSEL(-1,-1) → EM_REPLACESEL）。
        let _ = SendMessageW(
            log_hwnd,
            EM_SETSEL,
            Some(WPARAM(usize::MAX)),
            Some(LPARAM(-1)),
        );
        let mut wide: Vec<u16> = line
            .encode_utf16()
            .chain(std::iter::once(u16::from(b'\r')))
            .chain(std::iter::once(u16::from(b'\n')))
            .collect();
        wide.push(0);
        let _ = SendMessageW(
            log_hwnd,
            EM_REPLACESEL,
            Some(WPARAM(1)),
            Some(LPARAM(wide.as_ptr() as isize)),
        );
    }
}

fn on_timer(hwnd: HWND) {
    // フォーカスされているウィンドウ（通常は上段のEDIT）を対象にIME状態を
    // 読む。IME状態は「入力フォーカスを持つウィンドウ」に紐づくため、
    // トップレベルウィンドウ自身ではなく実際にフォーカスを持つ子ウィンドウを
    // 使う方が正確。
    let target = unsafe { GetFocus() };
    let target = if target.0.is_null() { hwnd } else { target };

    let a = method_a_imm_get_open_status(target);
    let b = method_b_wm_ime_control(target);
    let c = method_c_tsf_compartment();

    let changed = LAST_SEEN.with(|last| {
        let mut last = last.borrow_mut();
        let changed = *last != Some((a, b, c));
        *last = Some((a, b, c));
        changed
    });

    // 40tick(=10秒)ごとに強制的にheartbeatを出す。プロセス自体が生きている
    // ことと、タイマーが実際に動いていることを、値の変化が無い場合でも
    // 確認できるようにするため。
    let heartbeat = TICK_COUNT.with(|c| {
        let mut c = c.borrow_mut();
        *c += 1;
        *c % 40 == 0
    });

    if changed || heartbeat {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let mut line = String::new();
        let _ = write!(
            line,
            "[{:>10}.{:03}]{} A={} B={} C={}",
            now.as_secs(),
            now.subsec_millis(),
            if changed { "" } else { " (heartbeat)" },
            fmt(a),
            fmt(b),
            fmt(c),
        );
        append_log(&line);
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TIMER => {
                on_timer(hwnd);
                LRESULT(0)
            }
            WM_SETFOCUS => {
                // トップレベルウィンドウがフォーカスを得たら、常に上段の
                // 入力欄へ委譲する。
                if let Some(edit) = EDIT_HWND.with(|e| *e.borrow()) {
                    let _ = SetFocus(Some(edit));
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                let _ = KillTimer(Some(hwnd), TIMER_ID);
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn create_child_edit(
    parent: HWND,
    instance: windows::Win32::Foundation::HMODULE,
    style_extra: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> WinResult<HWND> {
    unsafe {
        let style = (WS_CHILD | WS_VISIBLE | WS_BORDER).0 | style_extra;
        CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
            w!("EDIT"),
            w!(""),
            WINDOW_STYLE(style),
            x,
            y,
            w,
            h,
            Some(parent),
            None,
            Some(instance.into()),
            None,
        )
    }
}

fn create_window() -> WinResult<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class_name = w!("ImeObservationSpikeWindowClass");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        RegisterClassW(&raw const wc);
        let hwnd = CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
            class_name,
            w!("ADR-176 IME observation spike"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            700,
            480,
            None,
            None,
            Some(instance.into()),
            None,
        )?;

        // 上段: 実際に打鍵する入力欄。
        let edit_hwnd = create_child_edit(hwnd, instance, 0, 10, 10, 660, 30)?;
        EDIT_HWND.with(|e| *e.borrow_mut() = Some(edit_hwnd));

        // 下段: ログ表示欄（複数行・読み取り専用・縦スクロール）。
        let log_hwnd = create_child_edit(
            hwnd,
            instance,
            ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL | WS_VSCROLL.0,
            10,
            50,
            660,
            380,
        )?;
        LOG_HWND.with(|h| *h.borrow_mut() = Some(log_hwnd));

        let _ = SetFocus(Some(edit_hwnd));
        let _ = ShowWindow(hwnd, SW_SHOW);
        Ok(hwnd)
    }
}

/// `#![windows_subsystem = "windows"]`だとコンソールが無く、パニックや
/// エラーが起きても何も見えず静かにプロセスが終了する。`MessageBoxW`で
/// 必ず可視化する。
fn report_fatal(msg: &str) {
    let title: Vec<u16> = "ADR-176 spike: fatal error"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let text: Vec<u16> = msg.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn run() -> WinResult<()> {
    let tsf_ok = init_tsf().is_ok();

    let hwnd = create_window()?;

    append_log("=== ADR-176 IME observation spike ===");
    append_log("A = ImmGetContext+ImmGetOpenStatus / B = ImmGetDefaultIMEWnd+WM_IME_CONTROL(awase本体と同型) / C = TSF GUID_COMPARTMENT_KEYBOARD_OPENCLOSE");
    if !tsf_ok {
        append_log("[init] TSF初期化に失敗しました（Cは使えません）");
    }
    append_log("上段の入力欄にフォーカスした状態でIMEをON/OFF切り替えてください。");
    append_log("");

    unsafe {
        SetTimer(Some(hwnd), TIMER_ID, TIMER_INTERVAL_MS, None);
    }

    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&raw mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&raw const msg);
            DispatchMessageW(&raw const msg);
        }
    }
    Ok(())
}

fn main() {
    std::panic::set_hook(Box::new(|info| {
        report_fatal(&format!("panic: {info}"));
    }));
    if let Err(e) = run() {
        report_fatal(&format!("error: {e}"));
    }
}
