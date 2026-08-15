#![allow(unsafe_code)]
// Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
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

/// `OUTPUT_GATE` はプロセス全体で共有される単一の `static` であり、`cargo test`は
/// デフォルトで複数スレッド並行実行する。`OutputActiveGuard::begin()`（noop でない方）
/// を実際に呼ぶテストは、この global を実際にミューテートするため互いに排他する必要が
/// ある（`tsf::warmup::ms_ime_ready_coro`・`output::probe_io` の GJI 系テストで共有、
/// 詳細は `ms_ime_ready_coro.rs` の `phase1_does_not_hold_output_gate_only_phase2_does`
/// コメント参照）。`TSF_OBS_TEST_LOCK`（`observer.rs`）と同型のロック統一（BUG-65 の
/// 続き、2026-08-14）。
#[cfg(test)]
pub(crate) static OUTPUT_GATE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 出力セッションを RAII で管理するガード（参照カウント方式）。
///
/// `begin()` で深度をインクリメントし、深度 0→1 のとき `OUTPUT_GATE.active=true` をセット。
/// Drop 時に深度をデクリメントし、深度 1→0 のとき `OUTPUT_GATE.active=false` + drain。
///
/// TSF probe 延期中は `TsfProbeData` がガードを保持し続けることで、
/// `OutputSession` が drop しても `OUTPUT_GATE.active` が維持される。
///
/// `real` フィールド（BUG-65 追補2、2026-08-15）: 以前はゼロフィールドの
/// ユニット構造体で `noop_for_test()`（生成時は depth に触れない）と
/// `begin()`（生成時に depth を +1 する）を区別していたが、`Drop` は
/// 型に対して1つしか実装できず、生成方法を問わず無条件に
/// `depth.fetch_sub(1)` していた。このため `noop_for_test()` で作った
/// ガードが drop されるたびに、実際には一度も +1 していない
/// `OUTPUT_GATE.depth`（`AtomicU32`）が -1 され、0 から `fetch_sub` すると
/// `u32::MAX` 付近へラップアラウンドしていた。一度ラップすると、以降の
/// 本物の `begin()` がいくら `depth` を +1 しても `prev == 0` の等値判定に
/// 二度と一致しなくなり、`OUTPUT_GATE.active` が恒久的に `true` にならない
/// （`tsf::warmup::ms_ime_ready_coro::tests::phase1_does_not_hold_output_gate_only_phase2_does`
/// が実機で毎回決定論的に失敗していた真因。`output/probe_io.rs`・
/// `tsf/warmup/chrome_probe.rs`・`tsf/warmup/probe_fsm.rs` の `noop_for_test()`
/// 呼び出しが同じプロセス内の他の全テストの `OUTPUT_GATE` を汚染し続けていた）。
/// `real` で生成経路を保持し、`Drop`側でノーオペレーションを実際に無害化する。
#[derive(Debug)]
pub(crate) struct OutputActiveGuard {
    real: bool,
}

impl OutputActiveGuard {
    /// テスト専用: OUTPUT_GATE を変更しない NOOP ガード（生成時・drop時とも無害）。
    #[cfg(test)]
    pub(crate) const fn noop_for_test() -> Self {
        Self { real: false }
    }

    pub(crate) fn begin() -> Self {
        let prev = OUTPUT_GATE.depth.fetch_add(1, Ordering::AcqRel);
        if prev == 0 {
            OUTPUT_GATE.active.store(true, Ordering::Release);
        }
        Self { real: true }
    }
}

impl Drop for OutputActiveGuard {
    fn drop(&mut self) {
        if !self.real {
            return;
        }
        let prev = OUTPUT_GATE.depth.fetch_sub(1, Ordering::AcqRel);
        if prev == 1 {
            OUTPUT_GATE.active.store(false, Ordering::Release);
            // OUTPUT_GATE 解除〜drain ハンドラ実行の間のキーは [drain-race] で記録される。
            // この時点 (=解除瞬間) のキュー長を出して race 期間の挙動を辿りやすくする。
            let pending = crate::INPUT_DEFER.pending_len_nonblocking();
            log::debug!(
                "[output-gate] deactivated (depth 1→0), pending_drain={} → post WM_DRAIN_OUTPUT_QUEUE",
                pending.map_or_else(|| "?".to_owned(), |n| n.to_string()),
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
