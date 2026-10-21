//! `runtime/executor.rs` の `execute_relay` / `drain_deferred` / `run_passthrough_pipeline` が下す
//! 「何を即時に実行し、何をキューに積み、何をガードするか」の判断の核（FCIS F3）。
//!
//! 元の分岐・順序を変えずに、判断だけを純粋関数へ出した。実行（`execute_one`・`OutputActiveGuard::begin()`・
//! `spawn_local`・キューへの積み・`TIMER_OUTPUT_GUARD` の set/kill・SendInput）は `executor.rs` の殻に残る。
//! 各 plan は**理由の enum**（variant 名）を返す。現状 journal には載らず、`execute_relay` のログ（`{:?}`）にだけ出る。
//!
//! 固定する辺と対:
//! - **e12**: `Decision::Consume` では Timer だけが即時に実行され、キューを追い越す（`plan_consume_effect`）。
//! - **executor キューの defer 側と drain 側の閾値の対（ADR-156。辺の表の c23 として追加）**:
//!   defer 側（`run_passthrough_pipeline` の `output_in_flight`）と drain 側（`reinject_wait_remaining`）が
//!   **同じ** `output_guard_remaining_ms` を使う。片方だけ条件を足す事故（ADR-123→128）を、
//!   `defer_and_drain_share_output_guard_threshold` と境界値のテストが固定する。
//!   **残る非対称（本モジュールは扱わない）**: defer 側の `has_pending` は `has_pending_tsf_work()` を OR する
//!   （BUG-58）が、drain 側が TSF の保留を見るのは確定キーの KeyDown だけ。ここに条件を足すときは両窓口を見ること。
//! - **c21・c22 は触らない**（`INPUT_DEFER` 側の制約。c21=`defer_during_output`/`replay_later` の post の非対称、
//!   c22=`raw_recovery_owns_deferred()` の gate。F6 などで別途扱う）。
//! - e13（`OutputActiveGuard::begin()` を `spawn_local` の前に取る順序）は殻（`handle_reinject` と
//!   `dispatch_ime_set_open` の async 経路）に残り、本モジュールは触らない（`architecture_guard` で固定）。

use super::physical_disposition::PhysicalKeyDisposition;

// ── execute_relay ──

/// `Decision` の種別（効果の中身は持たない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayDecisionKind {
    PassThrough,
    PassThroughWith,
    Consume,
}

/// `execute_relay` の判断の入力。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayFacts {
    pub kind: RelayDecisionKind,
    pub physical: PhysicalKeyDisposition,
}

/// `execute_relay` の決定。variant 名が理由（現状は `execute_relay` のログに `{:?}` で出るだけで、journal には載らない）。
/// 型が許す（実行の種類 × 理由）20 通りのうち、実際に返す 5 通りだけを持つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayPlan {
    /// 物理キーを抑止して Consume（passthrough パイプラインも reinject も走らせない）。
    PassThroughPhysicalSuppressed,
    /// passthrough パイプラインを走らせ、その結果を返す。
    PassThroughIdle,
    /// 効果をすべてキューへ積み、キーの再注入を末尾に足す。Consumed、`has_pending=true`。
    FlushWithReinject,
    /// 効果をすべてキューへ積む（物理キー抑止のため再注入なし）。Consumed、`has_pending=true`。
    FlushPhysicalSuppressedNoReinject,
    /// 効果ごとに `plan_consume_effect` で即時/キューを分ける。Consumed。
    EngineConsumed,
}

#[must_use]
pub const fn plan_relay(facts: RelayFacts) -> RelayPlan {
    let suppress = matches!(facts.physical, PhysicalKeyDisposition::Suppress);
    match (facts.kind, suppress) {
        (RelayDecisionKind::PassThrough, true) => RelayPlan::PassThroughPhysicalSuppressed,
        (RelayDecisionKind::PassThrough, false) => RelayPlan::PassThroughIdle,
        (RelayDecisionKind::PassThroughWith, true) => RelayPlan::FlushPhysicalSuppressedNoReinject,
        (RelayDecisionKind::PassThroughWith, false) => RelayPlan::FlushWithReinject,
        (RelayDecisionKind::Consume, _) => RelayPlan::EngineConsumed,
    }
}

/// `Consume` の効果 1 つの実行先（e12）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectRoute {
    /// その場で実行する（platform の timer state を常に最新に保つ。キューを追い越す）。
    Immediate,
    /// FIFO のキューへ積む。
    Queue,
}

