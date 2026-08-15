//! GJI charset 軸 (`CompositionMode*`) の F15-F19 バインド挙動を実機で検証する
//! 最小 IMM32 プローブ。ADR-091 (charset 軸の `CharsetSlot` 設計) の前提検証用、
//! 使い捨てツール。
//!
//! 自前のシンプルな Win32 ウィンドウ (`EDIT` コントロール) を作り、そこへ
//! `SendInput` で物理キー相当のイベントを注入し、`ImmGetConversionStatus` で
//! 実際の conv bits を直接読み取る。メモ帳を対象にした PowerShell 版
//! (clipwire 経由) は、Windows 11 のストアアプリ化によるウィンドウハンドル
//! 不整合と `SetForegroundWindow` のフォアグラウンド強奪制限で信頼できない
//! 結果しか返さなかったため、この専用プローブに切り替えた。
//!
//! `GetWindowTextW` で `EDIT` コントロールの内容を直接読めるため、
//! クリップボード往復（エンコーディング崩れ・他プロセスによる上書き）が
//! 一切不要になる点も PowerShell 版からの改善点。
//!
//! # 実行方法(Windows 実機のみ)
//!
//! ```powershell
//! cargo run -p awase-windows --example gji_composition_probe --release
//! ```
//!
//! 結果は標準出力と `gji_composition_probe_result.json`(カレントディレクトリ)
//! に書き出す。
//!
//! # 事前条件
//!
//! - GJI (Google 日本語入力) が既定 IME として選択されていること。
//! - `config1.db` の `custom_keymap_table` に F15-F19 が
//!   `CompositionModeHiragana`/`CompositionModeFullKatakana`/
//!   `CompositionModeHalfKatakana`/`CompositionModeFullAlphanumeric`/
//!   `CompositionModeHalfAlphanumeric` へバインド済みであること(手動、GJI
//!   プロパティのキー設定→編集→インポートで行う)。

#![cfg(windows)]
#![allow(unsafe_code)]

use std::time::{Duration, Instant};

use serde::Serialize;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::Ime::{
    ImmGetCompositionStringW, ImmGetContext, ImmGetConversionStatus, ImmGetOpenStatus,
    ImmReleaseContext, IME_COMPOSITION_STRING, IME_CONVERSION_MODE, IME_SENTENCE_MODE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, SendInput, SetFocus, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClassNameW,
    GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
    PeekMessageW, RegisterClassW, SetForegroundWindow, SetWindowTextW, ShowWindow,
    TranslateMessage, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, MSG, PM_REMOVE, SW_SHOW, WM_DESTROY,
    WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

const WINDOW_CLASS_NAME: &str = "gji_composition_probe_window";

/// `EDIT` コントロール(組み込みクラス)の class-specific style。
/// `windows` crate は `ES_*` を独立定数として持たないため、MSDN の raw 値を使う。
const ES_MULTILINE: u32 = 0x0004;
const ES_AUTOVSCROLL: u32 = 0x0040;
const WS_CHILD: u32 = 0x4000_0000;
const WS_BORDER: u32 = 0x0080_0000;

/// VK_F15..VK_F19 は標準ファンクションキー(ADR-057 が「物理キーが存在しない
/// 安全域」と確認した範囲)。ADR-091 の GJI charset 軸設計で使う5モード対応。
const KEY_MODE_TABLE: &[(&str, u16, &str, &str)] = &[
    ("F15", 0x7E, "CompositionModeHiragana", "ひらがな"),
    ("F16", 0x7F, "CompositionModeFullKatakana", "全角カタカナ"),
    ("F17", 0x80, "CompositionModeHalfKatakana", "半角カタカナ"),
    ("F18", 0x81, "CompositionModeFullAlphanumeric", "全角英数"),
    ("F19", 0x82, "CompositionModeHalfAlphanumeric", "半角英数"),
];

const VK_SPACE: u16 = 0x20;
const VK_RETURN: u16 = 0x0D;
/// IME を明示的に開く単一効果・冪等キー(ADR-067、config1.dbバインド不要で
/// 全環境で動作確認済み)。このプローブ用ウィンドウは新規作成直後は IME が
/// 閉じている可能性が高く、閉じたままでは打鍵がすべて素通しのASCIIになり
/// F15-F19 の効果を観測できない。
const VK_IME_ON: u16 = 0x16;

#[derive(Debug, Clone, Serialize)]
struct ConvState {
    open_status: Option<bool>,
    conversion_mode: Option<u32>,
    sentence_mode: Option<u32>,
    /// `ImmGetCompositionStringW(GCS_COMPSTR)`: 現在 composition 中(未確定)の文字列。
    comp_str: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ScenarioResult {
    scenario: String,
    key: String,
    mode_command: String,
    mode_label: String,
    edit_text: String,
    conv: ConvState,
}

#[derive(Debug, Clone, Serialize)]
struct ControlResult {
    pass: bool,
    expected: String,
    actual: String,
}

#[derive(Debug, Serialize)]
struct ProbeReport {
    control: ControlResult,
    scenarios: Vec<ScenarioResult>,
}

extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: DefWindowProcW はどんな (hwnd, msg, wparam, lparam) の組でも安全に
    // 呼べる Win32 の既定ハンドラ。WM_DESTROY だけ明示的に処理する必要はない
    // (このプローブはメッセージループを自前で終了させるため PostQuitMessage は使わない)。
    if msg == WM_DESTROY {
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// # Safety
/// メインスレッドから一度だけ呼ぶこと。
unsafe fn create_probe_windows() -> anyhow::Result<(HWND, HWND)> {
    let class_name_wide = to_wide(WINDOW_CLASS_NAME);
    let hinstance = windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
        .unwrap_or_default()
        .into();

    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wnd_proc),
        hInstance: hinstance,
        lpszClassName: PCWSTR(class_name_wide.as_ptr()),
        ..Default::default()
    };
    let atom = unsafe { RegisterClassW(&raw const wc) };
    anyhow::ensure!(atom != 0, "RegisterClassW failed for parent window");

    let title = to_wide("GJI Composition Probe");
    let parent = unsafe {
        CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
            PCWSTR(class_name_wide.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            600,
            300,
            None,
            None,
            Some(hinstance),
            None,
        )
    }
    .map_err(|e| anyhow::anyhow!("CreateWindowExW (parent) failed: {e}"))?;

    let edit_class = to_wide("EDIT");
    let edit_style = windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(
        WS_CHILD | WS_VISIBLE.0 | WS_BORDER | ES_MULTILINE | ES_AUTOVSCROLL,
    );
    let edit = unsafe {
        CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
            PCWSTR(edit_class.as_ptr()),
            PCWSTR::null(),
            edit_style,
            0,
            0,
            580,
            260,
            Some(parent),
            None,
            Some(hinstance),
            None,
        )
    }
    .map_err(|e| anyhow::anyhow!("CreateWindowExW (edit) failed: {e}"))?;

    Ok((parent, edit))
}

