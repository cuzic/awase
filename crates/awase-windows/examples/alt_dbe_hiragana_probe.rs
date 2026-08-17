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
//! # 使い方（Windows 実機のみ）
//!
//! ```powershell
//! cargo run -p awase-windows --example alt_dbe_hiragana_probe --release
//! ```
//!
//! 起動後、検証したいアプリ（メモ帳・IME を有効にしたテキストフィールド等）に
//! フォーカスを移し、**Alt キーを押す**。押した瞬間（立ち上がりエッジ）を検知し、
//! 自動でフォーカス中のウィンドウへ `VK_DBE_HIRAGANA` の down+up を
//! SendInput する（`crate::tsf::output::make_scan_key_input` と同じ方式:
//! `wVk` を明示しつつ `MapVirtualKeyW` で得た scan code も付与、
//! `KEYEVENTF_SCANCODE` は使わない）。送信前後で対象ウィンドウの
//! `ImmGetConversionStatus`/`ImmGetOpenStatus` を読み比較し、ROMAN ビットが
//! 落ちていれば（≒ JIS かなへの切替が起きた）警告を出す。
//!
//! Alt を離すまで再武装しない（1回の押下につき1回だけ発火、連打防止）。
//! **F9** キーで Alt 抜きのベースライン計測（比較用）も随時実行できる。
//! Ctrl+C で終了。
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
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::Ime::{
    ImmGetContext, ImmGetConversionStatus, ImmGetOpenStatus, ImmReleaseContext,
    IME_CONVERSION_MODE, IME_SENTENCE_MODE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow};

/// `VK_DBE_HIRAGANA` — かな入力キー本来の VK（F2 warmup、`vk.rs` と同じ値）。
const VK_DBE_HIRAGANA: u16 = 0xF2;
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

fn foreground_class_name() -> (HWND, String) {
    // SAFETY: GetForegroundWindow は常に安全（NULL の可能性はあるが、その場合
    // GetClassNameW 側で len<=0 になるだけ）。
    let hwnd = unsafe { GetForegroundWindow() };
    let mut buf = [0u16; 256];
    // SAFETY: hwnd が NULL でも GetClassNameW は 0 を返すだけで安全。buf は
    // スタック上の十分なサイズのバッファ。
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    let name = if len <= 0 {
        String::new()
    } else {
        String::from_utf16_lossy(&buf[..usize::try_from(len).unwrap_or(0)])
    };
    (hwnd, name)
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

fn run_probe(seq: u32, trigger: &'static str, start: Instant) -> ProbeEvent {
    let (hwnd, foreground_class) = foreground_class_name();
    let before = read_conv_snapshot(hwnd);
    // SAFETY: 診断ツールの唯一の呼び出し経路。フォーカス中のウィンドウへ
    // VK_DBE_HIRAGANA down+up を送るだけ。
    unsafe { send_dbe_hiragana_pair() };
    // IME 側の反映を待つ（ImmSetConversionStatus は同期 API だが、TSF 経由の
    // 反映は非同期になりうるため実測で使われている待機幅に合わせて余裕を見る）。
    std::thread::sleep(Duration::from_millis(150));
    let after = read_conv_snapshot(hwnd);

    let roman_dropped = before.roman == Some(true) && after.roman == Some(false);
    let ime_closed = before.open_status == Some(true) && after.open_status == Some(false);

    let event = ProbeEvent {
        seq,
        trigger,
        elapsed_ms: start.elapsed().as_millis(),
        foreground_class,
        before,
        after,
        roman_dropped,
        ime_closed,
    };

    println!(
        "[{seq:>4}] trigger={trigger:<8} fg={:<28} before(open={:?} native={:?} roman={:?}) \
         → after(open={:?} native={:?} roman={:?}){}{}",
        event.foreground_class,
        event.before.open_status,
        event.before.native,
        event.before.roman,
        event.after.open_status,
        event.after.native,
        event.after.roman,
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
    );
    append_log(&event);
    event
}

fn main() {
    println!("=== Alt + VK_DBE_HIRAGANA 実機プローブ ===");
    println!("検証したいアプリ（IME 有効なテキストフィールド）へフォーカスを移し、");
    println!("Alt キーを押してください。押した瞬間に自動で VK_DBE_HIRAGANA を送信します。");
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
            run_probe(seq, "alt", start);
        } else if !alt_held {
            alt_armed = true;
        }

        let f9_held = key_down(VK_F9);
        if f9_held && f9_armed {
            f9_armed = false;
            seq += 1;
            run_probe(seq, "baseline", start);
        } else if !f9_held {
            f9_armed = true;
        }

        std::thread::sleep(Duration::from_millis(15));
    }

    println!("完了。{seq} 件のイベントを {LOG_PATH} に記録しました。");
}
