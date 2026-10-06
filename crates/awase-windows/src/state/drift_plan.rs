//! drift correction の**実行計画**を決める純粋関数（FCIS F4）。
//!
//! `runtime/ime_refresh.rs::ir_apply_drift_correction` は以前、観測の読み取り・判断・IME への書き込みが
//! 1 つの関数に絡んでいた。これを、
//!
//! 1. observe（殻）: [`DriftFacts`] を作る（観測・`desired_open`・時刻・進行中の試行の写し）。
//! 2. [`decide_drift_plan`]（核、この module）: 何をするかと**その理由**を [`DriftPlan`] で返す。
//! 3. execute（殻）: 計画どおりに journal・タイマー・IME への書き込みを行う。
//!
//! に分けた。**判断の材料と順序は元のコードのまま**（ADR-080 の `FeedbackPolicy` による有界の再送・
//! BUG-43 の無限再送の防止・BUG-163 の「授権が下りない補正は検知しない」・BUG-68 の再武装クールダウン）。
//! 検知そのもの（`desired` と観測のずれ）は [`super::drift_correction::check_drift_correction`] が担い、
//! この module はその**結果を受けて**、再送・打ち切り・収束・保留のどれにするかを決める。二重に判断しない。
//!
//! 核は OS・壁時計・グローバルに触れない。時刻は [`DriftFacts::now`] で受ける。

use super::drift_correction::{DriftCorrection, NoDrift, OmissionBasis};
use super::event_origin::{EventOrigin, Generation};
use super::ime_actuation::{blind_rearm_cooldown_elapsed, ActuationAction, FeedbackPolicy};

/// 進行中の actuation 試行（`runtime/ime_actuation.rs::Actuation`）の読み取り用の写し。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActuationSnapshot {
    pub target: bool,
    pub policy: FeedbackPolicy,
    pub attempts: u32,
    pub sent_at: std::time::Instant,
    pub gave_up_at: Option<std::time::Instant>,
    pub origin: EventOrigin,
}

/// 目標値 `target` の試行を返す。既存の試行の `target` が同じなら再利用（`policy` は無視、ADR-080 不変条件4）、
/// 違う・無いなら新規（attempts=0・`sent_at=now`・`gave_up_at=None`・世代は `INITIAL`）。
/// 戻り値の `bool` は「既存を再利用したか」（`false` なら殻が新しい試行を据える）。
///
/// observe（読み戻しの `since` を決める）と [`decide_drift_plan`] が同じ関数を通る（解決が二通りにならない）。
#[must_use]
pub fn resolve_actuation(
    active: Option<&ActuationSnapshot>,
    target: bool,
    policy: FeedbackPolicy,
    now: std::time::Instant,
) -> (ActuationSnapshot, bool) {
    match active {
        Some(a) if a.target == target => (*a, true),
        _ => (
            ActuationSnapshot {
                target,
                policy,
                attempts: 0,
                sent_at: now,
                gave_up_at: None,
                origin: policy.origin(Generation::INITIAL),
            },
            false,
        ),
    }
}

/// [`decide_drift_plan`] への入力（所有型の事実）。
///
/// `imm_cross` 以降は、`drift` が `Some` かつ `settling` でないときだけ observe が読む（それ以外は既定値の
/// まま。[`decide_drift_plan`] はその場合それらを見ずに返す）。
#[derive(Debug, Clone, Copy)]
pub struct DriftFacts {
    pub now: std::time::Instant,
    /// `engine.is_user_enabled()`。
    pub engine_enabled: bool,
    /// `belief.is_japanese_ime()`。
    pub japanese_ime: bool,
    /// [`super::drift_correction::evaluate_drift`] の結果（補正が要らないときはその理由）。
    pub drift: Result<DriftCorrection, NoDrift>,
    /// フォーカス遷移の settle 中（`ime_apply_should_defer`）。
    pub settling: bool,
    /// 進行中の試行（無ければ `None`）。
    pub active: Option<ActuationSnapshot>,
    /// 新しい試行を作るときの方針（`AppImePolicy::default_feedback`）。
    pub default_policy: FeedbackPolicy,
    /// `can_use_imm32_cross_process()`。書き込み経路（ImmCross か strategy chain か）を決める。
    pub imm_cross: bool,
    /// 授権が下りない（`ActuationOrder::would_have_blocked`、BUG-163）。`imm_cross` のときだけ意味を持つ。
    pub warrant_would_block: bool,
    /// このフォーカスで診断バルーンを出し済み（`drift_giveup_notified_this_focus`）。
    pub diag_already_notified: bool,
    /// `Read` の収束確認（`read_back(.., sent_at, Converged{desired})`）。
    pub converged: bool,
    /// give-up 後の再武装の証拠（`read_back(.., gave_up_at, AnyFreshEvidence)` が `ExternalChange`、BUG-68）。
    pub fresh_evidence_after_giveup: bool,
}

