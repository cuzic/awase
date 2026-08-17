//! Alt キー押下中に `VK_DBE_HIRAGANA` (F2) を SendInput したらどうなるかを
//! 実機で検証する診断ツール。仮説検証用、使い捨てツール。
//!
//! # 仮説
//!
//! awase 自身の F2 warmup（`tsf/send.rs::send_vk_dbe_hiragana_pair` 等、
//! `VK_DBE_HIRAGANA` を SendInput で送る箇所）が、ユーザーが偶然 Alt を
//! 押しているタイミングと重なると、物理 Alt+かな（BUG-61/62 で確認済みの
//! MS-IME「ローマ字入力 ⇔ JIS かな直接入力」切替ショートカット）と同様に
//! 解釈され、JIS かな直接入力へ切り替わってしまうのではないか、というもの。
//!
//! BUG-61 の実機調査で、いったん JIS かな側へ切り替わると
//! `ImmSetConversionStatus`（IMC write）・`VK_DBE_ROMAN` 注入のどちらでも
//! 復旧不能と確定済み。本ツールは合成入力の SendInput 直後に
//! `ImmGetConversionStatus` の ROMAN ビットを読み直すだけで、実際に awase の
//! 打鍵出力へ影響が及ぶ前に検出できる。
//!
//! `hook.rs` の既存 Alt+かなガード（`VK_KANA`/`VK_DBE_ROMAN`/`VK_DBE_NOROMAN`
//! 対象）は自己注入キー（`is_self_injected`）を無条件に OS へ通しているため、
//! awase 自身の F2 warmup はこのガードの対象外になっている——本ツールはこの
//! 抜け穴を実機で確認するためのもの。
//!
//! # 実観測アプリの落とし穴（v2 で対処）
//!
//! v1 では `GetForegroundWindow()` に対して直接 `ImmGetContext` していたが、
//! Windows Terminal（TSF ネイティブ）・Chrome/Teams（Imm32Unavailable、自前の
//! フェイクウィンドウで IMM32 を隠す）では常に無効な HIMC が返り、
//! before/after が両方 null（＝未計測）になるだけで何も検証できなかった
//! （皮肉にも、この3つは awase の F2 warmup が実際によく使われるカテゴリ
//! そのもの）。`gji_composition_probe.rs` が最初から自前の classic EDIT
//! コントロールを使っている理由と同じ問題に当たった。
//!
//! v2 では本ツール自身が classic Win32 `EDIT` コントロールを持つウィンドウを
//! 作り、起動時にフォアグラウンド化・フォーカスする。**このウィンドウに
//! フォーカスしたまま Alt キーを押す**ことで、確実に IMM32 が読める状態で
//! 検証する（Alt を単独でタップするだけならフォーカス自体は移動しない —
//! Alt+Tab とは違う）。
//!
//! # ビット解釈と体感の食い違い（v3 で対処）
//!
//! v2 の実測で `roman` ビットの解釈とユーザーの実際のタイプ体感が食い違う
//! 場面があった。数値ビットだけに頼らず、Alt+F2 注入の直後に実際に "aiu" を
//! 打鍵し、`ImmGetCompositionStringW` で変換結果を直接確認する
//! （`typed_comp_str`）。ローマ字入力なら「あいう」になるはずで、JIS かな
//! 直接入力なら物理 A/I/U キー位置のローマ字と無関係な別のかなが出るはず。
//!
//! # 使い方（Windows 実機のみ）
//!
//! ```powershell
//! cargo run -p awase-windows --example alt_dbe_hiragana_probe --release
//! ```
//!
//! 起動すると小さなウィンドウが前面に出てフォーカスされる。**そのウィンドウに
//! フォーカスしたまま Alt キーを押す**（Alt+Tab はしないこと——別ウィンドウへ
//! フォーカスが移ると計測できなくなる）。押した瞬間（立ち上がりエッジ）を
//! 検知し、自動で `VK_DBE_HIRAGANA` の down+up を SendInput する
//! （`crate::tsf::output::make_scan_key_input` と同じ方式: `wVk` を明示し
//! つつ `MapVirtualKeyW` で得た scan code も付与、`KEYEVENTF_SCANCODE` は
//! 使わない）。送信前後で `ImmGetConversionStatus`/`ImmGetOpenStatus` を
//! 読み比較し、ROMAN ビットが落ちていれば（≒ JIS かなへの切替が起きた）
//! 警告を出す。`foreground_class` が想定外の値（例: エクスプローラのタスク
//! バー等）になっていたら、Alt 単独タップのはずが `SC_KEYMENU` 相当の
//! フォーカス奪取が起きた兆候として扱える。
//!
//! Alt を離すまで再武装しない（1回の押下につき1回だけ発火、連打防止）。
//! **F9** キーで Alt 抜きのベースライン計測（比較用）も随時実行できる。
//! Esc または Ctrl+C で終了。
//!
//! ログは標準出力に加え、カレントディレクトリの
//! `alt_dbe_hiragana_probe_log.jsonl`（1行1イベントの追記式 JSON Lines）
//! にも書き出す。

