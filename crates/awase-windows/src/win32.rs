#![allow(unsafe_code)]
// Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
//! Windows API の安全ラッパー

use std::time::Duration;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_KEYBOARD, KEYEVENTF_UNICODE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, PostMessageW, GUITHREADINFO,
};

/// タイムアウト付きで任意の処理をワーカースレッドで実行する。
///
/// `win32_async::run_with_timeout` の re-export。
pub use win32_async::run_with_timeout;

/// `HWND` の null チェック拡張トレイト。
pub trait HwndExt {
    /// null なら `None`、非 null なら `Some(self)` を返す。
    ///
    /// Win32 API が返す `HWND` は null（フォーカスなし・失敗）を示すことがある。
    /// 境界でこのメソッドを使い、以降は `Option<HWND>` として処理する。
    #[must_use]
    fn non_null(self) -> Option<HWND>;
}

impl HwndExt for HWND {
    fn non_null(self) -> Option<HWND> {
        (!self.0.is_null()).then_some(self)
    }
}

/// post-bypass latch のスコープ。武装時と評価時で必ず同じ関数で採る。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForegroundScope {
    pub pid: u32,
    /// `GetForegroundWindow()` の生値（`HWND` は `Send` でないため isize で持つ）。
    pub hwnd: isize,
}

impl ForegroundScope {
    /// 取得失敗（前景窓なし・pid 0）。実在のスコープとは決して等しくならない。
    pub const INVALID: Self = Self { pid: 0, hwnd: 0 };

    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.pid != 0 && self.hwnd != 0
    }
}

#[must_use]
pub fn foreground_scope() -> ForegroundScope {
    // SAFETY: GetForegroundWindow はどのスレッドからも安全に呼べる非ブロッキング API。
    //         pid の抽出は crate::focus::classify::get_window_process_id に委ねる
    //         （GetWindowThreadProcessId の呼び出し規約を1箇所に集約する）。
    //         前景窓なし・pid 0 は INVALID として扱う。
    let Some(hwnd) = unsafe { GetForegroundWindow() }.non_null() else {
        return ForegroundScope::INVALID;
    };
    let pid = crate::focus::classify::get_window_process_id(hwnd);
    if pid == 0 {
        ForegroundScope::INVALID
    } else {
        ForegroundScope {
            pid,
            hwnd: hwnd.0 as isize,
        }
    }
}

/// エンジン専用 HWND 宛にカスタムメッセージを POST する。
///
/// ADR-105 以降、この集約点はスレッド ID ではなく
/// `engine_window::engine_hwnd()` の HWND を宛先にする。HWND 宛メッセージは
/// トレイメニュー等のネストしたモーダルポンプ中でも通常の window message として
/// dispatch されるため、旧 `PostThreadMessageW` 経路の恒久消失を避けられる。
///
/// 旧実装は `PostMessageW(None, ..)` を使っていたが、hwnd=NULL の `PostMessageW` は
/// 「**呼び出しスレッド自身**への `PostThreadMessage`」と等価（Microsoft docs）であり、
/// ワーカースレッド（gji-io-monitor / UIA worker 等）から呼ぶとメッセージが誰にも
/// 処理されず消失していた。これにより `WM_IME_KIND_CHANGED` が main に一度も届かず、
/// MS-IME 環境でも warmup 戦略がデフォルトの GjiFsm のまま走り続けた
/// （docs/known-bugs.md BUG-09）。`WM_FOCUS_KIND_UPDATE`（UIA worker 発）も同罪だった。
///
/// `engine_hwnd()` がまだ `None` の起動最初期だけは互換フォールバックとして
/// `PostMessageW(None, ..)` を使う。この分岐は BUG-09 と同型の罠を持つため、
/// ワーカースレッドから確実に届ける用途では成功扱いにしない。呼び出し元が復旧手段を
/// 持つ場合は戻り値 `false` を見て再試行可能な状態へ戻すこと。
#[allow(clippy::must_use_candidate)]
pub fn post_to_main_thread(msg: u32) -> bool {
    post_to_main_thread_with(msg, 0, 0)
}

/// エンジン専用 HWND 宛にパラメータ付きでカスタムメッセージを POST する。
///
/// 戻り値は「エンジン HWND 宛の post が成功したか」。`engine_hwnd()` が未作成の間の
/// NULL-hwnd フォールバックは、呼び出しスレッド自身への thread message になりうるため
/// `false` を返す。失敗時は `log::warn!` を出す。**`WH_KEYBOARD_LL` フックコールバック
/// のスタックから同期的に呼んではならない**（ロック取得を伴うログ出力を持ち込むため）。
/// フック経由の合図には [`post_to_main_thread_quiet`] を使うこと。
#[allow(clippy::must_use_candidate)]
pub fn post_to_main_thread_with(msg: u32, wparam: usize, lparam: isize) -> bool {
    post_to_main_thread_inner(msg, wparam, lparam, true)
}

