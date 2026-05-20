//! メッセージループ統合 — PROBE_ACTIVE / PROBE_KEY_QUEUE と Win32 メッセージループを橋渡し。
//!
//! `TsfReadinessProbe` の `block_on` 待機中にキーフックが再入しないよう制御し、
//! プローブ完了後に退避キーを `WM_DRAIN_PROBE_QUEUE` 経由で順序保証付きで再配送する。

use std::sync::atomic::AtomicBool;

use awase::types::RawKeyEvent;

/// [probe] `wait_for_tsf_cold_settle()` がアクティブ中かどうかのフラグ。
///
/// true の間、フックコールバックは APP.get_mut() を呼ばず Consumed を返す。
/// キーイベントは [`PROBE_KEY_QUEUE`] に退避され、プローブ終了後に NICOLA へ再配送される。
/// これにより MsgWaitForMultipleObjects + PeekMessage ループ中の re-entrancy を防ぐ。
pub static PROBE_ACTIVE: AtomicBool = AtomicBool::new(false);

/// PROBE_ACTIVE を RAII で管理するガード。
/// Drop 時に `PROBE_ACTIVE` を false にリセットする。
#[derive(Debug)]
pub struct ProbeGuard;

impl Drop for ProbeGuard {
    fn drop(&mut self) {
        PROBE_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

/// PROBE_ACTIVE=true 中に到着したキーイベントの退避キュー。
///
/// プローブ終了後に WM_DRAIN_PROBE_QUEUE メッセージ経由で NICOLA へ再配送する。
/// これにより物理キーが TSF 注入バッチより先に WezTerm に届く順序逆転を防ぐ。
pub static PROBE_KEY_QUEUE: std::sync::Mutex<Vec<RawKeyEvent>> =
    std::sync::Mutex::new(Vec::new());

/// PROBE_ACTIVE 解除後にキューされたキーを NICOLA へ再配送するカスタムメッセージ。
///
/// `WM_APP + 18` = 0x8012
pub const WM_DRAIN_PROBE_QUEUE: u32 = 0x8000 + 18;

/// PROBE_ACTIVE 解除後に呼ぶ。キューに溜まったキーを再配送するメッセージを投げる。
pub fn post_drain_probe_queue() {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
    // Safety: PostMessageW は非同期送信のみ、HWND=null でスレッドキューに投稿
    let _ = unsafe {
        PostMessageW(
            HWND::default(),
            WM_DRAIN_PROBE_QUEUE,
            windows::Win32::Foundation::WPARAM(0),
            windows::Win32::Foundation::LPARAM(0),
        )
    };
}