#![cfg(windows)]
#![allow(unsafe_code)]

use std::fs::OpenOptions;
use std::io::Write as _;
use std::time::{Duration, Instant};

use serde::Serialize;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::Ime::{
    HIMC, IME_COMPOSITION_STRING, IME_CONVERSION_MODE, IME_SENTENCE_MODE, ImmGetCompositionStringW,
    ImmGetContext, ImmGetConversionStatus, ImmGetOpenStatus, ImmReleaseContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, MapVirtualKeyW, SendInput, SetFocus, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, GetClassNameW, GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, MSG, PM_REMOVE, PeekMessageW, RegisterClassW, SW_SHOW,
    SetForegroundWindow, SetWindowTextW, ShowWindow, TranslateMessage, WM_DESTROY, WNDCLASSW,
    WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};
use windows::core::PCWSTR;

const WINDOW_CLASS_NAME: &str = "alt_dbe_hiragana_probe_window";

/// `EDIT` コントロール(組み込みクラス)の class-specific style。
/// `gji_composition_probe.rs` と同じく `windows` crate に独立定数が無いため
/// MSDN の raw 値を使う。
const ES_MULTILINE: u32 = 0x0004;
const ES_AUTOVSCROLL: u32 = 0x0040;
const WS_CHILD: u32 = 0x4000_0000;
const WS_BORDER: u32 = 0x0080_0000;

/// `VK_DBE_HIRAGANA` — かな入力キー本来の VK（F2 warmup、`vk.rs` と同じ値）。
const VK_DBE_HIRAGANA: u16 = 0xF2;
/// IME を明示的に開く単一効果・冪等キー（`gji_composition_probe.rs` と同じ、
/// ADR-067）。新規作成直後のウィンドウは IME が閉じている可能性が高い。
const VK_IME_ON: u16 = 0x16;
/// `VK_MENU` — 左右どちらの Alt でも共通で立つ汎用コード
/// （`WH_KEYBOARD_LL` と異なり `GetAsyncKeyState` はこの区別をしない）。
const VK_MENU: i32 = 0x12;
const VK_ESCAPE: i32 = 0x1B;
const VK_F9: i32 = 0x78;

const IME_CMODE_NATIVE: u32 = 0x0001;
const IME_CMODE_ROMAN: u32 = 0x0010;

const LOG_PATH: &str = "alt_dbe_hiragana_probe_log.jsonl";

