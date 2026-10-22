//! ADR-191（IME が状態の正、awase は書かずに観測・予測に追随する）の不変条件の検査。
//!
//! 各検査は違反を全部集めて `Err(説明)` を返す（最初の1件で止めない）。シナリオ側は
//! `assert_ok(h, check(h))` で経過（`Harness::trace`）と一緒に表示する。

use std::time::{Duration, Instant};

use awase::engine::InputModeState;
use awase_windows::state::app_ime_policy::AppImePolicy;
use awase_windows::state::drift_correction::DriftCorrection;
use awase_windows::state::drift_plan::{
    decide_drift_plan, ActuationSnapshot, DriftFacts, DriftPlan, DriftStep,
};
use awase_windows::state::event_origin::Generation;
use awase_windows::state::ime_actuation::FeedbackPolicy;
use awase_windows::state::ime_event::{ObservationConfidence, ObservationSource};

use super::harness::{Harness, PredictionRecord};

/// 違反があれば経過つきで panic する。
pub fn assert_ok(h: &Harness, result: Result<(), String>) {
    if let Err(msg) = result {
        panic!("{msg}\n経過:\n{}", h.trace());
    }
}

/// **P1**: 明示意図が無い間に、warrant の下りた書き込み命令で IME の実状態を変えない。
///
/// タスクの原文は「明示意図が無く、観測が desired と一致（または desired が観測に揃えられた）間は、
/// warrant が下りる書き込み命令は出ない」。これを文字どおりに検査すると、正常系で Engine が
/// 活性化したときの書き込み（IME は既に開いていて、書いても実状態は変わらない。かつての
/// `SetOpen(true, ActivationSync)` がこれに当たった。ADR-213 P2c で撤去）まで違反になりうるため、
/// 「**実状態と異なる値**を warrant 付きで書く命令」に絞っている（書いても何も変わらない命令は
/// ADR-191 の「awase は書かない」を実害の意味で破らない）。
pub fn p1_no_warranted_write_without_intent(h: &Harness) -> Result<(), String> {
    let bad: Vec<String> = h
        .writes
        .iter()
        .filter(|w| !w.explicit_intent && w.warrant.is_some() && w.open != w.truth_open)
        .map(|w| {
            format!(
                "  step#{} t={}ms {:?}: 明示意図なしで open={} を warrant({:?}) 付きで書く（実IMEは open={}）",
                w.step,
                w.at_ms,
                w.origin,
                w.open,
                w.warrant.as_ref().map(|x| &x.basis),
                w.truth_open
            )
        })
        .collect();
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "P1 違反（ADR-191 決定1: 明示意図なしに awase が IME の開閉を書き換えた）:\n{}",
            bad.join("\n")
        ))
    }
}

/// 入力モードの belief が「かな入力系（Engine を有効にする側）」か。
const fn mode_is_native(mode: InputModeState) -> bool {
    !matches!(mode, InputModeState::ObservedEisu)
}

/// 予測1件が擬似 IME の真の結果と食い違う点を返す（無ければ `None`）。
///
/// 開閉は `effect.open`（`None` なら打鍵前の belief の開閉のまま）、入力モードのかな/英数は
/// `effect.mode`（`None` なら打鍵前の belief のまま）を予測値とし、開いているときだけモードを比べる。
pub fn prediction_mismatch(r: &PredictionRecord) -> Option<String> {
    let p = r.prediction?;
    let predicted_open = p.effect.open.unwrap_or(r.input_open);
    let predicted_native = mode_is_native(p.effect.mode.unwrap_or(r.input_mode));
    let truth = r.truth_after;
    let open_ok = predicted_open == truth.open;
    let mode_ok = !truth.open || predicted_native == truth.is_native();
    (!(open_ok && mode_ok)).then(|| {
        format!(
            "step#{} vk=0x{:02X}: 予測 open={} かな={} (effect={:?} track={:?}) ／ 真値 open={} conv=0x{:02X} {:?}",
            r.step,
            r.vk,
            predicted_open,
            predicted_native,
            p.effect,
            p.track,
            truth.open,
            truth.conv,
            truth.stage
        )
    })
}

/// **P2**: 予測器が「予測なし」を返したキーの**次の**キーで、古い記録段階が予測を狂わせない
/// （BUG-162 A-1）。
///
/// 次のキーで予測が返った場合、その予測（開閉・かな/英数）が擬似 IME の真の結果と一致すること。
/// 「予測なし」を返すのは許す（観測に任せるのは安全側）。
pub fn p2_no_stale_stage_after_unpredicted_key(h: &Harness) -> Result<(), String> {
    let bad: Vec<String> = h
        .predictions
        .windows(2)
        .filter(|pair| pair[0].prediction.is_none())
        .filter_map(|pair| {
            prediction_mismatch(&pair[1]).map(|m| {
                format!(
                    "  {m}（直前 step#{} vk=0x{:02X} は予測なし）",
                    pair[0].step, pair[0].vk
                )
            })
        })
        .collect();
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "P2 違反（BUG-162 A-1: 予測なしの後、記録された段階が古いまま次の予測を狂わせた）:\n{}",
            bad.join("\n")
        ))
    }
}

