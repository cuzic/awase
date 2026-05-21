//! 出力セッション統合 — OUTPUT_ACTIVE / OUTPUT_PENDING_QUEUE と Win32 メッセージループを橋渡し。
//!
//! `send_keys()` の全期間を一つの出力セッションとして管理し、
//! その間に到着した全キーを `OUTPUT_PENDING_QUEUE` に退避する。
//! セッション終了後に `WM_DRAIN_OUTPUT_QUEUE` 経由でキーを順序保証付きで再配送する。

use std::sync::atomic::AtomicBool;
use awase::types::RawKeyEvent;

/// `send_keys()` の全期間にわたる出力セッションフラグ。
///
/// true の間、フックコールバックは APP.get_mut() を呼ばず Consumed を返す。
/// キーイベントは [`OUTPUT_PENDING_QUEUE`] に退避され、セッション終了後に再配送される。
/// これにより TSF 送信バッチより先にキーが WezTerm へ届く順序逆転と
/// send_keys 実行中の APP re-entrancy を防ぐ。
pub static OUTPUT_ACTIVE: AtomicBool = AtomicBool::new(false);

/// 出力セッションを RAII で管理するガード。
///
/// `begin()` で `OUTPUT_ACTIVE=true` をセット。
/// Drop 時に `OUTPUT_ACTIVE=false` にリセットし、`post_drain_output_queue()` を呼ぶ。
#[derive(Debug)]
pub struct OutputActiveGuard;

impl OutputActiveGuard {
    /// 出力セッションを開始する。OUTPUT_ACTIVE を true にセットして Guard を返す。
    pub fn begin() -> Self {
        OUTPUT_ACTIVE.store(true, std::sync::atomic::Ordering::Release);
        Self
    }
}

impl Drop for OutputActiveGuard {
    fn drop(&mut self) {
        OUTPUT_ACTIVE.store(false, std::sync::atomic::Ordering::Release);
        post_drain_output_queue();
    }
}

/// OUTPUT_ACTIVE=true 中に到着したキーイベントの退避キュー。
///
/// セッション終了後に WM_DRAIN_OUTPUT_QUEUE メッセージ経由で NICOLA へ再配送する。
/// これにより物理キーが TSF 注入バッチより先に WezTerm に届く順序逆転を防ぐ。
pub static OUTPUT_PENDING_QUEUE: std::sync::Mutex<Vec<RawKeyEvent>> =
    std::sync::Mutex::new(Vec::new());

/// OUTPUT_ACTIVE 解除後にキューされたキーを NICOLA へ再配送するカスタムメッセージ。
///
/// `WM_APP + 18` = 0x8012
pub const WM_DRAIN_OUTPUT_QUEUE: u32 = 0x8000 + 18;

/// OUTPUT_ACTIVE 解除後に呼ぶ。キューに溜まったキーを再配送するメッセージを投げる。
pub fn post_drain_output_queue() {
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