/// Timer だけを即時に実行し、それ以外はキューへ積む。
#[must_use]
pub const fn plan_consume_effect(is_timer: bool) -> EffectRoute {
    if is_timer {
        EffectRoute::Immediate
    } else {
        EffectRoute::Queue
    }
}

// ── defer 側と drain 側の共有（ADR-156） ──

/// 直近の出力送信からの経過 `elapsed_ms` がガード窓 `guard_ms` の内なら、残りの待ち時間を返す。
/// defer 側（`output_in_flight`）と drain 側（`reinject_wait_remaining`）の唯一の閾値判断。
#[must_use]
pub(crate) const fn output_guard_remaining_ms(elapsed_ms: u64, guard_ms: u64) -> Option<u64> {
    if elapsed_ms < guard_ms {
        Some(guard_ms - elapsed_ms)
    } else {
        None
    }
}

/// defer 側: 出力が in-flight か（`output_guard_remaining_ms` が `Some`）。
#[must_use]
pub const fn output_in_flight(elapsed_ms: u64, guard_ms: u64) -> bool {
    output_guard_remaining_ms(elapsed_ms, guard_ms).is_some()
}

/// drain 側 `reinject_wait_remaining` の入力。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReinjectWaitFacts {
    /// 確定キー（Enter/Space/Escape）の KeyDown で、かつ TSF の保留作業がある。
    pub confirm_held_by_tsf: bool,
    /// 直近の出力送信からの経過。`confirm_held_by_tsf` が真なら使わない（読まなくてよい）。
    pub output_elapsed_ms: u64,
    pub guard_ms: u64,
}

/// 確定キーの TSF 待ちは 10ms、そうでなければ出力ガードの残り。
#[must_use]
pub const fn reinject_wait_remaining(facts: ReinjectWaitFacts) -> Option<u64> {
    if facts.confirm_held_by_tsf {
        return Some(CONFIRM_TSF_WAIT_MS);
    }
    output_guard_remaining_ms(facts.output_elapsed_ms, facts.guard_ms)
}

/// 確定キーが TSF の保留作業の完了を待つ再試行間隔（元の `Some(10)` のリテラル）。
pub(crate) const CONFIRM_TSF_WAIT_MS: u64 = 10;

// ── drain_deferred ──

/// drain が次に処理する項目。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainItem {
    /// `guard_held` に park 済みの `ReinjectKey`。
    Held,
    /// キューの先頭が `ReinjectKey`。
    QueuedReinject,
    /// キューの先頭が `ReinjectKey` 以外。
    QueuedOther,
}

/// この項目で `reinject_wait_remaining` を読む必要があるか。
/// 殻は真のときだけ OS を読む（元の読む回数を変えない）。
#[must_use]
pub const fn drain_needs_wait_check(item: DrainItem, guard_passed: bool) -> bool {
    match item {
        DrainItem::Held => true,
        DrainItem::QueuedReinject => !guard_passed,
        DrainItem::QueuedOther => false,
    }
}