/// [`post_to_main_thread_with`] のログ無し版。
///
/// `WH_KEYBOARD_LL` フックコールバックのスタックから同期的に呼ばれる
/// `hook_channel::request_engine_wake` 専用（ADR-102 決定2 / Opus敵対的レビュー指摘、
/// 2026-08-26）。フックコールバック上でロック取得を伴うログ出力を追加しないという
/// 制約を守るため、失敗しても `log::warn!` を呼ばない。失敗の可視化は呼び出し元
/// （`hook_channel`）がアトミックフラグで持ち回り、エンジンスレッド側のウォッチドッグ
/// がそのフラグを見てログを出す。
pub(crate) fn post_to_main_thread_quiet(msg: u32) -> bool {
    post_to_main_thread_inner(msg, 0, 0, false)
}

fn post_to_main_thread_inner(msg: u32, wparam: usize, lparam: isize, log_on_failure: bool) -> bool {
    let msg = if msg == windows::Win32::UI::WindowsAndMessaging::WM_QUIT {
        crate::WM_ENGINE_QUIT_REQUEST
    } else {
        msg
    };
    let Some(hwnd) = crate::runtime::engine_window::engine_hwnd() else {
        let _ = unsafe {
            PostMessageW(
                None,
                msg,
                windows::Win32::Foundation::WPARAM(wparam),
                windows::Win32::Foundation::LPARAM(lparam),
            )
        };
        return false;
    };
    if let Err(err) = unsafe {
        PostMessageW(
            Some(hwnd),
            msg,
            windows::Win32::Foundation::WPARAM(wparam),
            windows::Win32::Foundation::LPARAM(lparam),
        )
    } {
        if log_on_failure {
            log::warn!("[post-main] PostMessageW(engine_hwnd) failed msg=0x{msg:X}: {err}");
        }
        return false;
    }
    true
}

/// `send_input_safe` に渡された `INPUT` が conv-mode ワードを変えうる VK の
/// キーボードイベントかどうかを判定する（BUG-34 横展開 Step0-a）。
///
/// Unicode モード（`KEYEVENTF_UNICODE`）の `INPUT` は `wVk` が常に 0 で
/// 意味を持たない（`wScan` が UTF-16 code unit を運ぶ）ため対象外にする。
fn input_may_mutate_conv(input: &INPUT) -> bool {
    if input.r#type != INPUT_KEYBOARD {
        return false;
    }
    // SAFETY: r#type == INPUT_KEYBOARD を確認済みなので Anonymous.ki は
    //         このユニオンの有効なアクティブフィールドである。
    let ki = unsafe { input.Anonymous.ki };
    if ki.dwFlags.contains(KEYEVENTF_UNICODE) {
        return false;
    }
    crate::vk::vk_may_mutate_conv(awase::types::VkCode(ki.wVk.0))
}

/// `SendInput` の安全ラッパー（`size_of` キャストを安全に処理）
///
/// BUG-34 横展開 Step0-a: このクレートの全 `SendInput` 呼び出しは本関数を
/// 経由する唯一のチョークポイントであるため、ここで conv-mode ワードを
/// 変えうる VK の送信を検知して `conv_mutation::bump()` を呼ぶ
/// （`send_eager_tsf_warmup` 等の名前付きラッパー単位で個別に列挙すると、
/// `send_ime_mode_key` のようにユーザー設定 VK を送る関数を漏れなく数えられない
/// ——`vk::vk_may_mutate_conv` の doc 参照）。もう1つのゲート
/// （`imm::send_ime_control` の `IMC_SETCONVERSIONMODE` 経路）と合わせて
/// `conv_mutation::bump()` の doc 参照。
///
/// # Panics
/// `INPUT` のサイズが `i32` に収まらない場合（実際には起こらない）。
#[must_use]
pub(crate) fn send_input_safe(inputs: &[INPUT]) -> u32 {
    if inputs.iter().any(input_may_mutate_conv) {
        crate::conv_mutation::bump();
    }
    let size = i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32");
    // SAFETY: inputs スライスは呼び出し中有効であり、size は sizeof::<INPUT>() の正確な値。
    //         SendInput はスライスの範囲外を読まない。
    unsafe { SendInput(inputs, size) }
}

/// `&str` を NUL 終端 UTF-16 `Vec<u16>` に変換する。
///
/// Win32 API に渡す `PCWSTR` を作るときの定型句を集約する。
#[must_use]
pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 起動失敗などの致命的なエラーを `MessageBoxW` で表示する。
///
/// `main.rs::show_startup_error`（awase.exe 側）と
/// `awase-settings/src/startup_failure.rs::show_dialog`
/// （awase-settings.exe 側）の両方が同じ MessageBoxW 定型句
/// （UTF-16 変換・フラグ組み合わせ）を個別に持っていた（コードレビュー
/// 指摘）ため、ここに集約する。呼び出し元スレッドをブロックする点は
/// 呼び出し元が把握していること前提（`MessageBoxW` 自体の制約）。
pub fn show_error_dialog(title: &str, message: &str) {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
    };

    let title_wide = to_wide(title);
    let message_wide = to_wide(message);
    // SAFETY: title_wide/message_wide は NUL 終端済み UTF-16 で呼び出し中有効。
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(message_wide.as_ptr()),
            PCWSTR(title_wide.as_ptr()),
            MB_OK | MB_ICONERROR | MB_TOPMOST | MB_SETFOREGROUND,
        );
    }
}

