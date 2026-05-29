//! 出力セッション統合 — OUTPUT_GATE と Win32 メッセージループを橋渡し。
//!
//! `send_keys()` の全期間を一つの出力セッションとして管理し、
//! その間に到着した全キーを [`crate::input_defer::INPUT_DEFER`] に退避する。
//! セッション終了後に `WM_DRAIN_OUTPUT_QUEUE` 経由でキーを順序保証付きで再配送する。

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// SendInput 出力中にキー入力を保留するゲート。
///
/// # OutputGate vs TsfGate
/// - `OutputGate`: `send_keys` 実行中に外部キー入力を defer するゲート（再入防止）。
/// - `TsfGate` (in `probe.rs`): TSF warm-up 完了まで出力キューを保留するゲート。
///   両者は独立した目的を持ち、混同しないこと。
///
/// ## 内部フィールド（クロススレッド共有）
///
/// - `active`: true の間、フックコールバックはキーを INPUT_DEFER に退避する
/// - `depth`: RAII Guard の参照カウント（0→1 で active=true、1→0 で active=false）
/// - `last_vk_output_ms`: VK/TSF 最終 SendInput 時刻（with_app 再入回避のため atomic）
#[derive(Debug)]
pub struct OutputGate {
    pub(crate) active: AtomicBool,
    depth: AtomicU32,
    pub(crate) last_vk_output_ms: AtomicU64,
}

impl Default for OutputGate {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputGate {
    #[must_use] 
    pub const fn new() -> Self {
        Self {
            active: AtomicBool::new(false),
            depth: AtomicU32::new(0),
            last_vk_output_ms: AtomicU64::new(0),
        }
    }

    /// `OUTPUT_GATE.active` の現在値を取得する。
    #[inline]
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    /// VK/TSF 送信時刻を現在時刻（ms）で記録する。
    #[inline]
    pub(crate) fn mark_vk_output(&self, ms: u64) {
        self.last_vk_output_ms.store(ms, Ordering::Relaxed);
    }

    /// `last_vk_output_ms` の現在値を取得する。
    #[inline]
    pub fn last_vk_output_ms_val(&self) -> u64 {
        self.last_vk_output_ms.load(Ordering::Relaxed)
    }
}

pub static OUTPUT_GATE: OutputGate = OutputGate::new();

/// 出力セッションを RAII で管理するガード（参照カウント方式）。
///
/// `begin()` で深度をインクリメントし、深度 0→1 のとき `OUTPUT_GATE.active=true` をセット。
/// Drop 時に深度をデクリメントし、深度 1→0 のとき `OUTPUT_GATE.active=false` + drain。
///
/// TSF probe 延期中は `TsfProbeData` がガードを保持し続けることで、
/// `OutputSession` が drop しても `OUTPUT_GATE.active` が維持される。
#[derive(Debug)]
pub(crate) struct OutputActiveGuard;

impl OutputActiveGuard {
    /// テスト専用: OUTPUT_GATE を変更しない NOOP ガード。
    #[cfg(test)]
    pub(crate) const fn noop_for_test() -> Self { Self }

    pub(crate) fn begin() -> Self {
        let prev = OUTPUT_GATE.depth.fetch_add(1, Ordering::AcqRel);
        if prev == 0 {
            OUTPUT_GATE.active.store(true, Ordering::Release);
        }
        Self
    }
}

impl Drop for OutputActiveGuard {
    fn drop(&mut self) {
        let prev = OUTPUT_GATE.depth.fetch_sub(1, Ordering::AcqRel);
        if prev == 1 {
            OUTPUT_GATE.active.store(false, Ordering::Release);
            // OUTPUT_GATE 解除〜drain ハンドラ実行の間のキーは [drain-race] で記録される。
            // この時点 (=解除瞬間) のキュー長を出して race 期間の挙動を辿りやすくする。
            let pending = crate::INPUT_DEFER.pending_len_nonblocking();
            log::debug!(
                "[output-gate] deactivated (depth 1→0), pending_drain={} → post WM_DRAIN_OUTPUT_QUEUE",
                pending.map_or("?".to_owned(), |n| n.to_string()),
            );
            post_drain_output_queue();
        }
    }
}

/// OUTPUT_GATE.active 解除後にキューされたキーを NICOLA へ再配送するカスタムメッセージ。
///
/// `WM_APP + 18` = 0x8012
pub const WM_DRAIN_OUTPUT_QUEUE: u32 = 0x8000 + 18;

/// OUTPUT_GATE.active 解除後に呼ぶ。キューに溜まったキーを再配送するメッセージを投げる。
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
