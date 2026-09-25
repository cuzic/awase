//! ADR-191（IME が状態の正、awase は書かずに観測・予測に追随する）の不変条件の検査。
//!
//! 各検査は違反を全部集めて `Err(説明)` を返す（最初の1件で止めない）。シナリオ側は
//! `assert_ok(h, check(h))` で経過（`Harness::trace`）と一緒に表示する。

use awase::engine::InputModeState;

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
/// 活性化したときの `SetOpen(true, ActivationSync)`（IME は既に開いていて、書いても実状態は
/// 変わらない。本番は `dispatch_ime_set_open` を通る）まで違反になるため、
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
fn prediction_mismatch(r: &PredictionRecord) -> Option<String> {
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
