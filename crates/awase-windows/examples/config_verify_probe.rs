//! 設定の実機確認(ADR-201、`.github/workflows/config-verify.yml`)用の最小ハーネス。
//! awase 本体は変更しない。**実際に起動している awase.exe** に対して、キーを `SendInput` で注入して結果を読み戻す。
//!
//! ## モード
//! - `--mode=hotkey [--marker=0|1]`: awase の `engine_toggle_hotkey`(Ctrl+Shift+F12)が実際に効くかの補助。
//!   比較対象(対照)として**自プロセスで** Ctrl+Shift+F10 を `RegisterHotKey` し、同じ方法で注入して `WM_HOTKEY`
//!   が届くかを記録する。対照が届かないなら「注入では RegisterHotKey が発火しない環境」で、awase 側の判定は
//!   CI では確かめられない(UNVERIFIABLE)。awase 側の結果は awase.log(`Engine user_enabled toggled:`)を
//!   ワークフローのスクリプトが読む。`--marker=1` は `AWASE_TEST_INJECTION` 用の目印を `dwExtraInfo` に付ける。
//! - `--mode=caret`: 複数行 EDIT に3行の文字列を入れ、`Ctrl+P`(→上)・`Ctrl+N`(→下)を注入して、
//!   キャレットの行(`EM_LINEFROMCHAR`)が動くかを読む。対照として素の `VK_UP`/`VK_DOWN` も注入する
//!   (ハーネス自体が動くかの確認)。目印付き(`AWASE_TEST_INJECTION=1` の debug ビルド awase が物理キー扱い)。
//!
//! ログは `[CV-JSON] {...}` の行(1行1JSON)。完走マーカーは `=== 完了 ===`。

#![windows_subsystem = "windows"]
#![allow(unsafe_code)]

use std::io::Write as _;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::json;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, SendInput, SetFocus, UnregisterHotKey, HOT_KEY_MODIFIERS, INPUT, INPUT_0,
    INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, MOD_CONTROL, MOD_SHIFT,
    VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetForegroundWindow,
    GetGUIThreadInfo, GetMessageW, GetWindowThreadProcessId, PeekMessageW, PostMessageW,
    PostQuitMessage, RegisterClassExW, SendMessageW, SetForegroundWindow, ShowWindow,
    TranslateMessage, CW_USEDEFAULT, GUITHREADINFO, MSG, PM_REMOVE, SW_SHOW, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_APP, WM_CLOSE, WM_DESTROY, WM_HOTKEY, WM_SETTEXT, WNDCLASSEXW, WS_BORDER,
    WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE, WS_VSCROLL,
};

const WM_CV_FRONT: u32 = WM_APP + 1;
const ES_MULTILINE: u32 = 0x0004;
const ES_AUTOVSCROLL: u32 = 0x0040;
const EM_GETSEL: u32 = 0x00B0;
const EM_SETSEL: u32 = 0x00B1;
const EM_LINEFROMCHAR: u32 = 0x00C9;

const VK_LSHIFT: u32 = 0xA0;
const VK_LCONTROL: u32 = 0xA2;
const VK_UP: u32 = 0x26;
const VK_DOWN: u32 = 0x28;
const VK_P: u32 = 0x50;
const VK_N: u32 = 0x4E;
const VK_F10: u32 = 0x79;
const VK_F12: u32 = 0x7B;

/// スパイク/プローブと同じ目印(二重定義しない)。
const MARKER: usize = awase_windows::hook::TEST_INJECTION_MARKER;

static TOP: AtomicIsize = AtomicIsize::new(0);
static CHILD: AtomicIsize = AtomicIsize::new(0);
static LOG_PATH: OnceLock<String> = OnceLock::new();
/// 注入時の `dwExtraInfo`(`--marker=1` で目印、既定は0=素の SendInput)。
static EXTRA: OnceLock<usize> = OnceLock::new();