/// 補助（P2 の一般形）: 返った予測は、すべて擬似 IME の真の結果と一致する。
///
/// P2 は「予測なしの直後」だけを見るので、予測器が追跡だけの更新を返すようになると（BUG-162 A の
/// 1段目 `5476cdaa`）BUG-162 の列では空振りする。回帰の検出にはこちらも併用する。
pub fn predictions_agree_with_truth(h: &Harness) -> Result<(), String> {
    let bad: Vec<String> = h
        .predictions
        .iter()
        .filter_map(prediction_mismatch)
        .map(|m| format!("  {m}"))
        .collect();
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "予測が擬似 IME の真の結果と食い違う:\n{}",
            bad.join("\n")
        ))
    }
}

/// **P3**: 起動直後（awase が何も書いていない・明示意図なし）に、最初の成功観測が「閉」なら、
/// desired がその観測に揃い、drift 補正が発火しない（BUG-163、代案A）。
///
/// 検査の範囲は「起動から、最初の明示意図、または awase が IME の実状態を変えた書き込み
/// （warrant 付きで実状態と異なる値）まで」。実状態と同じ値の echo（Engine の活性遷移の
/// `SetOpen`）は「awase が書いた」に数えない（書いても IME は何も変わらない）。
pub fn p3_startup_aligns_desired_without_drift(h: &Harness) -> Result<(), String> {
    let horizon = h
        .steps
        .iter()
        .find(|s| s.explicit_intent)
        .map(|s| s.step)
        .into_iter()
        .chain(
            h.writes
                .iter()
                .filter(|w| w.warrant.is_some() && w.open != w.truth_open)
                .map(|w| w.step),
        )
        .min()
        .unwrap_or(usize::MAX);
    let mut bad = Vec::new();
    if let Some(first) = h
        .steps
        .iter()
        .find(|s| s.step < horizon && s.observed_open.is_some())
    {
        let observed = first.observed_open.expect("filter 済み");
        if !observed && first.desired_open {
            bad.push(format!(
                "  step#{} t={}ms: 最初の成功観測は閉なのに desired_open=true のまま（起動時の初期値）",
                first.step, first.at_ms
            ));
        }
    }
    // `detected == false`（ImmCross で warrant が下りず、検知の手前で見送った補正）は発火に数えない。
    for d in h
        .drift_fires
        .iter()
        .filter(|d| d.step < horizon && d.detected)
    {
        bad.push(format!(
            "  step#{} t={}ms: 明示意図なしで drift 補正が発火: {:?}",
            d.step, d.at_ms, d.drift
        ));
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "P3 違反（BUG-163: 起動直後の desired_open=true 初期値が最初の観測に揃わない／明示意図なしの補正が発火）:\n{}",
            bad.join("\n")
        ))
    }
}

/// 補助: 最後のステップで、belief（開閉・かな/英数）と Engine の活性が擬似 IME の真値に揃っている。
pub fn belief_matches_truth_at_end(h: &Harness) -> Result<(), String> {
    let last = h.steps.last().expect("start の記録がある");
    let truth_active = last.truth.open && last.truth.is_native();
    let mut bad = Vec::new();
    if last.effective_open != last.truth.open {
        bad.push(format!(
            "  開閉: belief={} 真値={}",
            last.effective_open, last.truth.open
        ));
    }
    if last.truth.open && mode_is_native(last.input_mode) != last.truth.is_native() {
        bad.push(format!(
            "  入力モード: belief={:?} 真値 conv=0x{:02X}",
            last.input_mode, last.truth.conv
        ));
    }
    if last.engine_active != truth_active {
        bad.push(format!(
            "  Engine: {} だが真値からは {}",
            last.engine_active, truth_active
        ));
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "最後の belief が真値と食い違う:\n{}",
            bad.join("\n")
        ))
    }
}

// ── 窓モデル（ADR-277 段階 2）────────────────────────────────────────────────

/// 十分に多い試行回数（どの `Blind { max_attempts }` よりも大きい）。
const MANY_ATTEMPTS: u32 = 1000;

