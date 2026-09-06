//! GJI IME actuation（`SendInput` の kanji_marker/tsf_marker_warmup、
//! `SendMessageTimeoutW` の actuation cmd）が発行されたことを記録する単調カウンタ
//! （ADR-140 Step1 確定設計・案D）。
//!
//! # なぜ `conv_mutation`（[`crate::conv_mutation`]）を流用しないか
//!
//! `conv_mutation::bump()` のゲート（`win32::input_may_mutate_conv` /
//! `imm::send_ime_control` の `IMC_SETCONVERSIONMODE` 判定）は open 専用 VK
//! （`VK_IME_ON`/`VK_IME_OFF`/`VK_KANJI`）や `IMC_SETOPENSTATUS` では増分しない
//! 仕様（`conv_mutation.rs` module doc）。BUG-113 の actuation はまさに GJI の
//! open 軸（`IME_KANJI_MARKER`）であり、既存フェンスはこの actuation を
//! 1 回も数えていない——「issue 時点を見ていない」以前に、このカウンタは
//! そもそもこの actuation を対象にしていなかった。
//!
//! # bump 地点（物理 syscall 境界2箇所、必ず syscall の前）
//!
//! 論理呼び出し箇所（`ime_controller.rs::apply` 等）を個別に bump する方式は
//! 採らない——未発見の呼び出し経路（ADR-140 確定事実5が「少なくとも3つ確認、
//! 全てとは限らない」と明記）が残っていると、issue #136 型の「1箇所塞いで
//! 別箇所に穴」を再演するため。代わりに OS に到達する物理境界そのもの
//! （唯一のチョークポイント）で bump する:
//!
//! - [`crate::win32::send_input_safe`]: ADR-140 Step0 診断ログの
//!   `ime_actuation_marker_kind` 判定と**同一の条件式**で bump する。
//! - [`crate::imm::send_ime_control`]: 同診断ログの `kind=actuation` 判定
//!   （`!matches!(cmd, IMC_GETOPENSTATUS | IMC_GETCONVERSIONMODE)`）と
//!   **同一の条件式**で bump する。
//!
//! 判定を診断ログと共有することで将来の乖離を防ぐ。`Ordering::Relaxed` で
//! 足りる（`conv_mutation` と同じ理屈: 単一ロケーションのカウンタで、これ経由で
//! 他のデータを publish しないため）。
//!
//! # 比較点（probe 側の呼び出し元のみ、`ime::offload_unsafe` には絶対に置かない）
//!
//! 現状の唯一の消費者は `runtime/key_pipeline.rs::kp_stage_idle_conv_check_inner`
//! （BUG-113 の idle-conv-check probe）。`crate::ime::offload_unsafe` は
//! probe/actuation 双方が通る共通ヘルパーのため、ここに比較を置くと
//! 「actuation が actuation を待つ」という、却下済みの対称ロック方式と同型の
//! 失敗モードを1階層上で再現する。将来別の probe 経路がこのフェンスを使う
//! 場合も、比較は必ずその probe 呼び出し元（`ime.rs`/`imm.rs` の共通ヘルパー
//! ではない）で行うこと。
//!
//! # abandon カウンタ（決定I: resync 経路と通常経路を分けて数える）
//!
//! resync 経路（[`crate::focus_resync`] 由来）の abandon は「defer 中のキーが
//! `FOCUS_RESYNC_DEADLINE_MS` まで出てこない」という体感遅延に直結する一方、
//! 通常経路の abandon は「今回の idle-conv-check を1回諦めた」だけ
//! ——ユーザー体感コストが桁違いのため、合算した1つのカウンタだと前者に
//! 埋もれて後者の頻発を見逃す。starvation（probe が永久に不成立になり
//! idle-conv-check の本来の目的が静かに死ぬ事態）が起きていないかを実機
//! ソークで確認する（Step1 の完了条件）用に、不具合報告（`bug_report.rs`）へ
//! 両方とも累積値のまま渡す。

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// 0 は「まだ一度も観測していない」ことを表すセンチネルとして予約する
/// （`conv_mutation`/`send_health` 等、他の fence 実装と同じ規約）ため 1 から始める。
static PROBE_ACTUATION_FENCE: AtomicU64 = AtomicU64::new(1);

/// GJI IME actuation の物理 syscall 境界（[`crate::win32::send_input_safe`] /
/// [`crate::imm::send_ime_control`]）で呼ぶ。
pub(crate) fn bump() {
    PROBE_ACTUATION_FENCE.fetch_add(1, Ordering::Relaxed);
}

/// 現在の値を読む。probe 側の spawn 時スナップショットと issue/apply 時の
/// 再読み取りを比較するために使う（ビット一致で判定すること、経過時間で
/// 判定しない——`conv_mutation::current()` と同じ規約）。
pub(crate) fn current() -> u64 {
    PROBE_ACTUATION_FENCE.load(Ordering::Relaxed)
}

/// resync 経路（`kp_trigger_focus_resync` 由来）の probe abandon 累計回数
/// （プロセス生存期間中、リセットしない）。
static ABANDONED_RESYNC_LIFETIME_COUNT: AtomicU32 = AtomicU32::new(0);
/// 通常経路（`kp_stage_idle_conv_check`）の probe abandon 累計回数。
static ABANDONED_NORMAL_LIFETIME_COUNT: AtomicU32 = AtomicU32::new(0);

/// probe が actuation との交錯を検知して abandon したことを記録する。
/// `resync_generation.is_some()` なら resync 経路、`None` なら通常経路として
/// 別々に数える（module doc 参照）。
pub(crate) fn record_abandoned(resync_generation: Option<u64>) {
    if resync_generation.is_some() {
        ABANDONED_RESYNC_LIFETIME_COUNT.fetch_add(1, Ordering::Relaxed);
    } else {
        ABANDONED_NORMAL_LIFETIME_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}

/// `ABANDONED_RESYNC_LIFETIME_COUNT` を消費せずに読む（不具合報告用診断）。
pub(crate) fn abandoned_resync_lifetime_count() -> u32 {
    ABANDONED_RESYNC_LIFETIME_COUNT.load(Ordering::Relaxed)
}

/// `ABANDONED_NORMAL_LIFETIME_COUNT` を消費せずに読む（不具合報告用診断）。
pub(crate) fn abandoned_normal_lifetime_count() -> u32 {
    ABANDONED_NORMAL_LIFETIME_COUNT.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bump_advances_current_monotonically() {
        let before = current();
        bump();
        let after = current();
        assert!(after > before, "bump() は current() を単調に進める");
    }

    #[test]
    fn record_abandoned_splits_resync_and_normal_counters() {
        let resync_before = abandoned_resync_lifetime_count();
        let normal_before = abandoned_normal_lifetime_count();

        record_abandoned(Some(42));
        assert_eq!(abandoned_resync_lifetime_count(), resync_before + 1);
        assert_eq!(abandoned_normal_lifetime_count(), normal_before);

        record_abandoned(None);
        assert_eq!(abandoned_resync_lifetime_count(), resync_before + 1);
        assert_eq!(abandoned_normal_lifetime_count(), normal_before + 1);
    }
}