#[derive(Debug, Clone, Serialize)]
struct ConvSnapshot {
    open_status: Option<bool>,
    conversion_mode_raw: Option<u32>,
    /// `conversion_mode_raw` の NATIVE ビットが立っているか（true=かな系, false=英数）。
    native: Option<bool>,
    /// `conversion_mode_raw` の ROMAN ビットが立っているか
    /// （true=ローマ字入力, false=JIS かな直接入力）。
    roman: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
struct ProbeEvent {
    seq: u32,
    trigger: &'static str,
    elapsed_ms: u128,
    foreground_class: String,
    before: ConvSnapshot,
    after: ConvSnapshot,
    /// `before.roman == Some(true) && after.roman == Some(false)`。
    /// true なら「ローマ字入力 → JIS かな直接入力」への切替を実際に検出した。
    roman_dropped: bool,
    /// `before.open_status == Some(true) && after.open_status == Some(false)`。
    ime_closed: bool,
    /// "aiu" を実際に打鍵したときの未確定 composition 文字列
    /// （`GCS_COMPSTR`）。ローマ字入力なら「あいう」になるはず。
    typed_comp_str: String,
    /// 同じ打鍵後の EDIT コントロールの確定テキスト。IME が閉じていれば
    /// 素の "aiu" がここに入る。
    typed_edit_text: String,
}

fn read_conv_snapshot(hwnd: HWND) -> ConvSnapshot {
    // SAFETY: hwnd は GetForegroundWindow が返した有効なハンドル（NULL は
    // 呼び出し元でチェック済み）。ImmGetContext は失敗時 invalid HIMC を返す
    // だけでクラッシュしない。
    let himc = unsafe { ImmGetContext(hwnd) };
    if himc.is_invalid() {
        return ConvSnapshot {
            open_status: None,
            conversion_mode_raw: None,
            native: None,
            roman: None,
        };
    }
    // SAFETY: himc は直前に取得した有効なハンドル。
    let open_status = Some(unsafe { ImmGetOpenStatus(himc) }.as_bool());

    let mut conv = IME_CONVERSION_MODE::default();
    let mut sent = IME_SENTENCE_MODE::default();
    // SAFETY: himc は有効。書き込み先は両方スタック上の有効な変数。
    let ok = unsafe { ImmGetConversionStatus(himc, Some(&raw mut conv), Some(&raw mut sent)) };
    let (conversion_mode_raw, native, roman) = if ok.as_bool() {
        (
            Some(conv.0),
            Some(conv.0 & IME_CMODE_NATIVE != 0),
            Some(conv.0 & IME_CMODE_ROMAN != 0),
        )
    } else {
        (None, None, None)
    };

    // SAFETY: hwnd/himc は対応する有効なペア。
    let _ = unsafe { ImmReleaseContext(hwnd, himc) };

    ConvSnapshot {
        open_status,
        conversion_mode_raw,
        native,
        roman,
    }
}

/// 現在のフォアグラウンドウィンドウのクラス名（診断用）。想定は常に本ツール
/// 自身のウィンドウ（`WINDOW_CLASS_NAME`）——それ以外が出たら、Alt 単独タップ
/// のはずが `SC_KEYMENU`/タスクバー等へフォーカスが奪われた兆候。
fn foreground_class_name() -> String {
    // SAFETY: GetForegroundWindow は常に安全（NULL の可能性はあるが、その場合
    // GetClassNameW 側で len<=0 になるだけ）。
    let hwnd = unsafe { GetForegroundWindow() };
    let mut buf = [0u16; 256];
    // SAFETY: hwnd が NULL でも GetClassNameW は 0 を返すだけで安全。buf は
    // スタック上の十分なサイズのバッファ。
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len <= 0 {
        String::new()
    } else {
        String::from_utf16_lossy(&buf[..usize::try_from(len).unwrap_or(0)])
    }
}

extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: DefWindowProcW はどんな (hwnd, msg, wparam, lparam) の組でも
    // 安全に呼べる Win32 の既定ハンドラ（`gji_composition_probe.rs` と同じ）。
    if msg == WM_DESTROY {
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// classic Win32 `EDIT` コントロールを持つ最小ウィンドウを作る
/// （`gji_composition_probe.rs::create_probe_windows` と同じ構成）。
/// このウィンドウなら IMM32 が確実に読める（TSF ネイティブ/Imm32Unavailable
/// アプリでの計測失敗を回避するのが目的）。
///
/// # Safety
/// メインスレッドから一度だけ呼ぶこと。
unsafe fn create_probe_window() -> anyhow::Result<(HWND, HWND)> {
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

    let title =
        to_wide("Alt+VK_DBE_HIRAGANA Probe — このウィンドウにフォーカスしたまま Alt を押す");
    let parent = unsafe {
        CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
            PCWSTR(class_name_wide.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            640,
            240,
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
            620,
            200,
            Some(parent),
            None,
            Some(hinstance),
            None,
        )
    }
    .map_err(|e| anyhow::anyhow!("CreateWindowExW (edit) failed: {e}"))?;

    Ok((parent, edit))
}

/// バックグラウンドから起動した場合の `SetForegroundWindow` 制限
/// (anti-focus-stealing) を `AttachThreadInput` で回避してからフォーカスを奪う
/// （`gji_composition_probe.rs::force_foreground` と同じ）。
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
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// `crate::tsf::output::make_scan_key_input` と同じ方式で INPUT を組み立てる:
/// `wVk` を明示しつつ `MapVirtualKeyW(VK_TO_VSC)` の scan code も併記し、
/// `KEYEVENTF_SCANCODE` フラグは立てない（= wVk が正、wScan は付随情報）。
/// awase の実際の F2 warmup 送信を忠実に再現することが本プローブの前提。
fn make_dbe_hiragana_input(is_keyup: bool) -> INPUT {
    // SAFETY: MapVirtualKeyW はスレッドセーフで、0xF2 は有効な仮想キー値。
    let scan = unsafe { MapVirtualKeyW(u32::from(VK_DBE_HIRAGANA), MAPVK_VK_TO_VSC) as u16 };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(VK_DBE_HIRAGANA),
                wScan: scan,
                dwFlags: if is_keyup {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// # Safety
/// SendInput はプロセス全体（フォアグラウンドウィンドウ）に影響する。
/// 診断目的でのみ呼ぶこと。
unsafe fn send_dbe_hiragana_pair() {
    let inputs = [
        make_dbe_hiragana_input(false),
        make_dbe_hiragana_input(true),
    ];
    let size = i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32");
    // SAFETY: inputs は有効な INPUT 配列2件、size は size_of::<INPUT> と一致。
    unsafe {
        SendInput(&inputs, size);
    }
}

/// 汎用の VK down+up 送信（起動時の `VK_IME_ON` 用、scan code なし）。
///
/// # Safety
/// SendInput はプロセス全体に影響する。診断目的でのみ呼ぶこと。
unsafe fn send_vk_tap(vk: u16) {
    let make = |is_keyup: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: if is_keyup {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [make(false), make(true)];
    let size = i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32");
    // SAFETY: inputs は有効な INPUT 配列2件、size は size_of::<INPUT> と一致。
    unsafe {
        SendInput(&inputs, size);
    }
}

/// "aiu" を打鍵したとき QWERTY の A/I/U キー位置に来る VK コード。
const ROMAJI_PROBE_VKS: [u16; 3] = [0x41, 0x49, 0x55]; // 'A', 'I', 'U'

/// # Safety
/// SendInput はプロセス全体に影響する。診断目的でのみ呼ぶこと。
unsafe fn send_romaji_probe_string() {
    for &vk in &ROMAJI_PROBE_VKS {
        // SAFETY: 呼び出し元の契約と同じ。
        unsafe { send_vk_tap(vk) };
    }
}

/// `ImmGetCompositionStringW` で GCS_COMPSTR (未確定 composition 文字列) を読む
/// （`gji_composition_probe.rs::read_comp_str` と同じ）。
///
/// # Safety
/// `himc` は `ImmGetContext` で得た有効な HIMC であること。
unsafe fn read_comp_str(himc: HIMC) -> Option<String> {
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

fn get_edit_text(hwnd: HWND) -> String {
    // SAFETY: hwnd は create_probe_window で作成した有効な EDIT コントロール。
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

/// Alt+F2 注入の直後に "aiu" を打鍵し、実際にどう変換されたかを直接確認する。
///
/// - ローマ字入力なら "aiu" → 未確定 composition 「あいう」になるはず。
/// - JIS かな直接入力なら、物理 A/I/U キー位置に割り当てられた**ローマ字と
///   無関係な別のかな**が出るはず（人が見て一目で分かる）。
/// - IME が閉じていれば素の "aiu"（ASCII）がそのまま edit テキストに入る。
///
/// 数値ビットの解釈だけに頼らない、挙動そのものの直接証拠を得るための関数
/// （前回の実測でユーザー体感と `roman` ビットの解釈が食い違ったため追加）。
fn probe_romaji_typing(edit: HWND) -> (String, String) {
    clear_edit_text(edit);
    // SAFETY: 診断目的の単発呼び出し。
    unsafe { send_romaji_probe_string() };
    pump_messages(Duration::from_millis(300));

    // SAFETY: edit は有効なウィンドウ。
    let himc = unsafe { ImmGetContext(edit) };
    let comp_str = if himc.is_invalid() {
        None
    } else {
        // SAFETY: himc は直前に取得した有効なハンドル。
        let s = unsafe { read_comp_str(himc) };
        // SAFETY: edit/himc は対応する有効なペア。
        let _ = unsafe { ImmReleaseContext(edit, himc) };
        s
    };
    let edit_text = get_edit_text(edit);

    // 後始末: 未確定 composition が残ると次回計測に影響するため Escape で破棄し、
    // 入力済みテキストもクリアする。
    // SAFETY: 診断目的の単発呼び出し。
    unsafe { send_vk_tap(0x1B) }; // VK_ESCAPE
    pump_messages(Duration::from_millis(100));
    clear_edit_text(edit);

    (comp_str.unwrap_or_default(), edit_text)
}

fn key_down(vk: i32) -> bool {
    // SAFETY: GetAsyncKeyState は任意の vk コードに対して安全に呼べる。
    (unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000) != 0
}

fn append_log(event: &ProbeEvent) {
    let Ok(line) = serde_json::to_string(event) else {
        return;
    };
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(LOG_PATH) {
        let _ = writeln!(file, "{line}");
    }
}

fn run_probe(seq: u32, trigger: &'static str, start: Instant, edit: HWND) -> ProbeEvent {
    let foreground_class = foreground_class_name();
    let before = read_conv_snapshot(edit);
    // SAFETY: 診断ツールの唯一の呼び出し経路。フォーカス中の（=本ツールの
    // EDIT コントロールである想定の）ウィンドウへ VK_DBE_HIRAGANA down+up を
    // 送るだけ。
    unsafe { send_dbe_hiragana_pair() };
    // IME 側の反映を待つ（ImmSetConversionStatus は同期 API だが、TSF 経由の
    // 反映は非同期になりうるため実測で使われている待機幅に合わせて余裕を見る）。
    pump_messages(Duration::from_millis(150));
    let after = read_conv_snapshot(edit);

    let (typed_comp_str, typed_edit_text) = probe_romaji_typing(edit);

    let roman_dropped = before.roman == Some(true) && after.roman == Some(false);
    let ime_closed = before.open_status == Some(true) && after.open_status == Some(false);
    // before/after のどちらかで ImmGetContext 自体が失敗した（= 計測不能。
    // 「変化なし」と紛らわしいので明示的に区別する）。
    let read_failed = before.roman.is_none() || after.roman.is_none();
    // "aiu" が正しく「あいう」（ローマ字→ひらがな変換）になったか。
    let typed_as_expected = typed_comp_str == "あいう";

    let event = ProbeEvent {
        seq,
        trigger,
        elapsed_ms: start.elapsed().as_millis(),
        foreground_class,
        before,
        after,
        roman_dropped,
        ime_closed,
        typed_comp_str,
        typed_edit_text,
    };

    println!(
        "[{seq:>4}] trigger={trigger:<8} fg={:<28} before(open={:?} native={:?} roman={:?}) \
         → after(open={:?} native={:?} roman={:?}) typed comp_str={:?} edit_text={:?}{}{}{}{}",
        event.foreground_class,
        event.before.open_status,
        event.before.native,
        event.before.roman,
        event.after.open_status,
        event.after.native,
        event.after.roman,
        event.typed_comp_str,
        event.typed_edit_text,
        if read_failed {
            "  !!! 計測失敗（ImmGetContext が無効な HIMC を返した。foreground_class が \
             本ツール自身のウィンドウでない場合、Alt でフォーカスが奪われた可能性） !!!"
        } else {
            ""
        },
        if event.roman_dropped {
            "  !!! ROMAN ビットが落ちました（JIS かな直接入力に切り替わった可能性） !!!"
        } else {
            ""
        },
        if event.ime_closed {
            "  !!! IME が閉じました !!!"
        } else {
            ""
        },
        if !typed_as_expected && !event.typed_comp_str.is_empty() {
            "  !!! \"aiu\" が「あいう」以外に変換されました（JIS かな直接入力の可能性、\
             目視でも確認してください） !!!"
        } else {
            ""
        },
    );
    append_log(&event);
    event
}

fn main() -> anyhow::Result<()> {
    println!("=== Alt + VK_DBE_HIRAGANA 実機プローブ (v2: 自前 EDIT ウィンドウ版) ===");

    // SAFETY: メインスレッドから一度だけウィンドウを作成する。
    let (parent, edit) = unsafe { create_probe_window() }?;
    // SAFETY: parent は直前に作成した有効なウィンドウ。
    unsafe { force_foreground(parent) };
    pump_messages(Duration::from_millis(300));
    // SAFETY: edit は直前に作成した有効な子ウィンドウ。
    let _ = unsafe { SetFocus(Some(edit)) };
    pump_messages(Duration::from_millis(200));

    // 新規作成直後は IME が閉じている可能性が高いので明示的に開く。
    let pre_open = read_conv_snapshot(edit).open_status;
    if pre_open != Some(true) {
        // SAFETY: 起動時の初期化専用の単発呼び出し。
        unsafe { send_vk_tap(VK_IME_ON) };
        pump_messages(Duration::from_millis(800));
    }
    let post_open = read_conv_snapshot(edit).open_status;
    println!("ime open_status: before={pre_open:?} after_VK_IME_ON={post_open:?}");
    if post_open != Some(true) {
        println!(
            "!!! IME を開けませんでした（open_status={post_open:?}）。日本語 IME が既定に \
             設定されているか確認してください。計測を続けますが before/after が null に \
             なる可能性があります。"
        );
    }
    println!();
    println!("この小さなウィンドウにフォーカスしたまま Alt キーを押してください");
    println!("（Alt+Tab はしないこと——フォーカスが外れると計測できません）。");
    println!("押した瞬間に自動で VK_DBE_HIRAGANA を送信します。");
    println!("F9: Alt 抜きのベースライン計測（比較用）");
    println!("Esc または Ctrl+C: 終了");
    println!("ログ: {LOG_PATH}（追記式 JSON Lines）");
    println!();

    let start = Instant::now();
    let mut seq: u32 = 0;
    let mut alt_armed = true; // Alt が離されている状態からの立ち上がりのみ発火
    let mut f9_armed = true;

    loop {
        if key_down(VK_ESCAPE) {
            println!("Esc 検出、終了します。");
            break;
        }

        let alt_held = key_down(VK_MENU);
        if alt_held && alt_armed {
            alt_armed = false;
            seq += 1;
            run_probe(seq, "alt", start, edit);
        } else if !alt_held {
            alt_armed = true;
        }

        let f9_held = key_down(VK_F9);
        if f9_held && f9_armed {
            f9_armed = false;
            seq += 1;
            run_probe(seq, "baseline", start, edit);
        } else if !f9_held {
            f9_armed = true;
        }

        pump_messages(Duration::from_millis(15));
    }

    println!("完了。{seq} 件のイベントを {LOG_PATH} に記録しました。");
    // SAFETY: parent は有効なウィンドウ。プローブ終了時に破棄する。
    let _ = unsafe { DestroyWindow(parent) };
    Ok(())
}