/// 現在フォアグラウンドのウィンドウクラス名を返す(診断用)。
fn get_foreground_class_name() -> String {
    let hwnd = unsafe { GetForegroundWindow() };
    let mut buf = [0u16; 256];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..usize::try_from(len).unwrap_or(0)])
}

/// バックグラウンドから起動した場合の `SetForegroundWindow` 制限
/// (anti-focus-stealing) を `AttachThreadInput` で回避してからフォーカスを奪う。
///
/// # Safety
/// `hwnd` は有効なウィンドウハンドルであること。
unsafe fn force_foreground(hwnd: HWND) {
    let fg = unsafe { GetForegroundWindow() };
    let mut fg_thread_pid = 0u32;
    let fg_thread_id = unsafe { GetWindowThreadProcessId(fg, Some(&raw mut fg_thread_pid)) };
    let my_thread_id = unsafe { GetCurrentThreadId() };

    let attached = if fg_thread_id != 0 && fg_thread_id != my_thread_id {
        unsafe { AttachThreadInput(my_thread_id, fg_thread_id, true) }.as_bool()
    } else {
        false
    };

    let _ = unsafe { ShowWindow(hwnd, SW_SHOW) };
    let _ = unsafe { SetForegroundWindow(hwnd) };

    if attached {
        let _ = unsafe { AttachThreadInput(my_thread_id, fg_thread_id, false) };
    }
}

/// # Safety
/// SendInput はプロセス全体に影響する。テスト目的でのみ呼ぶこと。
unsafe fn send_vk(vk: u16, keyup: bool) {
    let flags = if keyup {
        KEYEVENTF_KEYUP
    } else {
        KEYBD_EVENT_FLAGS(0)
    };
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let size = i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32");
    unsafe { SendInput(&[input], size) };
}

unsafe fn send_vk_tap(vk: u16) {
    unsafe {
        send_vk(vk, false);
        send_vk(vk, true);
    }
}

fn send_ascii_tap(c: char) {
    let upper = c.to_ascii_uppercase();
    let vk = match upper {
        'A'..='Z' | '0'..='9' => u16::from(upper as u8),
        ' ' => VK_SPACE,
        _ => return,
    };
    unsafe { send_vk_tap(vk) };
}