/// drain の 1 項目の決定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrainStep {
    pub action: DrainAction,
    pub reason: DrainReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainAction {
    /// 項目を `guard_held` に park し、`remaining` ms 後の `TIMER_OUTPUT_GUARD` で drain を打ち切る。
    Park { remaining: u64 },
    /// 項目を実行し、`guard_passed` を `guard_passed_after` に更新して続ける。
    Execute { guard_passed_after: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainReason {
    HeldStillGuarded,
    HeldReleased,
    ReinjectStillGuarded,
    ReinjectGuardPassed,
    /// 同一 drain 内で先頭の reinject が guard を通過済み。残りはまとめて送出する（Win スタック防止）。
    ReinjectBatchedAfterGuard,
    /// reinject 以外は `mark_send` を呼ぶので、次の reinject には再びガードを適用する。
    NonReinjectResetsGuard,
}

/// `wait_remaining` は `drain_needs_wait_check` が真のときだけ意味を持つ（偽のときは `None` を渡す）。
#[must_use]
pub const fn plan_drain_step(
    item: DrainItem,
    guard_passed: bool,
    wait_remaining: Option<u64>,
) -> DrainStep {
    match item {
        DrainItem::Held => match wait_remaining {
            Some(remaining) => DrainStep {
                action: DrainAction::Park { remaining },
                reason: DrainReason::HeldStillGuarded,
            },
            None => DrainStep {
                action: DrainAction::Execute {
                    guard_passed_after: true,
                },
                reason: DrainReason::HeldReleased,
            },
        },
        DrainItem::QueuedReinject => {
            if guard_passed {
                return DrainStep {
                    action: DrainAction::Execute {
                        guard_passed_after: true,
                    },
                    reason: DrainReason::ReinjectBatchedAfterGuard,
                };
            }
            match wait_remaining {
                Some(remaining) => DrainStep {
                    action: DrainAction::Park { remaining },
                    reason: DrainReason::ReinjectStillGuarded,
                },
                None => DrainStep {
                    action: DrainAction::Execute {
                        guard_passed_after: true,
                    },
                    reason: DrainReason::ReinjectGuardPassed,
                },
            }
        }
        DrainItem::QueuedOther => DrainStep {
            action: DrainAction::Execute {
                guard_passed_after: false,
            },
            reason: DrainReason::NonReinjectResetsGuard,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: [RelayDecisionKind; 3] = [
        RelayDecisionKind::PassThrough,
        RelayDecisionKind::PassThroughWith,
        RelayDecisionKind::Consume,
    ];
    const PHYS: [PhysicalKeyDisposition; 2] = [
        PhysicalKeyDisposition::Allow,
        PhysicalKeyDisposition::Suppress,
    ];

    #[test]
    fn plan_relay_exhaustive() {
        use PhysicalKeyDisposition::{Allow, Suppress};
        use RelayDecisionKind::*;
        use RelayPlan::*;
        let table = [
            (PassThrough, Allow, PassThroughIdle),
            (PassThrough, Suppress, PassThroughPhysicalSuppressed),
            (PassThroughWith, Allow, FlushWithReinject),
            (PassThroughWith, Suppress, FlushPhysicalSuppressedNoReinject),
            (Consume, Allow, EngineConsumed),
            (Consume, Suppress, EngineConsumed),
        ];
        assert_eq!(table.len(), KINDS.len() * PHYS.len());
        for (kind, physical, plan) in table {
            assert_eq!(
                plan_relay(RelayFacts { kind, physical }),
                plan,
                "{kind:?} {physical:?}"
            );
        }
    }

    /// e12: Timer だけが即時でキューを追い越し、他はキュー。
    #[test]
    fn e12_consume_runs_only_timer_immediately() {
        assert_eq!(plan_consume_effect(true), EffectRoute::Immediate);
        assert_eq!(plan_consume_effect(false), EffectRoute::Queue);
    }

    /// c23（executor キューの閾値の対）: defer 側の in-flight と drain 側の待ちが、確定キー待ち以外では同じ閾値を使う。
    #[test]
    fn defer_and_drain_share_output_guard_threshold() {
        for guard_ms in [0_u64, 1, 50, 350] {
            for elapsed in [0_u64, 1, 49, 50, 51, 349, 350, 351, u64::MAX] {
                let drain = reinject_wait_remaining(ReinjectWaitFacts {
                    confirm_held_by_tsf: false,
                    output_elapsed_ms: elapsed,
                    guard_ms,
                });
                assert_eq!(
                    output_in_flight(elapsed, guard_ms),
                    drain.is_some(),
                    "elapsed={elapsed} guard={guard_ms}"
                );
                if let Some(r) = drain {
                    assert_eq!(r, guard_ms - elapsed);
                }
            }
        }
    }

    /// 境界（`<` と `<=` の取り違えを検出する。リテラルで固定）。
    #[test]
    fn output_guard_boundary_literals() {
        assert_eq!(output_guard_remaining_ms(49, 50), Some(1));
        assert_eq!(output_guard_remaining_ms(50, 50), None);
        assert_eq!(output_guard_remaining_ms(51, 50), None);
        assert_eq!(output_guard_remaining_ms(0, 50), Some(50));
        assert_eq!(output_guard_remaining_ms(u64::MAX, 50), None);
        assert!(output_in_flight(49, 50));
        assert!(!output_in_flight(50, 50));
        assert!(!output_in_flight(51, 50));
        let wait = |elapsed| {
            reinject_wait_remaining(ReinjectWaitFacts {
                confirm_held_by_tsf: false,
                output_elapsed_ms: elapsed,
                guard_ms: 50,
            })
        };
        assert_eq!(wait(49), Some(1));
        assert_eq!(wait(50), None);
        assert_eq!(wait(51), None);
    }

    #[test]
    fn reinject_wait_confirm_key_takes_priority() {
        for elapsed in [0, 49, 50, u64::MAX] {
            assert_eq!(
                reinject_wait_remaining(ReinjectWaitFacts {
                    confirm_held_by_tsf: true,
                    output_elapsed_ms: elapsed,
                    guard_ms: 50,
                }),
                Some(10)
            );
        }
    }

    #[test]
    fn needs_wait_check_exhaustive() {
        for passed in [false, true] {
            assert!(drain_needs_wait_check(DrainItem::Held, passed));
            assert_eq!(
                drain_needs_wait_check(DrainItem::QueuedReinject, passed),
                !passed
            );
            assert!(!drain_needs_wait_check(DrainItem::QueuedOther, passed));
        }
    }

    #[test]
    fn plan_drain_step_exhaustive() {
        use DrainAction::{Execute, Park};
        use DrainReason::*;
        let ex = |b| Execute {
            guard_passed_after: b,
        };
        // (item, guard_passed, wait, action, reason)
        let table = [
            (
                DrainItem::Held,
                false,
                Some(7),
                Park { remaining: 7 },
                HeldStillGuarded,
            ),
            (DrainItem::Held, false, None, ex(true), HeldReleased),
            (
                DrainItem::Held,
                true,
                Some(7),
                Park { remaining: 7 },
                HeldStillGuarded,
            ),
            (DrainItem::Held, true, None, ex(true), HeldReleased),
            (
                DrainItem::QueuedReinject,
                false,
                Some(3),
                Park { remaining: 3 },
                ReinjectStillGuarded,
            ),
            (
                DrainItem::QueuedReinject,
                false,
                None,
                ex(true),
                ReinjectGuardPassed,
            ),
            // guard 通過済みでは wait は読まれない（None を渡す）。
            (
                DrainItem::QueuedReinject,
                true,
                None,
                ex(true),
                ReinjectBatchedAfterGuard,
            ),
            (
                DrainItem::QueuedOther,
                false,
                None,
                ex(false),
                NonReinjectResetsGuard,
            ),
            (
                DrainItem::QueuedOther,
                true,
                None,
                ex(false),
                NonReinjectResetsGuard,
            ),
        ];
        for (item, passed, wait, action, reason) in table {
            assert_eq!(
                plan_drain_step(item, passed, wait),
                DrainStep { action, reason },
                "{item:?} passed={passed} wait={wait:?}"
            );
        }
    }

    /// 先頭が guard で park され、再開後は残りが wait を読まずに一括送出される。
    #[test]
    fn drain_resumes_after_park_and_batches_rest() {
        let first = plan_drain_step(DrainItem::Held, false, Some(5));
        assert_eq!(first.action, DrainAction::Park { remaining: 5 });
        let resumed = plan_drain_step(DrainItem::Held, false, None);
        let DrainAction::Execute { guard_passed_after } = resumed.action else {
            panic!("再開は Execute");
        };
        for _ in 0..3 {
            assert!(!drain_needs_wait_check(
                DrainItem::QueuedReinject,
                guard_passed_after
            ));
            assert_eq!(
                plan_drain_step(DrainItem::QueuedReinject, guard_passed_after, None).reason,
                DrainReason::ReinjectBatchedAfterGuard
            );
        }
    }

    /// Win_DOWN→X_DOWN→X_UP→Win_UP の連続 reinject: 先頭だけが guard を受け、残りは一括送出される。
    #[test]
    fn drain_batches_consecutive_reinjects_after_first_guard() {
        let mut passed = false;
        let mut parks = 0;
        for _ in 0..4 {
            let wait = drain_needs_wait_check(DrainItem::QueuedReinject, passed).then_some(None);
            let step = plan_drain_step(DrainItem::QueuedReinject, passed, wait.flatten());
            match step.action {
                DrainAction::Park { .. } => parks += 1,
                DrainAction::Execute { guard_passed_after } => passed = guard_passed_after,
            }
        }
        assert_eq!(parks, 0);
        assert!(passed);
        // 間に reinject 以外が入ると次の reinject は再びガードされる。
        let step = plan_drain_step(DrainItem::QueuedOther, passed, None);
        assert_eq!(
            step.action,
            DrainAction::Execute {
                guard_passed_after: false
            }
        );
        assert!(drain_needs_wait_check(DrainItem::QueuedReinject, false));
    }
}