/// 何もしない理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftIdle {
    /// ユーザーがエンジンを止めている。
    EngineDisabled,
    /// 日本語 IME ではない。
    NotJapaneseIme,
    /// 補正が要るずれが無い（理由は `evaluate_drift` の各早期 return、[`NoDrift`]）。
    NoDrift(NoDrift),
}

impl DriftIdle {
    /// 省略の根拠。
    #[must_use]
    pub const fn basis(self) -> OmissionBasis {
        match self {
            Self::EngineDisabled => OmissionBasis::EngineSetting,
            Self::NotJapaneseIme => OmissionBasis::Belief,
            Self::NoDrift(reason) => reason.basis(),
        }
    }
}

/// give-up（`Blind` が `max_attempts` 到達）したときの、その tick の扱い。いずれも再送しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GiveUpPark {
    /// この tick で初めて到達。境界時刻を刻んで parked にする。
    FirstTime,
    /// parked 済みだが再武装のクールダウン未経過（BUG-68）。再武装の判定自体をしない。
    CooldownPending,
    /// クールダウン経過後、`gave_up_at` 以降に新しい観測があった。試行を破棄して次の tick でやり直す。
    Rearm,
    /// クールダウン経過後だが新しい観測が無い。parked のまま。
    StillParked,
}

/// 検知後に行うことの分類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftStep {
    /// 授権が下りない ImmCross の補正は検知しない（BUG-163）。
    SkipWarrantWouldBlock,
    /// `Blind` の打ち切り。観測ストアへは何も書かない（BUG-33 型の収束偽装の防止）。
    GiveUp(GiveUpPark),
    /// `Read` が収束していた。試行を破棄する。
    Confirmed,
    /// 実際に送る。
    Send(SendPath),
}

/// 書き込み経路。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendPath {
    /// `set_ime_open_ordered`（IMM32 クロスプロセス）。実際に書いたときだけ `applied` を `Optimistic` にする。
    ImmCross,
    /// strategy chain（`apply_ime_open_with_view`、Blacklist/TsfNative）。
    StrategyChain,
}

impl GiveUpPark {
    /// 打ち切り・parked の根拠。
    #[must_use]
    pub const fn basis(self) -> OmissionBasis {
        match self {
            Self::FirstTime => OmissionBasis::AttemptBudget,
            Self::CooldownPending => OmissionBasis::Cooldown,
            Self::Rearm | Self::StillParked => OmissionBasis::FreshRead,
        }
    }
}

impl DriftStep {
    /// 送らない・打ち切る・収束とみなす決定の根拠。実際に送る `Send` は `None`。
    #[must_use]
    pub const fn basis(self) -> Option<OmissionBasis> {
        match self {
            Self::SkipWarrantWouldBlock => Some(OmissionBasis::Warrant),
            Self::GiveUp(park) => Some(park.basis()),
            Self::Confirmed => Some(OmissionBasis::FreshRead),
            Self::Send(_) => None,
        }
    }
}