fn hwnd_of(v: &AtomicIsize) -> HWND {
    HWND(v.load(Ordering::SeqCst) as *mut core::ffi::c_void)
}

fn sleep_ms(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn log(line: &str) {
    let path = LOG_PATH
        .get()
        .map_or("config_verify_probe.log", String::as_str)
        .to_string();
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{line}");
    }
}

fn rec(v: &serde_json::Value) {
    log(&format!("[CV-JSON] {v}"));
}

fn arg_value(key: &str) -> Option<String> {
    std::env::args().find_map(|a| a.strip_prefix(key).map(str::to_string))
}

fn send_key(vk: u32, scan: u16, down: bool, extra: usize) -> bool {
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(u16::try_from(vk).unwrap_or(0)),
                wScan: scan,
                dwFlags: if down {
                    KEYBD_EVENT_FLAGS(0)
                } else {
                    KEYEVENTF_KEYUP
                },
                time: 0,
                dwExtraInfo: extra,
            },
        },
    };
    unsafe { SendInput(&[input], size_of::<INPUT>() as i32) == 1 }
}

/// 修飾キー(Ctrl と、あれば Shift)を押したまま主キーを押して離す。注入が1つでも落ちたら false。
fn chord(ctrl: bool, shift: bool, vk: u32, scan: u16, extra: usize) -> bool {
    let mut ok = true;
    if ctrl {
        ok &= send_key(VK_LCONTROL, 0x1D, true, extra);
        sleep_ms(30);
    }
    if shift {
        ok &= send_key(VK_LSHIFT, 0x2A, true, extra);
        sleep_ms(30);
    }
    ok &= send_key(vk, scan, true, extra);
    sleep_ms(60);
    ok &= send_key(vk, scan, false, extra);
    sleep_ms(30);
    if shift {
        ok &= send_key(VK_LSHIFT, 0x2A, false, extra);
        sleep_ms(30);
    }
    if ctrl {
        ok &= send_key(VK_LCONTROL, 0x1D, false, extra);
    }
    ok
}

// ---------------------------------------------------------------- hotkey モード

/// 指定時間、スレッドのメッセージキューを読んで `WM_HOTKEY` が届くかを見る。
fn poll_hotkey(id: i32, ms: u64) -> bool {
    let deadline = Instant::now() + Duration::from_millis(ms);
    let mut fired = false;
    while Instant::now() < deadline {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&raw mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_HOTKEY && msg.wParam.0 == id as usize {
                    fired = true;
                }
                let _ = TranslateMessage(&raw const msg);
                DispatchMessageW(&raw const msg);
            }
        }
        sleep_ms(20);
    }
    fired
}

fn run_hotkey(extra: usize) {
    // 対照: 自プロセスの Ctrl+Shift+F10。
    let registered = unsafe {
        RegisterHotKey(
            None,
            1,
            HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_SHIFT.0),
            VK_F10,
        )
        .is_ok()
    };
    // awase の hotkey 登録の前後で状態が落ち着くのを待つ。
    sleep_ms(500);
    let sent_ok = chord(true, true, VK_F10, 0x44, extra);
    let control_fired = poll_hotkey(1, 1500);
    rec(
        &json!({"type":"control_hotkey","registered":registered,"sent_ok":sent_ok,
                "fired":control_fired,"marker":extra != 0}),
    );
    // 対象: awase の Ctrl+Shift+F12(awase.log は呼び出し側が読む)。
    let sent_ok = chord(true, true, VK_F12, 0x58, extra);
    let _ = poll_hotkey(2, 1500);
    rec(&json!({"type":"awase_hotkey_injected","sent_ok":sent_ok,"marker":extra != 0}));
    unsafe {
        let _ = UnregisterHotKey(None, 1);
    }
    log("=== 完了 ===");
}

// ---------------------------------------------------------------- caret モード

