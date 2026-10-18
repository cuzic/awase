//! `output/mod.rs` / `output/vk_send.rs` の「probe・recovery の進行中は新しいモーラを退避する」判断の核
//! （FCIS F6、ADR-123 変更 A+C・ADR-128）。
//!
//! 元の分岐を変えずに、判断だけを純粋関数へ出した。値を読む（`has_pending_tsf`・`RAW_TSF_LITERAL`・退避キューの長さ）と、
//! 実行する（`push_deferred_vks`・`flush_pending_deferred_vks`・ログ）のは殻（`Output`）に残る。事実は bool と件数だけで、
//! 時刻は持たない（V4 の対象外）。各 plan は理由の enum を返す（journal 用）。
//!
//! **ADR-156 の窓口のうち、`plan_blocking` を共有するもの**:
//! - defer 側: `Output::defer_vks_if_probe_or_recovery_in_flight`（`DeferGate::Enforced` は `check_raw_recovery=true`、
//!   `Exempt` は `false`。この対応は純粋側では固定できない。走査が固定するのは、共通コアの引数の流れと `defer_if_probe_in_flight`(true)・`..._recovery_exempt`(false) の呼び出し、drain の `(true)` まで。`defer_respecting_gate` の Enforced/Exempt の arm の入れ替えと、`defer_vk_if_probe_in_flight` の `true`→`false` は走査も純粋側のテストも検出しない(未固定)）。
//! - drain-before-send 側: `Output::drain_pending_deferred_before_send_if_queue_only`（`Exempt` は何も読まずに戻り、
//!   `Enforced` は `check_raw_recovery=true` で読む。ADR-128 で Exempt の早期 return が入ったので、「gate に関わらず
//!   raw を見る」非対称は今は観測できない）。
//! - 参照: `Output::probe_or_recovery_in_flight`（ADR-203）。
//!
//! - 解放側: `Output::finish_probe_stage`（段末の解放。F6c で `probe_or_recovery_block_reason(true)` 経由にした）。
//!
//! **`plan_blocking` を通らない窓口（同じ述語だが共有していない）**: 解放側の
//! `take_pending_deferred_if_probe_idle`（`!has_pending_tsf()`。`warmup_coord` の中で raw を読めない。`finish_probe_stage` と
//! drain-before-send は先に `plan_blocking` で raw を見てから `flush_pending_deferred_vks` 経由で呼ぶ）・
//! `flush_stale_deferred_vks_after_recovery`（回収の最後の同期点。直前に backs/romaji を消費済みで raw は偽になるため読まない）、
//! 破棄側の `cancel_probe`・`discard_raw_recovery_if_moved`（BUG-194。どちらも `take_pending_deferred()` で退避キューを捨てる）、
//! 参照側の `step_probe` の `deferred_pending`。
//! **`BlockReason` を足すときは、これらも見直すこと**（片方だけ配線する ADR-123→128 型の再発を防ぐ）。

/// 退避（または drain の見送り）の理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    /// TSF probe（`pending_tsf`）が進行中。
    TsfProbeInFlight,
    /// raw literal 回収 / GJI reinit retry が deferred の解放権を持っている（INV-F）。
    RawRecoveryOwnsDeferred,
}

/// `plan_blocking` の入力。`raw_recovery_owns` を読んでいないとき（`needs_raw_recovery_read` が偽）は偽を渡す。
///
/// 「`check_raw_recovery=false` かつ `raw_recovery_owns=true`」の組は殻からは来ない（読まないため）が、
/// `plan_blocking` はその組でも raw を無視する（全数表で固定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockingFacts {
    pub has_pending_tsf: bool,
    /// `raw_recovery_owns_deferred()` を見るか（`DeferGate::Exempt` の defer 側だけ偽）。
    pub check_raw_recovery: bool,
    /// `raw_recovery_owns_deferred()` の値。`needs_raw_recovery_read` が偽のときは読まないので偽を渡す。
    pub raw_recovery_owns: bool,
}

/// `raw_recovery_owns_deferred()` を読む必要があるか。殻は真のときだけ読む（元の短絡評価と同じ読む回数）。
#[must_use]
pub const fn needs_raw_recovery_read(check_raw_recovery: bool, has_pending_tsf: bool) -> bool {
    check_raw_recovery && !has_pending_tsf
}

/// probe/recovery が進行中か。`has_pending_tsf` が先で、次に（見る場合のみ）`raw_recovery_owns`。
#[must_use]
pub const fn plan_blocking(facts: BlockingFacts) -> Option<BlockReason> {
    if facts.has_pending_tsf {
        Some(BlockReason::TsfProbeInFlight)
    } else if facts.check_raw_recovery && facts.raw_recovery_owns {
        Some(BlockReason::RawRecoveryOwnsDeferred)
    } else {
        None
    }
}

/// defer 側の決定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferPlan {
    /// 進行中ではない。退避せず通常送信へ。
    NotBlocked,
    /// 進行中だが件数上限を超えるので、退避を諦めて通常送信へ縮退する。
    DegradeCapExceeded(BlockReason),
    /// 退避キューに積む。
    Defer(BlockReason),
}

/// `would_exceed_cap` は `blocking` が `Some` のときだけ意味を持つ（殻は `Some` のときだけ読む）。
#[must_use]
pub const fn plan_defer(blocking: Option<BlockReason>, would_exceed_cap: bool) -> DeferPlan {
    match blocking {
        None => DeferPlan::NotBlocked,
        Some(reason) if would_exceed_cap => DeferPlan::DegradeCapExceeded(reason),
        Some(reason) => DeferPlan::Defer(reason),
    }
}