/// 検知後の計画。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriftAct {
    pub drift: DriftCorrection,
    /// この tick で使う試行（`install_actuation` が真なら殻が据える）。
    pub actuation: ActuationSnapshot,
    pub install_actuation: bool,
    /// ユーザー向け診断（バルーン・journal）を出す。
    pub notify_diagnostic: bool,
    pub step: DriftStep,
}

/// [`decide_drift_plan`] の出力。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftPlan {
    Idle(DriftIdle),
    /// settle 中。settle 明けに再試行を予約する。試行は触らない。
    DeferToSettle {
        drift: DriftCorrection,
    },
    Act(DriftAct),
}

impl DriftPlan {
    /// 送らない・打ち切る・収束とみなす・保留する決定の根拠（`Send` は `None`）。殻はこれをログに出すだけ。
    #[must_use]
    pub const fn basis(&self) -> Option<OmissionBasis> {
        match self {
            Self::Idle(idle) => Some(idle.basis()),
            Self::DeferToSettle { .. } => Some(OmissionBasis::FocusSettle),
            Self::Act(act) => act.step.basis(),
        }
    }
}

/// 診断を出すか。ずれの継続時間が再武装クールダウン以上で、このフォーカスで未通知のとき（ADR-132 Phase 1）。
const fn should_notify_diagnostic(duration_ms: u64, already_notified: bool) -> bool {
    duration_ms >= crate::tuning::DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS && !already_notified
}

/// give-up 後のその tick の扱い（BUG-68: クールダウンが先、再武装の証拠はその後）。
fn park_after_giveup(
    gave_up_at: Option<std::time::Instant>,
    now: std::time::Instant,
    fresh_evidence: bool,
) -> GiveUpPark {
    gave_up_at.map_or(GiveUpPark::FirstTime, |gave_up_at| {
        let elapsed = blind_rearm_cooldown_elapsed(
            gave_up_at,
            now,
            crate::tuning::DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS,
        );
        match (elapsed, fresh_evidence) {
            (false, _) => GiveUpPark::CooldownPending,
            (true, true) => GiveUpPark::Rearm,
            (true, false) => GiveUpPark::StillParked,
        }
    })
}