/// 殻の `ir_observe_drift_facts` と同じ式の facts（`default_policy` は核、`imm_cross` は殻の live）に、方針
/// `active_policy`・試行 `MANY_ATTEMPTS` 回の進行中の試行を載せて、本物の `decide_drift_plan` が選ぶ段を返す。
/// ずれは「開きたいのに閉と観測、3 秒」に固定する（段の選び方は方針と試行回数だけで決まる）。
pub fn drift_step_with(
    active_policy: FeedbackPolicy,
    default_policy: FeedbackPolicy,
    imm_cross: bool,
) -> DriftStep {
    let now = Instant::now();
    let facts = DriftFacts {
        now,
        engine_enabled: true,
        japanese_ime: true,
        drift: Ok(DriftCorrection {
            desired: true,
            observed: false,
            duration_ms: 3000,
            source: ObservationSource::ObserverPoll,
            confidence: ObservationConfidence::Medium,
        }),
        settling: false,
        active: Some(ActuationSnapshot {
            target: true,
            policy: active_policy,
            attempts: MANY_ATTEMPTS,
            sent_at: now.checked_sub(Duration::from_secs(5)).unwrap_or(now),
            gave_up_at: None,
            origin: active_policy.origin(Generation::INITIAL),
        }),
        default_policy,
        imm_cross,
        warrant_would_block: false,
        diag_already_notified: false,
        converged: false,
        fresh_evidence_after_giveup: false,
    };
    match decide_drift_plan(&facts) {
        DriftPlan::Act(a) => a.step,
        other => panic!("ずれがあり settle 中でもないので Act のはず: {other:?}"),
    }
}

/// 方針の種類（golden・失敗メッセージ用）。
pub const fn feedback_kind(p: FeedbackPolicy) -> &'static str {
    match p {
        FeedbackPolicy::Read { .. } => "Read",
        FeedbackPolicy::Blind { .. } => "Blind",
    }
}

/// **窓モデル (a)**（DD3-01・#779）: 各ステップの後、核の `app_policy` が殻の live の profile から導いた値と一致する。
pub fn w_app_policy_follows_live_profile(h: &Harness) -> Result<(), String> {
    let bad: Vec<String> = h
        .steps
        .iter()
        .filter(|s| s.core_policy != AppImePolicy::from_profile(s.live_profile))
        .map(|s| {
            format!(
                "  step#{} {}: 核の app_policy={:?} だが殻の live の profile は {:?}",
                s.step, s.label, s.core_policy, s.live_profile
            )
        })
        .collect();
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "窓モデル (a) 違反（DD3-01: 核の app_policy が殻の live の profile に追随していない）:\n{}",
            bad.join("\n")
        ))
    }
}

/// **窓モデル (b)**: drift correction の再送の上限が窓の種類に応じて効く。各ステップで、核の方針で新しく据えた試行の
/// 段（本物の `decide_drift_plan`）が、殻の live の profile から導いた方針で据えた試行の段と一致する
/// （Blind の窓では上限で打ち切り、Read の窓では回数で打ち切らない）。
pub fn w_drift_cap_follows_live_window(h: &Harness) -> Result<(), String> {
    let bad: Vec<String> = h
        .steps
        .iter()
        .filter_map(|s| {
            let core = s.core_policy.default_feedback;
            let live = AppImePolicy::from_profile(s.live_profile).default_feedback;
            let got = drift_step_with(core, core, s.live_imm_cross);
            let want = drift_step_with(live, live, s.live_imm_cross);
            (got != want).then(|| {
                format!(
                    "  step#{} {}: 核の方針 {} で {:?}、窓（{:?}）の方針 {} なら {:?}",
                    s.step,
                    s.label,
                    feedback_kind(core),
                    got,
                    s.live_profile,
                    feedback_kind(live),
                    want
                )
            })
        })
        .collect();
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "窓モデル (b) 違反（drift の再送の上限が窓の種類に応じて効かない）:\n{}",
            bad.join("\n")
        ))
    }
}

/// **窓モデルの疑い（不変条件ではなく特性の記録）**: 殻は進行中の試行をプロセスが変わったときだけ捨てる
/// （`ir_notify_focus_changed` の `discard_actuation`、ADR-080 破棄条件2）。同じプロセスの中で窓の種類が変わる
/// （クラスの変化・学習による降格）前に据えた試行は、前の窓の方針のまま続く（`resolve_actuation` は同じ目標の試行の
/// 方針を見ない、ADR-080 不変条件4）。その段が、いまの窓の方針で据えた試行の段と食い違う最初の組を返す。
pub fn w_carried_actuation_matches_live_window(h: &Harness) -> Result<(), String> {
    for (j, sj) in h.steps.iter().enumerate() {
        let live = AppImePolicy::from_profile(sj.live_profile).default_feedback;
        let want = drift_step_with(live, live, sj.live_imm_cross);
        for si in h.steps[..=j]
            .iter()
            .filter(|si| si.focus_epoch == sj.focus_epoch)
        {
            let carried = si.core_policy.default_feedback;
            let got = drift_step_with(carried, sj.core_policy.default_feedback, sj.live_imm_cross);
            if got != want {
                return Err(format!(
                    "step#{}（{}）で据えた {} の試行が step#{}（{}、{:?}）まで残り {:?}。いまの窓の方針 {} なら {:?}",
                    si.step,
                    si.label,
                    feedback_kind(carried),
                    sj.step,
                    sj.label,
                    sj.live_profile,
                    got,
                    feedback_kind(live),
                    want
                ));
            }
        }
    }
    Ok(())
}
