//! 出力セッション統合 — OUTPUT_ACTIVE と Win32 メッセージループを橋渡し。
//!
//! `send_keys()` の全期間を一つの出力セッションとして管理し、
//! その間に到着した全キーを [`crate::input_defer::INPUT_DEFER`] に退避する。
//! セッション終了後に `WM_DRAIN_OUTPUT_QUEUE` 経由でキーを順序保証付きで再配送する。

use std::sync::atomic::AtomicBool;
use awase::types::Timestamp;

/// `send_keys()` の全期間にわたる出力セッションフラグ。
///
/// true の間、フックコールバックは APP.get_mut() を呼ばず Consumed を返す。
/// キーイベントは [`crate::input_defer::INPUT_DEFER`] に退避され、セッション終了後に再配送される。
/// これにより TSF 送信バッチより先にキーが WezTerm へ届く順序逆転と
/// send_keys 実行中の APP re-entrancy を防ぐ。
pub static OUTPUT_ACTIVE: AtomicBool = AtomicBool::new(false);

/// OUTPUT_ACTIVE 解除後にキューされたキーを NICOLA へ再配送するカスタムメッセージ。
///
/// `WM_APP + 18` = 0x8012
pub const WM_DRAIN_OUTPUT_QUEUE: u32 = 0x8000 + 18;

/// `in_with_app()` = true のとき hook から退避した生キーイベント。
///
/// hook.rs 内で APP.get_mut() が呼べない（二重借用 UB）ため、
/// 最小限の情報のみ保存し classify は drain 時に行う。
#[derive(Debug, Clone, Copy)]
pub struct RawHookData {
    pub vk_code: u16,
    pub scan_code: u32,
    pub is_keydown: bool,
    pub extra_info: usize,
    pub timestamp: Timestamp,
}

/// OUTPUT_ACTIVE 解除後に呼ぶ。キューに溜まったキーを再配送するメッセージを投げる。
pub(crate) fn post_drain_output_queue() {
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
    let _ = unsafe {
        PostMessageW(
            None,
            WM_DRAIN_OUTPUT_QUEUE,
            windows::Win32::Foundation::WPARAM(0),
            windows::Win32::Foundation::LPARAM(0),
        )
    };
}