fn pump_messages(duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        let mut msg = MSG::default();
        // SAFETY: msg はスタック上の有効な MSG バッファ。hwnd=None で
        // 呼び出しスレッドの全ウィンドウ分のメッセージを対象にする。
        while unsafe { PeekMessageW(&raw mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            let _ = unsafe { TranslateMessage(&raw const msg) };
            unsafe { DispatchMessageW(&raw const msg) };
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn get_edit_text(hwnd: HWND) -> String {
    // SAFETY: hwnd は create_probe_windows で作成した有効な EDIT コントロール。
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; usize::try_from(len).unwrap_or(0) + 1];
    let written = unsafe { GetWindowTextW(hwnd, &mut buf) };
    let written = usize::try_from(written).unwrap_or(0);
    String::from_utf16_lossy(&buf[..written])
}

fn clear_edit_text(hwnd: HWND) {
    let empty = to_wide("");
    // SAFETY: hwnd は有効な EDIT コントロール。empty は NUL 終端済み。
    let _ = unsafe { SetWindowTextW(hwnd, PCWSTR(empty.as_ptr())) };
}

/// `ImmGetCompositionStringW` で GCS_COMPSTR (未確定 composition 文字列) を読む。
///
/// # Safety
/// `himc` は `ImmGetContext` で得た有効な HIMC であること。
unsafe fn read_comp_str(himc: windows::Win32::UI::Input::Ime::HIMC) -> Option<String> {
    const GCS_COMPSTR: u32 = 0x0008;
    // SAFETY: lpBuf=None かつ dwBufLen=0 で呼んでバイト長を取得する公式パターン。
    let byte_len =
        unsafe { ImmGetCompositionStringW(himc, IME_COMPOSITION_STRING(GCS_COMPSTR), None, 0) };
    if byte_len < 0 {
        return None;
    }
    let byte_len = usize::try_from(byte_len).unwrap_or(0);
    if byte_len == 0 {
        return Some(String::new());
    }
    let mut buf = vec![0u16; byte_len.div_ceil(2)];
    // SAFETY: buf は十分なサイズを確保済み。WCHAR バッファとして書き込まれる。
    let written = unsafe {
        ImmGetCompositionStringW(
            himc,
            IME_COMPOSITION_STRING(GCS_COMPSTR),
            Some(buf.as_mut_ptr().cast()),
            u32::try_from(buf.len() * 2).unwrap_or(0),
        )
    };
    if written <= 0 {
        return None;
    }
    let char_count = usize::try_from(written).unwrap_or(0) / 2;
    Some(String::from_utf16_lossy(&buf[..char_count]))
}

fn read_conv_state(hwnd: HWND) -> ConvState {
    // SAFETY: hwnd は create_probe_windows で作成した有効なウィンドウ。
    let himc = unsafe { ImmGetContext(hwnd) };
    if himc.is_invalid() {
        return ConvState {
            open_status: None,
            conversion_mode: None,
            sentence_mode: None,
            comp_str: None,
        };
    }

    // SAFETY: himc は有効。ImmGetOpenStatus はクラッシュしない読み取り API。
    let open_status = Some(unsafe { ImmGetOpenStatus(himc) }.as_bool());

    let mut conv = IME_CONVERSION_MODE::default();
    let mut sent = IME_SENTENCE_MODE::default();
    // SAFETY: himc は有効。書き込み先は両方 null でない (&raw mut)。
    let ok = unsafe { ImmGetConversionStatus(himc, Some(&raw mut conv), Some(&raw mut sent)) };
    let (conversion_mode, sentence_mode) = if ok.as_bool() {
        (Some(conv.0), Some(sent.0))
    } else {
        (None, None)
    };

    // SAFETY: himc は有効。
    let comp_str = unsafe { read_comp_str(himc) };

    // SAFETY: hwnd/himc は対応する有効なペア。
    let _ = unsafe { ImmReleaseContext(hwnd, himc) };

    ConvState {
        open_status,
        conversion_mode,
        sentence_mode,
        comp_str,
    }
}

fn run_control_test(edit: HWND) -> ControlResult {
    clear_edit_text(edit);
    for c in "test123".chars() {
        send_ascii_tap(c);
    }
    pump_messages(Duration::from_millis(300));
    let actual = get_edit_text(edit);
    ControlResult {
        pass: actual == "test123",
        expected: "test123".to_string(),
        actual,
    }
}

fn run_idle_scenario(edit: HWND, vk: u16) -> (String, ConvState) {
    clear_edit_text(edit);
    unsafe { send_vk_tap(vk) };
    pump_messages(Duration::from_millis(200));
    send_ascii_tap('a');
    pump_messages(Duration::from_millis(200));
    (get_edit_text(edit), read_conv_state(edit))
}

fn run_pending_scenario(edit: HWND, vk: u16) -> (String, ConvState) {
    clear_edit_text(edit);
    send_ascii_tap('a');
    send_ascii_tap('i');
    pump_messages(Duration::from_millis(200));
    unsafe { send_vk_tap(vk) };
    pump_messages(Duration::from_millis(200));
    (get_edit_text(edit), read_conv_state(edit))
}

fn run_henkan_scenario(edit: HWND, vk: u16) -> (String, ConvState) {
    clear_edit_text(edit);
    send_ascii_tap('a');
    send_ascii_tap('i');
    pump_messages(Duration::from_millis(200));
    send_ascii_tap(' ');
    pump_messages(Duration::from_millis(300));
    unsafe { send_vk_tap(vk) };
    pump_messages(Duration::from_millis(200));
    let state = read_conv_state(edit);
    unsafe { send_vk_tap(VK_RETURN) };
    pump_messages(Duration::from_millis(200));
    (get_edit_text(edit), state)
}

fn main() -> anyhow::Result<()> {
    // SAFETY: メインスレッドから一度だけウィンドウを作成する。
    let (parent, edit) = unsafe { create_probe_windows() }?;
    // SAFETY: parent は直前に作成した有効なウィンドウ。
    unsafe { force_foreground(parent) };
    pump_messages(Duration::from_millis(300));

    // SAFETY: edit は直前に作成した有効な子ウィンドウ。
    let _ = unsafe { SetFocus(Some(edit)) };
    pump_messages(Duration::from_millis(200));

    // コントロールテストは IME を開く**前**に行う。IME が開いた状態で
    // "test123" を送ると、GJI がローマ字入力として変換を試み始め、未確定の
    // composition 文字列（IME 側で保持、GetWindowTextW では読めない）に
    // なってしまい、EDIT コントロールの実テキストには何も入らない
    // （このプローブ自身の実装バグとして実機で確認済み: open_status=true
    // の状態で送ると毎回 actual='' になっていた）。IME 閉状態でのプレーン
    // ASCII 素通しを確認してから、その後で IME を開いて F15-F19 の本番
    // シナリオへ進む。
    let control = run_control_test(edit);
    println!(
        "control: pass={} expected='{}' actual='{}'",
        control.pass, control.expected, control.actual
    );

    // 新規作成直後のウィンドウは IME が閉じている可能性が高い。VK_IME_ON
    // (単一効果・冪等、config1.dbバインド不要、ADR-067) で明示的に開く。
    let pre_open = read_conv_state(edit).open_status;
    if pre_open != Some(true) {
        unsafe { send_vk_tap(VK_IME_ON) };
        // TSF composition context の初期化は実測で ~100-300ms かかることが
        // ある(known-bugs.md BUG-02 系)。ここでは安全側に長めに待つ。
        pump_messages(Duration::from_millis(800));
    }
    let post_open = read_conv_state(edit).open_status;
    println!("ime open_status: before={pre_open:?} after_VK_IME_ON={post_open:?}");

    let mut scenarios = Vec::new();

    if control.pass && post_open == Some(true) {
        for (key, vk, command, label) in KEY_MODE_TABLE {
            let (text, conv) = run_idle_scenario(edit, *vk);
            println!(
                "idle    {key} ({label}) text='{text}' conv_mode={:?} comp_str={:?}",
                conv.conversion_mode, conv.comp_str
            );
            scenarios.push(ScenarioResult {
                scenario: "idle".to_string(),
                key: (*key).to_string(),
                mode_command: (*command).to_string(),
                mode_label: (*label).to_string(),
                edit_text: text,
                conv,
            });

            let (text, conv) = run_pending_scenario(edit, *vk);
            println!(
                "pending {key} ({label}) text='{text}' conv_mode={:?} comp_str={:?}",
                conv.conversion_mode, conv.comp_str
            );
            scenarios.push(ScenarioResult {
                scenario: "pending".to_string(),
                key: (*key).to_string(),
                mode_command: (*command).to_string(),
                mode_label: (*label).to_string(),
                edit_text: text,
                conv,
            });

            let (text, conv) = run_henkan_scenario(edit, *vk);
            println!(
                "henkan  {key} ({label}) text='{text}' conv_mode={:?} comp_str={:?}",
                conv.conversion_mode, conv.comp_str
            );
            scenarios.push(ScenarioResult {
                scenario: "henkan".to_string(),
                key: (*key).to_string(),
                mode_command: (*command).to_string(),
                mode_label: (*label).to_string(),
                edit_text: text,
                conv,
            });
        }
    } else if !control.pass {
        println!("control test failed - skipping F15-F19 scenarios (SendInput likely not reaching this window)");
    } else {
        println!("IME did not open (VK_IME_ON had no effect) - skipping F15-F19 scenarios");
    }

    let report = ProbeReport { control, scenarios };
    let json = serde_json::to_string_pretty(&report)?;
    std::fs::write("gji_composition_probe_result.json", &json)?;
    println!("result written: gji_composition_probe_result.json");

    // SAFETY: parent は有効なウィンドウ。プローブ終了時に破棄する。
    let _ = unsafe { DestroyWindow(parent) };

    Ok(())
}