unsafe extern "system" fn top_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_CV_FRONT => {
                front_and_focus(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

/// 前面化(前面スレッドへ入力をアタッチする定番の回避策、typing_stress と同じ)と入力欄へのフォーカス。
fn front_and_focus(top: HWND) {
    unsafe {
        let fg = GetForegroundWindow();
        let fg_tid = if fg.0.is_null() {
            0
        } else {
            GetWindowThreadProcessId(fg, None)
        };
        let my_tid = GetCurrentThreadId();
        let attached =
            fg_tid != 0 && fg_tid != my_tid && AttachThreadInput(my_tid, fg_tid, true).as_bool();
        let _ = BringWindowToTop(top);
        let _ = SetForegroundWindow(top);
        let child = hwnd_of(&CHILD);
        if !child.0.is_null() {
            let _ = SetFocus(Some(child));
        }
        if attached {
            let _ = AttachThreadInput(my_tid, fg_tid, false);
        }
    }
}

fn focus_ok() -> bool {
    unsafe {
        if GetForegroundWindow() != hwnd_of(&TOP) {
            return false;
        }
        let mut gi = GUITHREADINFO {
            cbSize: size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        GetGUIThreadInfo(0, &raw mut gi).is_ok() && gi.hwndFocus == hwnd_of(&CHILD)
    }
}

fn create_form() -> HWND {
    unsafe {
        let instance = GetModuleHandleW(None).expect("module");
        let top_w = wide("ConfigVerifyTop");
        let top_wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(top_proc),
            hInstance: instance.into(),
            lpszClassName: PCWSTR(top_w.as_ptr()),
            ..Default::default()
        };
        RegisterClassExW(&raw const top_wc);
        let top = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(top_w.as_ptr()),
            w!("config verify"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            760,
            360,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .expect("top window");
        let child = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("EDIT"),
            w!(""),
            WINDOW_STYLE(
                WS_CHILD.0
                    | WS_VISIBLE.0
                    | WS_BORDER.0
                    | ES_MULTILINE
                    | ES_AUTOVSCROLL
                    | WS_VSCROLL.0,
            ),
            10,
            10,
            700,
            240,
            Some(top),
            None,
            Some(instance.into()),
            None,
        )
        .unwrap_or_else(|e| {
            log(&format!("[FATAL] EDIT の作成に失敗: {e}"));
            std::process::exit(2);
        });
        TOP.store(top.0 as isize, Ordering::SeqCst);
        CHILD.store(child.0 as isize, Ordering::SeqCst);
        let _ = ShowWindow(top, SW_SHOW);
        let _ = SetFocus(Some(child));
        top
    }
}

/// キャレットの位置(文字オフセット)と行番号(0始まり)。
fn caret() -> (usize, usize) {
    unsafe {
        let child = hwnd_of(&CHILD);
        let mut start: u32 = 0;
        let mut end: u32 = 0;
        let _ = SendMessageW(
            child,
            EM_GETSEL,
            Some(WPARAM(&raw mut start as usize)),
            Some(LPARAM(&raw mut end as isize)),
        );
        let line = SendMessageW(child, EM_LINEFROMCHAR, Some(WPARAM(end as usize)), None).0;
        (end as usize, usize::try_from(line).unwrap_or(0))
    }
}

fn set_caret(pos: usize) {
    unsafe {
        let _ = SendMessageW(
            hwnd_of(&CHILD),
            EM_SETSEL,
            Some(WPARAM(pos)),
            Some(LPARAM(pos as isize)),
        );
    }
}

/// 3行の文字列 `aaaa\r\nbbbb\r\ncccc`(各行4文字+CRLF)。行 n の先頭オフセットは 6n、行内の4文字目の後は 6n+4。
const TEXT: &str = "aaaa\r\nbbbb\r\ncccc";

/// `label` の操作前にキャレットを `from_pos` に置き、`act` を実行して、操作後の行を記録する。
fn trial(label: &str, from_pos: usize, expect_line: usize, act: impl FnOnce() -> bool) {
    set_caret(from_pos);
    sleep_ms(150);
    let (pos_before, line_before) = caret();
    let focused = focus_ok();
    let sent_ok = act();
    sleep_ms(600);
    let (pos_after, line_after) = caret();
    rec(
        &json!({"type":"trial","label":label,"focused":focused,"sent_ok":sent_ok,
                "pos_before":pos_before,"line_before":line_before,
                "pos_after":pos_after,"line_after":line_after,"expect_line":expect_line,
                "moved":line_after != line_before,"as_expected":line_after == expect_line}),
    );
}

fn run_caret_worker() {
    // awase の focus 分類(フォーカス変更の検知)が落ち着くのを待ってから前面化し直す。
    sleep_ms(2500);
    unsafe {
        let _ = PostMessageW(Some(hwnd_of(&TOP)), WM_CV_FRONT, WPARAM(0), LPARAM(0));
    }
    sleep_ms(1500);
    let text = wide(TEXT);
    unsafe {
        let _ = SendMessageW(
            hwnd_of(&CHILD),
            WM_SETTEXT,
            None,
            Some(LPARAM(text.as_ptr() as isize)),
        );
    }
    sleep_ms(300);
    let ok = focus_ok();
    rec(&json!({"type":"focus","on_target":ok}));
    if !ok {
        // 前面化を数回やり直す(CI の揺らぎ)。
        for _ in 0..5 {
            unsafe {
                let _ = PostMessageW(Some(hwnd_of(&TOP)), WM_CV_FRONT, WPARAM(0), LPARAM(0));
            }
            sleep_ms(800);
            if focus_ok() {
                break;
            }
        }
        rec(&json!({"type":"focus_retry","on_target":focus_ok()}));
    }
    let x = *EXTRA.get().unwrap_or(&MARKER);
    // 位置: 行0の先頭=0、行1の先頭=6、行2の先頭=12。行2の4文字目の後=16。
    // 対照(ハーネス自体の確認): 素の Up/Down。awase の keymap は関与しない。
    trial("control_up", 16, 1, || chord(false, false, VK_UP, 0x48, x));
    trial("control_down", 4, 1, || {
        chord(false, false, VK_DOWN, 0x50, x)
    });
    // 本命: Ctrl+P → Up、Ctrl+N → Down。
    trial("ctrl_p_up", 16, 1, || chord(true, false, VK_P, 0x19, x));
    trial("ctrl_n_down", 4, 1, || chord(true, false, VK_N, 0x31, x));
    // 修飾キーが押されたままにならなかったか(Ctrl の押下状態を確認できないので、素の Up がまだ動くことで代用)。
    trial("after_up", 16, 1, || chord(false, false, VK_UP, 0x48, x));
    log("=== 完了 ===");
    unsafe {
        let _ = PostMessageW(Some(hwnd_of(&TOP)), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

fn main() {
    let log_path = arg_value("--log=").unwrap_or_else(|| "config_verify_probe.log".into());
    let _ = LOG_PATH.set(log_path.clone());
    let _ = std::fs::remove_file(&log_path);
    std::panic::set_hook(Box::new(|info| {
        log(&format!("[FATAL] panic: {info}"));
    }));
    let mode = arg_value("--mode=").unwrap_or_default();
    let extra = if arg_value("--marker=").as_deref() == Some("1") {
        MARKER
    } else if mode == "caret" {
        // caret は awase の keymap(物理キー扱いが必要)を試すので、既定で目印を付ける。
        MARKER
    } else {
        0
    };
    let _ = EXTRA.set(extra);
    match mode.as_str() {
        "hotkey" => run_hotkey(extra),
        "caret" => {
            let _top = create_form();
            std::thread::spawn(run_caret_worker);
            unsafe {
                let mut msg = MSG::default();
                while GetMessageW(&raw mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&raw const msg);
                    DispatchMessageW(&raw const msg);
                }
            }
        }
        other => {
            log(&format!("[FATAL] 引数エラー: --mode={other}"));
            std::process::exit(2);
        }
    }
}