/// `GetGUIThreadInfo` の結果
#[derive(Debug, Clone, Copy)]
pub struct GuiThreadResult {
    /// フォーカスを持つウィンドウ。null（フォーカスなし）の場合は `None`。
    pub focused_hwnd: Option<HWND>,
    /// ウィンドウが属するスレッド ID（0 = 取得失敗）
    pub thread_id: u32,
}

/// `GetGUIThreadInfo(0, ...)` のラッパー — ブロッキングが一定時間を超えたら
/// フォールバックとして `GetForegroundWindow()` を返す。
///
/// `GetGUIThreadInfo` はフォアグラウンドウィンドウの GUI スレッドにメッセージを送るため、
/// 対象スレッドがハングしていると無期限にブロックする。
/// `run_with_timeout` でワーカースレッドで実行し、タイムアウト時は
/// 非ブロッキングな `GetForegroundWindow` にフォールバックする。
///
/// # Panics
/// `GUITHREADINFO` のサイズが `u32` に収まらない場合（実際には起こらない）。
///
/// # Safety
/// Win32 API を呼び出す。
#[must_use]
pub unsafe fn get_gui_thread_info_with_timeout(timeout: Duration) -> GuiThreadResult {
    // HWND はポインタだが、スレッド間で安全に送信可能
    // （Win32 ウィンドウハンドルはプロセス内で有効なグローバルリソース）
    struct SendableResult(Option<HWND>, u32);
    unsafe impl Send for SendableResult {}

    let result = run_with_timeout(timeout, || {
        let mut info = GUITHREADINFO {
            cbSize: u32::try_from(size_of::<GUITHREADINFO>())
                .expect("GUITHREADINFO size is a small constant that always fits in u32"),
            ..Default::default()
        };
        // SAFETY: info は cbSize を正しく設定したスタック上の有効な構造体。
        //         GetGUIThreadInfo(0, ...) はフォアグラウンドスレッドの情報を取得する。
        //         GetForegroundWindow / GetWindowThreadProcessId はどのスレッドからも安全に呼べる。
        unsafe {
            if GetGUIThreadInfo(0, &raw mut info).is_ok() {
                // hwndFocus が null なら hwndActive を使う
                let hwnd = info
                    .hwndFocus
                    .non_null()
                    .or_else(|| info.hwndActive.non_null());
                let tid = hwnd.map_or(0, |h| {
                    let mut pid = 0u32;
                    GetWindowThreadProcessId(h, Some(&raw mut pid))
                });
                SendableResult(hwnd, tid)
            } else {
                SendableResult(GetForegroundWindow().non_null(), 0)
            }
        }
    });

    match result {
        Some(SendableResult(hwnd, tid)) => GuiThreadResult {
            focused_hwnd: hwnd,
            thread_id: tid,
        },
        None => {
            // フォールバック: GetForegroundWindow は非ブロッキング
            // SAFETY: GetForegroundWindow はどのスレッドからも安全に呼べる非ブロッキング API。
            GuiThreadResult {
                focused_hwnd: unsafe { GetForegroundWindow() }.non_null(),
                thread_id: 0,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{input_may_mutate_conv, INPUT, INPUT_KEYBOARD, KEYEVENTF_UNICODE};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT_0, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS, MOUSEEVENTF_MOVE, MOUSEINPUT,
        VIRTUAL_KEY,
    };

    fn key_input(vk: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
        INPUT {
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
        }
    }

    fn mouse_input() -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: 0,
                    dwFlags: MOUSEEVENTF_MOVE,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    /// 通常のキーボード INPUT で conv-mutating な VK（VK_DBE_HIRAGANA）なら true。
    #[test]
    fn keyboard_input_with_conv_mutating_vk_is_true() {
        let input = key_input(0xF2, KEYBD_EVENT_FLAGS::default()); // VK_DBE_HIRAGANA
        assert!(input_may_mutate_conv(&input));
    }

    /// 通常のキーボード INPUT で open-only な VK（VK_IME_ON）なら false。
    #[test]
    fn keyboard_input_with_open_only_vk_is_false() {
        let input = key_input(0x16, KEYBD_EVENT_FLAGS::default()); // VK_IME_ON
        assert!(!input_may_mutate_conv(&input));
    }

    /// `KEYEVENTF_UNICODE` が立っている場合、`wVk` に conv-mutating な値が
    /// たまたま入っていても対象外（wVk は意味を持たず、常に 0 で送られる想定だが、
    /// 万一非ゼロでも安全側＝false であることを固定する）。
    #[test]
    fn unicode_mode_input_is_false_even_if_wvk_looks_conv_mutating() {
        let input = key_input(0xF2, KEYEVENTF_UNICODE); // VK_DBE_HIRAGANA だが Unicode モード
        assert!(!input_may_mutate_conv(&input));
    }

    /// マウス INPUT（`INPUT_KEYBOARD` ではない）は常に false。
    #[test]
    fn mouse_input_is_false() {
        assert!(!input_may_mutate_conv(&mouse_input()));
    }
}
