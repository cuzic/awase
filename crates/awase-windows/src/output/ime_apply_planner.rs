//! IME open 状態の観測値を「適用時ビリーフ」へ純粋関数で還元する data-model 層。
//!
//! IME 適用機構の選択と実行は [`crate::ime_controller`] の Strategy 群が唯一の
//! SSOT として担う。このモジュールは観測値（conv_mode / candidate / shadow 等）を
//! `effective_open` / `confident` に副作用なしで還元する `reduce_open_belief` のみを
//! 提供する。結果は `platform.rs` の apply 経路で診断ログに使われる。

// ── Observation → Belief reduction ───────────────────────────────

/// [`reduce_open_belief`] へ渡す観測値の集約。
///
/// 呼び出し元が収集できる全観測値をここにまとめる。
/// planner / strategy は自らこれらの値を読まない（テスト可能性のため）。
#[derive(Debug, Clone, Copy)]
pub(crate) struct OpenBeliefInputs {
    // ── 指令状態 ──
    pub shadow_on: bool,
    pub applied: crate::state::AppliedImeState,
    // ── OS イベント観測 ──
    pub candidate_visible: bool,
    pub candidate_was_seen: bool,
    pub gji_monitor_ok: bool,
    // ── OS 直接読み取り（任意） ──
    pub conv_mode: Option<u32>,
    // ── コンテキスト ──
    pub can_imm32_cross_process: bool,
    pub now_ms: u64,
}

/// 複数の観測値を 1 つの「適用時 IME 状態ビリーフ」に純粋関数で還元する。
#[derive(Debug, Clone, Copy)]
pub(crate) struct OpenBelief {
    /// 現在の IME open 状態の推定値。
    pub effective_open: bool,
    /// 推定に十分な確信があるか。`false` の場合は already_matched を強制 false にする。
    pub confident: bool,
}

impl OpenBelief {
    /// shadow のみから自明なビリーフを作る（後方互換ラッパー用）。
    pub(crate) fn from_shadow(shadow_on: bool) -> Self {
        Self {
            effective_open: shadow_on,
            confident: true,
        }
    }
}

/// 観測値を純粋に還元して `OpenBelief` を返す。
///
/// # effective_open の計算
/// `conv_mode` が取得できた場合はそれを ground-truth として使用する（conv=0 → DirectInput=false）。
/// 取得できない場合は shadow_on + candidate 観測で推定する。
///
/// # confident の計算
/// ImmCross/GJI で確認できない環境（KanjiToggle 系）でのみ `safely_confirmed` を
/// 検査する。それ以外は常に `true`。
/// （旧 `is_engine_intent` 条件は 2026-07-06 到達不能パス監査 B6 で撤去 —
/// SetOpen は常に Engine の意図であり恒真だった。）
/// `confident=false` は「already_matched を強制 false」つまり「必ず apply する」を意味する。
pub(crate) fn reduce_open_belief(inputs: &OpenBeliefInputs, desired_open: bool) -> OpenBelief {
    let effective_open = inputs.conv_mode.map_or(
        inputs.shadow_on
            || inputs.candidate_visible
            || (!desired_open && inputs.candidate_was_seen),
        |conv| {
            if desired_open {
                // open=true 要求時: IME_CMODE_NATIVE(0x1) ビットでひらがな/カタカナを判定。
                // conv=0 (DirectInput) や conv=0x10 (ROMAN のみ) は半角英数直接入力 = IME OFF 相当扱い。
                // VK_DBE_HIRAGANA を送ってひらがなモードに復帰させる必要がある。
                conv & 0x0001 != 0
            } else {
                // open=false 要求時: DirectInput(0) でなければ「IME ON」扱い（従来通り）。
                conv != 0
            }
        },
    );

    let confident = if !inputs.can_imm32_cross_process
        && !inputs.gji_monitor_ok
        && inputs.conv_mode.is_none()
    {
        // KanjiToggle 系（Chrome/TsfNative 等）: Confirmed かつ shadow 一致 かつ 300ms 以内のみ確信あり
        inputs.shadow_on == desired_open
            && inputs.applied.is_confirmed()
            && inputs
                .now_ms
                .saturating_sub(inputs.applied.confirmed_at_ms())
                < 300
    } else {
        true
    };

    OpenBelief {
        effective_open,
        confident,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduce_open_belief_roman_only_conv_is_not_hiragana() {
        // conv=0x10 (IME_CMODE_ROMAN のみ) = GJI 半角英数。open=true 時は effective_open=false。
        let inputs = OpenBeliefInputs {
            shadow_on: true,
            applied: crate::state::AppliedImeState::Unknown,
            candidate_visible: false,
            candidate_was_seen: false,
            gji_monitor_ok: false,
            conv_mode: Some(0x10),
            can_imm32_cross_process: false,
            now_ms: 0,
        };
        let belief = reduce_open_belief(&inputs, true);
        assert!(
            !belief.effective_open,
            "conv=16 (ROMAN only) は open=true 要求時に false"
        );
        // open=false 要求時は従来通り true（DirectInput でないため）
        let belief_off = reduce_open_belief(&inputs, false);
        assert!(
            belief_off.effective_open,
            "conv=16 は open=false 要求時に true（IME ON 状態）"
        );
    }

    #[test]
    fn reduce_open_belief_hiragana_conv_is_matched() {
        // conv=0x09 (NATIVE|ROMAN) = ひらがなローマ字。open=true 時は effective_open=true。
        let inputs = OpenBeliefInputs {
            shadow_on: true,
            applied: crate::state::AppliedImeState::Unknown,
            candidate_visible: false,
            candidate_was_seen: false,
            gji_monitor_ok: false,
            conv_mode: Some(0x09),
            can_imm32_cross_process: false,
            now_ms: 0,
        };
        let belief = reduce_open_belief(&inputs, true);
        assert!(
            belief.effective_open,
            "conv=9 (NATIVE|ROMAN) は open=true 要求時に true"
        );
    }
}