/// drain-before-send（ADR-123 決定 4-3）の決定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainBeforeSendPlan {
    /// `DeferGate::Exempt`（raw recovery 回収再送・retry）は drain しない（ADR-128）。
    SkipGateExempt,
    /// probe または recovery が進行中（`raw_recovery` は gate に関わらず見る。ADR-128 round4-3）。
    SkipBlocked(BlockReason),
    /// 退避キューが空。
    SkipQueueEmpty,
    /// 新規モーラの前に取り残しを flush する。
    Flush,
}

/// `blocking`・`queue_len` は `gate_enforced` が真のとき（`queue_len` はさらに `blocking` が `None` のとき）だけ
/// 意味を持つ。殻は読まなかった値に `None`・`0` を渡す。
#[must_use]
pub const fn plan_drain_before_send(
    gate_enforced: bool,
    blocking: Option<BlockReason>,
    queue_len: usize,
) -> DrainBeforeSendPlan {
    if !gate_enforced {
        return DrainBeforeSendPlan::SkipGateExempt;
    }
    if let Some(reason) = blocking {
        return DrainBeforeSendPlan::SkipBlocked(reason);
    }
    if queue_len == 0 {
        DrainBeforeSendPlan::SkipQueueEmpty
    } else {
        DrainBeforeSendPlan::Flush
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocking(tsf: bool, check: bool, raw: bool) -> Option<BlockReason> {
        plan_blocking(BlockingFacts {
            has_pending_tsf: tsf,
            check_raw_recovery: check,
            raw_recovery_owns: raw,
        })
    }

    #[test]
    fn plan_blocking_exhaustive() {
        use BlockReason::{RawRecoveryOwnsDeferred as R, TsfProbeInFlight as T};
        // (tsf, check_raw, raw_owns, 期待)
        let table = [
            (false, false, false, None),
            (false, false, true, None), // Exempt は raw を見ない
            (false, true, false, None),
            (false, true, true, Some(R)),
            (true, false, false, Some(T)),
            (true, false, true, Some(T)), // tsf が先
            (true, true, false, Some(T)),
            (true, true, true, Some(T)), // tsf が先
        ];
        for (tsf, check, raw, want) in table {
            assert_eq!(blocking(tsf, check, raw), want, "{tsf} {check} {raw}");
        }
    }

    #[test]
    fn raw_read_is_needed_only_when_checked_and_tsf_idle() {
        assert!(needs_raw_recovery_read(true, false));
        assert!(!needs_raw_recovery_read(true, true));
        assert!(!needs_raw_recovery_read(false, false));
        assert!(!needs_raw_recovery_read(false, true));
    }

    #[test]
    fn plan_defer_exhaustive() {
        for reason in [
            BlockReason::TsfProbeInFlight,
            BlockReason::RawRecoveryOwnsDeferred,
        ] {
            assert_eq!(plan_defer(Some(reason), false), DeferPlan::Defer(reason));
            assert_eq!(
                plan_defer(Some(reason), true),
                DeferPlan::DegradeCapExceeded(reason)
            );
        }
        assert_eq!(plan_defer(None, false), DeferPlan::NotBlocked);
        assert_eq!(plan_defer(None, true), DeferPlan::NotBlocked);
    }

    #[test]
    fn plan_drain_before_send_exhaustive() {
        use DrainBeforeSendPlan::*;
        let t = Some(BlockReason::TsfProbeInFlight);
        let r = Some(BlockReason::RawRecoveryOwnsDeferred);
        assert_eq!(plan_drain_before_send(false, None, 5), SkipGateExempt);
        assert_eq!(plan_drain_before_send(false, t, 0), SkipGateExempt);
        assert_eq!(
            plan_drain_before_send(true, t, 5),
            SkipBlocked(BlockReason::TsfProbeInFlight)
        );
        assert_eq!(
            plan_drain_before_send(true, r, 5),
            SkipBlocked(BlockReason::RawRecoveryOwnsDeferred)
        );
        assert_eq!(plan_drain_before_send(true, None, 0), SkipQueueEmpty);
        assert_eq!(plan_drain_before_send(true, None, 1), Flush);
    }

    /// 純粋側が固定するのは事実の組合せだけ: `check_raw_recovery=false` は raw を無視し、`gate_enforced=false` の drain は
    /// 何もしない。gate → `check_raw_recovery` の対応（Exempt の defer は false、Enforced の drain は true）は殻にあり、
    /// `architecture_guard::deferred_gate_plan_defer_and_drain_windows_share_plan_blocking` が固定する。
    #[test]
    fn defer_and_drain_share_blocking_with_documented_asymmetry() {
        for tsf in [false, true] {
            for raw in [false, true] {
                // 共有: Enforced の defer 側と drain 側は同じ blocking を見る。
                let b = blocking(tsf, true, raw);
                assert_eq!(b.is_some(), tsf || raw);
                assert_eq!(
                    matches!(
                        plan_drain_before_send(true, b, 1),
                        DrainBeforeSendPlan::Flush
                    ),
                    !(tsf || raw)
                );
                assert_eq!(plan_defer(b, false) != DeferPlan::NotBlocked, tsf || raw);
                // 非対称: defer 側の Exempt は raw を見ない。drain 側は Exempt なら何もしない。
                assert_eq!(blocking(tsf, false, raw).is_some(), tsf);
                assert_eq!(
                    plan_drain_before_send(false, blocking(tsf, false, raw), 1),
                    DrainBeforeSendPlan::SkipGateExempt
                );
            }
        }
    }
}