/// 検知後の計画を決める。**元の `ir_apply_drift_correction` の判断の順序のまま**:
/// 稼働条件 → ずれの有無 → settle → 試行の解決 → 授権 → 診断 → 方針ごとの打ち切り/収束/送信。
#[must_use]
pub fn decide_drift_plan(f: &DriftFacts) -> DriftPlan {
    if !f.engine_enabled {
        return DriftPlan::Idle(DriftIdle::EngineDisabled);
    }
    if !f.japanese_ime {
        return DriftPlan::Idle(DriftIdle::NotJapaneseIme);
    }
    let drift = match f.drift {
        Ok(drift) => drift,
        Err(reason) => return DriftPlan::Idle(DriftIdle::NoDrift(reason)),
    };
    if f.settling {
        return DriftPlan::DeferToSettle { drift };
    }

    let (actuation, reused) =
        resolve_actuation(f.active.as_ref(), drift.desired, f.default_policy, f.now);
    let install_actuation = !reused;

    if f.imm_cross && f.warrant_would_block {
        return DriftPlan::Act(DriftAct {
            drift,
            actuation,
            install_actuation,
            notify_diagnostic: false,
            step: DriftStep::SkipWarrantWouldBlock,
        });
    }

    let notify_diagnostic = should_notify_diagnostic(drift.duration_ms, f.diag_already_notified);
    let send = DriftStep::Send(if f.imm_cross {
        SendPath::ImmCross
    } else {
        SendPath::StrategyChain
    });
    let step = match actuation.policy {
        FeedbackPolicy::Blind { .. } => {
            if actuation.policy.decide_action(actuation.attempts) == ActuationAction::GiveUp {
                DriftStep::GiveUp(park_after_giveup(
                    actuation.gave_up_at,
                    f.now,
                    f.fresh_evidence_after_giveup,
                ))
            } else {
                send
            }
        }
        FeedbackPolicy::Read { .. } => {
            if f.converged {
                DriftStep::Confirmed
            } else {
                send
            }
        }
    };

    DriftPlan::Act(DriftAct {
        drift,
        actuation,
        install_actuation,
        notify_diagnostic,
        step,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ime_event::{ObservationConfidence, ObservationSource};
    use std::time::{Duration, Instant};

    const MAX: u32 = 5;

    fn blind() -> FeedbackPolicy {
        FeedbackPolicy::Blind {
            max_attempts: MAX,
            backoff: Duration::from_millis(0),
        }
    }

    fn read() -> FeedbackPolicy {
        FeedbackPolicy::Read {
            source: ObservationSource::ObserverPoll,
            deadline: Duration::from_millis(300),
        }
    }

    fn drift(desired: bool, duration_ms: u64) -> DriftCorrection {
        DriftCorrection {
            desired,
            observed: !desired,
            duration_ms,
            source: ObservationSource::ObserverPoll,
            confidence: ObservationConfidence::Medium,
        }
    }

    fn snap(
        policy: FeedbackPolicy,
        target: bool,
        attempts: u32,
        sent_at: Instant,
        gave_up_at: Option<Instant>,
    ) -> ActuationSnapshot {
        ActuationSnapshot {
            target,
            policy,
            attempts,
            sent_at,
            gave_up_at,
            origin: policy.origin(Generation::INITIAL),
        }
    }

    fn facts(now: Instant) -> DriftFacts {
        DriftFacts {
            now,
            engine_enabled: true,
            japanese_ime: true,
            drift: Ok(drift(false, 0)),
            settling: false,
            active: None,
            default_policy: blind(),
            imm_cross: false,
            warrant_would_block: false,
            diag_already_notified: false,
            converged: false,
            fresh_evidence_after_giveup: false,
        }
    }

    fn act(f: &DriftFacts) -> DriftAct {
        match decide_drift_plan(f) {
            DriftPlan::Act(a) => a,
            other => panic!("Act を期待したが {other:?}"),
        }
    }

    #[test]
    fn idle_when_engine_disabled_or_not_japanese_even_with_drift() {
        let now = Instant::now();
        for (engine_enabled, japanese_ime) in [(false, true), (true, false), (false, false)] {
            let f = DriftFacts {
                engine_enabled,
                japanese_ime,
                ..facts(now)
            };
            let want = if engine_enabled {
                DriftIdle::NotJapaneseIme
            } else {
                DriftIdle::EngineDisabled
            };
            let plan = decide_drift_plan(&f);
            assert_eq!(plan, DriftPlan::Idle(want));
            // 読み取り元から: エンジン設定(`is_user_enabled`)が先、次に IME の信念(`is_japanese_ime`)。
            let basis = if engine_enabled {
                OmissionBasis::Belief
            } else {
                OmissionBasis::EngineSetting
            };
            assert_eq!(plan.basis(), Some(basis));
        }
    }

    #[test]
    fn idle_when_no_drift_carries_the_reason_and_its_basis() {
        for reason in [
            NoDrift::NotExplicitIntent,
            NoDrift::NotDrifting,
            NoDrift::NoTrustedObservation,
            NoDrift::StaleObservation,
            NoDrift::ObservationMatchesDesired,
        ] {
            let f = DriftFacts {
                drift: Err(reason),
                ..facts(Instant::now())
            };
            let plan = decide_drift_plan(&f);
            // 理由はそのまま運ばれる（根拠の正しさは `evaluate_drift` の入力から確かめるテスト側）。
            assert_eq!(plan, DriftPlan::Idle(DriftIdle::NoDrift(reason)));
            assert!(plan.basis().is_some());
        }
    }

    #[test]
    fn every_omission_names_its_basis_and_only_send_has_none() {
        let now = Instant::now();
        let cool = Duration::from_millis(crate::tuning::DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS);
        let parked = Some(snap(blind(), false, MAX, now, Some(now)));
        let cases = [
            (
                DriftFacts {
                    settling: true,
                    ..facts(now)
                },
                Some(OmissionBasis::FocusSettle),
            ),
            (
                DriftFacts {
                    imm_cross: true,
                    warrant_would_block: true,
                    ..facts(now)
                },
                Some(OmissionBasis::Warrant),
            ),
            (
                DriftFacts {
                    active: Some(snap(blind(), false, MAX, now, None)),
                    ..facts(now)
                },
                Some(OmissionBasis::AttemptBudget),
            ),
            (
                DriftFacts {
                    active: parked,
                    ..facts(now)
                },
                Some(OmissionBasis::Cooldown),
            ),
            (
                DriftFacts {
                    active: parked,
                    fresh_evidence_after_giveup: true,
                    ..facts(now + cool)
                },
                Some(OmissionBasis::FreshRead),
            ),
            (
                // クールダウン後で新しい読み戻しが無い（parked のまま）も `FreshRead`。
                DriftFacts {
                    active: parked,
                    fresh_evidence_after_giveup: false,
                    ..facts(now + cool)
                },
                Some(OmissionBasis::FreshRead),
            ),
            (
                DriftFacts {
                    active: Some(snap(read(), false, 0, now, None)),
                    converged: true,
                    ..facts(now)
                },
                Some(OmissionBasis::FreshRead),
            ),
            (facts(now), None),
        ];
        for (f, want) in cases {
            assert_eq!(decide_drift_plan(&f).basis(), want, "{f:?}");
        }
    }

    #[test]
    fn settling_defers_without_touching_actuation() {
        let f = DriftFacts {
            settling: true,
            // settle 中は以降の事実を見ない（警告・授権・収束がどうでも保留）。
            imm_cross: true,
            warrant_would_block: true,
            converged: true,
            ..facts(Instant::now())
        };
        assert_eq!(
            decide_drift_plan(&f),
            DriftPlan::DeferToSettle {
                drift: drift(false, 0)
            }
        );
    }

    #[test]
    fn new_target_installs_a_fresh_actuation_and_sends_via_strategy_chain() {
        let now = Instant::now();
        // 既存は target が違う → 作り直し（attempts は 0 に戻る）。
        let f = DriftFacts {
            active: Some(snap(blind(), true, 4, now, None)),
            ..facts(now)
        };
        let a = act(&f);
        assert!(a.install_actuation);
        assert_eq!(a.actuation.attempts, 0);
        assert!(!a.actuation.target);
        assert_eq!(a.step, DriftStep::Send(SendPath::StrategyChain));
    }

    #[test]
    fn same_target_reuses_actuation_and_ignores_new_policy() {
        let now = Instant::now();
        let existing = snap(read(), false, 2, now, None);
        let f = DriftFacts {
            active: Some(existing),
            default_policy: blind(),
            ..facts(now)
        };
        let a = act(&f);
        assert!(!a.install_actuation);
        assert_eq!(a.actuation, existing);
        assert_eq!(a.step, DriftStep::Send(SendPath::StrategyChain));
    }

    #[test]
    fn warrant_block_applies_only_to_imm_cross_and_suppresses_diagnostic() {
        let now = Instant::now();
        let long = drift(false, 10_000);
        let blocked_imm = DriftFacts {
            drift: Ok(long),
            imm_cross: true,
            warrant_would_block: true,
            ..facts(now)
        };
        let a = act(&blocked_imm);
        assert_eq!(a.step, DriftStep::SkipWarrantWouldBlock);
        assert!(!a.notify_diagnostic, "BUG-163: 検知（診断）へ進めない");
        assert!(
            a.install_actuation,
            "試行の据え付けは授権判定より前（元の順序）"
        );

        // ImmCross でなければ授権は見ない（元は `can_use_imm32_cross_process() && ..`）。
        let blocked_chain = DriftFacts {
            imm_cross: false,
            ..blocked_imm
        };
        assert_eq!(
            act(&blocked_chain).step,
            DriftStep::Send(SendPath::StrategyChain)
        );
        // 授権が下りれば ImmCross で送る。
        let ok_imm = DriftFacts {
            warrant_would_block: false,
            ..blocked_imm
        };
        assert_eq!(act(&ok_imm).step, DriftStep::Send(SendPath::ImmCross));
    }

    #[test]
    fn diagnostic_needs_long_drift_and_not_yet_notified() {
        let now = Instant::now();
        let cool = crate::tuning::DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS;
        for (dur, notified, want) in [
            (cool - 1, false, false),
            (cool, false, true),
            (cool, true, false),
            (cool + 1, true, false),
        ] {
            let f = DriftFacts {
                drift: Ok(drift(false, dur)),
                diag_already_notified: notified,
                ..facts(now)
            };
            assert_eq!(
                act(&f).notify_diagnostic,
                want,
                "dur={dur} notified={notified}"
            );
        }
    }

    #[test]
    fn blind_sends_below_max_and_gives_up_at_max_without_ever_resuming() {
        let now = Instant::now();
        for attempts in 0..MAX {
            let f = DriftFacts {
                active: Some(snap(blind(), false, attempts, now, None)),
                ..facts(now)
            };
            assert_eq!(
                act(&f).step,
                DriftStep::Send(SendPath::StrategyChain),
                "{attempts}"
            );
        }
        for attempts in [MAX, MAX + 1, MAX + 100] {
            for converged in [false, true] {
                let f = DriftFacts {
                    active: Some(snap(blind(), false, attempts, now, None)),
                    converged, // Blind は収束確認をしない。
                    ..facts(now)
                };
                assert_eq!(
                    act(&f).step,
                    DriftStep::GiveUp(GiveUpPark::FirstTime),
                    "BUG-43: max 以上で Send に戻らない attempts={attempts}"
                );
            }
        }
    }

    #[test]
    fn blind_parked_rearm_depends_on_cooldown_then_fresh_evidence() {
        let gave_up = Instant::now();
        let cool = Duration::from_millis(crate::tuning::DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS);
        let before = gave_up + cool - Duration::from_millis(1);
        let after = gave_up + cool;
        for fresh in [false, true] {
            let active = Some(snap(blind(), false, MAX, gave_up, Some(gave_up)));
            let pending = DriftFacts {
                active,
                fresh_evidence_after_giveup: fresh,
                ..facts(before)
            };
            assert_eq!(
                act(&pending).step,
                DriftStep::GiveUp(GiveUpPark::CooldownPending),
                "BUG-68: クールダウン中は証拠があっても再武装しない"
            );
            let elapsed = DriftFacts {
                active,
                fresh_evidence_after_giveup: fresh,
                ..facts(after)
            };
            let want = if fresh {
                GiveUpPark::Rearm
            } else {
                GiveUpPark::StillParked
            };
            assert_eq!(act(&elapsed).step, DriftStep::GiveUp(want));
        }
    }

    #[test]
    fn read_confirms_when_converged_and_otherwise_resends_regardless_of_attempts() {
        let now = Instant::now();
        for attempts in [0, MAX, MAX + 50] {
            let active = Some(snap(read(), false, attempts, now, None));
            let unconverged = DriftFacts {
                active,
                default_policy: read(),
                ..facts(now)
            };
            assert_eq!(
                act(&unconverged).step,
                DriftStep::Send(SendPath::StrategyChain)
            );
            let converged = DriftFacts {
                converged: true,
                ..unconverged
            };
            assert_eq!(act(&converged).step, DriftStep::Confirmed);
            let imm = DriftFacts {
                imm_cross: true,
                ..unconverged
            };
            assert_eq!(act(&imm).step, DriftStep::Send(SendPath::ImmCross));
        }
    }

    /// 全入力の組合せ（真偽 8 軸 × 方針 2 × attempts 3 × gave_up_at 3 × 時刻 2 × target 2）で、
    /// 計画が壊してはならない性質を検査する。
    #[test]
    fn invariants_hold_over_the_whole_input_grid() {
        let base = Instant::now();
        let cool = Duration::from_millis(crate::tuning::DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS);
        let gave = base + Duration::from_millis(10);
        let bools = [false, true];
        for mask in 0u32..256 {
            let b = |i: u32| mask & (1 << i) != 0;
            for policy in [blind(), read()] {
                for attempts in [0, MAX - 1, MAX] {
                    for gave_up_at in [None, Some(gave)] {
                        for now in [gave + Duration::from_millis(1), gave + cool] {
                            for target in bools {
                                let f = DriftFacts {
                                    now,
                                    engine_enabled: b(0),
                                    japanese_ime: b(1),
                                    drift: b(2)
                                        .then(|| drift(false, if b(3) { 10_000 } else { 0 }))
                                        .ok_or(NoDrift::NotDrifting),
                                    settling: b(4),
                                    active: Some(snap(policy, target, attempts, base, gave_up_at)),
                                    default_policy: policy,
                                    imm_cross: b(5),
                                    warrant_would_block: b(6),
                                    diag_already_notified: false,
                                    converged: b(7),
                                    fresh_evidence_after_giveup: b(7),
                                };
                                check_invariants(&f);
                            }
                        }
                    }
                }
            }
        }
    }

    fn check_invariants(f: &DriftFacts) {
        let plan = decide_drift_plan(f);
        // 省略の根拠は、実際に送る `Send` 以外の全ての計画にある（E1）。
        assert_eq!(
            plan.basis().is_none(),
            matches!(
                plan,
                DriftPlan::Act(DriftAct {
                    step: DriftStep::Send(_),
                    ..
                })
            ),
            "{f:?}"
        );
        let active_ok = f.engine_enabled && f.japanese_ime;
        let d = match f.drift {
            Ok(d) if active_ok => d,
            _ => {
                assert!(matches!(plan, DriftPlan::Idle(_)), "{f:?}");
                return;
            }
        };
        if f.settling {
            assert_eq!(plan, DriftPlan::DeferToSettle { drift: d }, "{f:?}");
            return;
        }
        let DriftPlan::Act(a) = plan else {
            panic!("Act を期待: {f:?} → {plan:?}")
        };
        // 試行は desired の向き。同じ向きなら既存を再利用する。
        assert_eq!(a.actuation.target, d.desired);
        assert_eq!(
            a.install_actuation,
            !matches!(f.active, Some(x) if x.target == d.desired)
        );
        if let DriftStep::Send(path) = a.step {
            // 送るのは授権が下りていて、（Blind なら）上限に達していないときだけ。
            assert!(!(f.imm_cross && f.warrant_would_block), "{f:?}");
            assert_eq!(path == SendPath::ImmCross, f.imm_cross);
            if matches!(a.actuation.policy, FeedbackPolicy::Blind { .. }) {
                assert!(a.actuation.attempts < MAX, "BUG-43 {f:?}");
            } else {
                assert!(!f.converged, "{f:?}");
            }
        }
        if a.step == DriftStep::SkipWarrantWouldBlock {
            assert!(f.imm_cross && f.warrant_would_block && !a.notify_diagnostic);
        }
        // BUG-68: クールダウン中は、新しい観測の証拠があっても再武装しない。
        let cool = Duration::from_millis(crate::tuning::DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS);
        let in_cooldown = matches!(a.actuation.policy, FeedbackPolicy::Blind { .. })
            && a.actuation.attempts >= MAX
            && a.actuation.gave_up_at.is_some_and(|g| f.now < g + cool);
        // 授権で見送る補正は、方針の判断（打ち切り/再武装）より前に返る。
        if in_cooldown && a.step != DriftStep::SkipWarrantWouldBlock {
            assert_eq!(
                a.step,
                DriftStep::GiveUp(GiveUpPark::CooldownPending),
                "BUG-68 {f:?}"
            );
        }
        // 再武装は parked 済みのときだけ。
        if a.step == DriftStep::GiveUp(GiveUpPark::Rearm) {
            assert!(a.actuation.gave_up_at.is_some());
        }
    }
}
