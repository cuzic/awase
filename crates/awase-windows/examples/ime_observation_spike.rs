//! ADR-176 技術スパイク: awase-settings相当の生ウィンドウ（TextEdit非フォーカス、
//! egui/winit不使用）に対して、IME open/close状態を観測する3手法を同時に監視し、
//! どれが確実に機能するかを1回の実機操作で確定する。
//!
//! - **手法A**: `ImmGetContext(hwnd)` → `ImmGetOpenStatus(himc)`。
//!   ADR-125（BUG-107調査）がawase-settings.exeでは`himc=0x0`固定と実測した経路。
//!   このスパイクは素のWin32ウィンドウ（winitのIME明示デタッチが無い）なので、
//!   ADR-125とは異なる結果になりうる（比較対象として有用）。
//! - **手法B**: `ImmGetDefaultIMEWnd(hwnd)` → `SendMessageTimeoutW(WM_IME_CONTROL,
//!   IMC_GETOPENSTATUS)`。awase本体が実際に使っている経路
//!   （`crates/awase-windows/src/imm.rs::probe_ime_control`と同型、ADR-125が
//!   awase-settings.exeで機能することを実測済み）。
//! - **手法C**: TSF `ITfThreadMgr::GetGlobalCompartment()` →
//!   `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE`の`ITfCompartment::GetValue()`。
//!   HIMCにもクロスプロセスメッセージにも依存しない、COMベースの第3の経路。
//!
//! ## 使い方
//! 1. Windows実機でビルド: `cargo build --example ime_observation_spike -p awase-windows`
//! 2. `target/debug/examples/ime_observation_spike.exe`を実行する
//!    （コンソールが自動的に開き、ログがそこに出力される）。
//! 3. 表示されたウィンドウ（テキスト入力欄は無い、ボタン等も無い、較正パネルの
//!    「ボタンとラベルの画面」を模したもの）にフォーカスする。
//! 4. GJIまたはMS-IMEを手動でON/OFF切り替える（半角/全角キー等）。
//! 5. コンソールに出力される3手法それぞれの値の変化を確認する。
//!    250msごとにポーリングし、いずれかの値が変化したときだけログを出す。
//!
//! 各手法が`None`（取得失敗）を返す場合と、値は取れるが実際のIME操作と
//! 相関しない場合の両方を区別できるよう、`None`/`Some(bool)`をそのまま表示する。

#![allow(unsafe_code)]

use std::cell::RefCell;

use windows::core::{w, Result as WinResult};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::Ime::{
    ImmGetContext, ImmGetDefaultIMEWnd, ImmGetOpenStatus, ImmReleaseContext,
};
use windows::Win32::UI::TextServices::{
    CLSID_TF_ThreadMgr, ITfCompartmentMgr, ITfThreadMgr, GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, PostQuitMessage,
    RegisterClassW, SendMessageTimeoutW, SetTimer, TranslateMessage, CW_USEDEFAULT, MSG,
    SMTO_ABORTIFHUNG, WM_DESTROY, WM_TIMER, WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

const WM_IME_CONTROL: u32 = 0x0283;
const IMC_GETOPENSTATUS: usize = 0x0005;
const TIMER_ID: usize = 1;
const TIMER_INTERVAL_MS: u32 = 250;

struct TsfState {
    // ITfThreadMgr自体は保持し続けないとCOMオブジェクトが解放される。
    _thread_mgr: ITfThreadMgr,
    compartment_mgr: ITfCompartmentMgr,
}

/// 3手法それぞれの観測値（`None`=取得失敗）。
type Observation = (Option<bool>, Option<bool>, Option<bool>);

thread_local! {
    static TSF_STATE: RefCell<Option<TsfState>> = const { RefCell::new(None) };
    // `None` = まだ1回も観測していない（初回は必ず印字する、実際の観測値が
    // たまたま全部Noneだった場合と区別するため、番兵として`(Option<bool>,...)`の
    // タプル自体をOptionで包む）。
    static LAST_SEEN: RefCell<Option<Observation>> = const { RefCell::new(None) };
    static TICK_COUNT: RefCell<u64> = const { RefCell::new(0) };
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
        println!("[init] TSF ITfThreadMgr activated, GetGlobalCompartment OK");
    }
    Ok(())
}

fn fmt(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "Some(true)",
        Some(false) => "Some(false)",
        None => "None       ",
    }
}

fn on_timer(hwnd: HWND) {
    use std::io::Write as _;

    let a = method_a_imm_get_open_status(hwnd);
    let b = method_b_wm_ime_control(hwnd);
    let c = method_c_tsf_compartment();

    let changed = LAST_SEEN.with(|last| {
        let mut last = last.borrow_mut();
        let changed = *last != Some((a, b, c));
        *last = Some((a, b, c));
        changed
    });

    // 20tick(=5秒)ごとに強制的にheartbeatを出す。プロセス自体が生きている
    // ことと、タイマーが実際に動いていることを、値の変化が無い場合でも
    // 確認できるようにするため。
    let heartbeat = TICK_COUNT.with(|c| {
        let mut c = c.borrow_mut();
        *c += 1;
        *c % 20 == 0
    });

    if changed || heartbeat {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        println!(
            "[{:>10}.{:03}]{} A(ImmGetOpenStatus)={} B(WM_IME_CONTROL)={} C(TSF compartment)={}",
            now.as_secs(),
            now.subsec_millis(),
            if changed { "" } else { " [heartbeat]" },
            fmt(a),
            fmt(b),
            fmt(c),
        );
        let _ = std::io::stdout().flush();
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TIMER => {
                on_timer(hwnd);
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
            w!("IME Observation Spike (ADR-176) - no TextEdit, focus me and toggle IME"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            560,
            200,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        Ok(hwnd)
    }
}

fn main() -> WinResult<()> {
    use std::io::Write as _;

    println!("=== ADR-176 IME observation spike ===");
    println!("手法A: ImmGetContext + ImmGetOpenStatus");
    println!("手法B: ImmGetDefaultIMEWnd + WM_IME_CONTROL/IMC_GETOPENSTATUS（awase本体と同型）");
    println!("手法C: TSF GUID_COMPARTMENT_KEYBOARD_OPENCLOSE（COM、HIMC非依存）");
    println!();
    let _ = std::io::stdout().flush();

    if let Err(e) = init_tsf() {
        println!("[init] TSF初期化に失敗しました（手法Cはこのプロセスでは使えません）: {e}");
        let _ = std::io::stdout().flush();
    }

    let hwnd = match create_window() {
        Ok(hwnd) => hwnd,
        Err(e) => {
            eprintln!("[fatal] ウィンドウ作成に失敗しました: {e}");
            let _ = std::io::stdout().flush();
            return Err(e);
        }
    };
    println!("[init] ウィンドウ作成OK, hwnd={hwnd:?}");
    unsafe {
        SetTimer(Some(hwnd), TIMER_ID, TIMER_INTERVAL_MS, None);
    }
    let _ = std::io::stdout().flush();

    println!("ウィンドウにフォーカスして、GJI/MS-IMEを手動でON/OFF切り替えてください。");
    println!("このウィンドウにはテキスト入力欄がありません（較正パネルの想定に近い状態）。");
    println!("値が変化したときだけログが出ます。Ctrl+C または ウィンドウを閉じて終了。");
    println!();

    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&raw mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&raw const msg);
            DispatchMessageW(&raw const msg);
        }
    }
    Ok(())
}
