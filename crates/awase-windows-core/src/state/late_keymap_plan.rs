//! `[[keymap]]` の遅いルール（`ime = "off"`、ADR-255）を発動してよいかの判断の核。
//!
//! 照合の位置は `kp_run_inner` の `engine.on_input` の直後で、エンジンが打鍵を素通しにしたときだけ。
//! ここは「ルールが一致した後、発動してよいか」を決める純粋関数で、事実はすべて殻が集めて渡す。
//! 発動しない理由は journal・debug ログに出すため enum で返す（ADR-255 決定2「素通しの理由は debug ログに出す」）。
//! 条件は ADR-255 決定2 の「発動条件（すべて AND）」に対応する（判定の順は理由の優先順で、決定2 の番号順とは違う。結果は AND なので変わらない）。**不確かなときは今までどおり素通し**。

use awase_gji_config::role::KeyDirectInputEffect;

/// 発動可否の判断に使う事実。殻（`kp_run_inner`）が集める。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LateKeymapFacts {
    /// 条件1: エンジンが `Inactive(ImeOff)`（`Engine::ime_off_inactive`）。
    pub engine_inactive_by_ime_off: bool,
    /// 条件2: エンジンの判断が素通し（`PassThrough`/`PassThroughWith`）。
    pub decision_passed_through: bool,
    /// 条件9: 自動リピートの Down（`was_down`）。
    pub was_down: bool,
    /// 条件5: 他ツールが注入したイベント。
    pub injected: bool,
    /// 条件5: Alt なりすまし由来（フックが Alt を親指キーへ書き換えた）。
    pub impersonated: bool,
    /// 条件6-i: `keys.ime_*` の bare 割り当て（`bare_ime_action`）がある。
    pub has_bare_ime_action: bool,
    /// 条件6-i: 役割由来の開閉（`thumb_role_open_actions()` の該当側。単独タップ設定が Passthrough でも数える）がある。
    pub has_thumb_role_open_action: bool,
    /// 条件6-i: 専用 Fn キー（`muhenkan_solo_tap_dedicated_fn_key`）がある。
    pub has_dedicated_fn_key: bool,
    /// 条件6-i: IME 側が定める開閉の方向（`ime_relevance.sync_direction`）がある。
    pub has_sync_direction: bool,
    /// 条件6-ii: 直接入力状態でこのキーに IME の機能があるか。TIP 未同定・MS-IME 本体・表が読めない・
    /// GJI 以外は殻が `Unknown` にして渡す。
    pub direct_input_effect: KeyDirectInputEffect,
    /// 条件7a: InputRelay（ローカルの belief がリモートの IME 状態を表さない）。
    pub input_relay: bool,
    /// 条件7b: ADR-245 の半角英数の「戻り待ち」が立っている。
    pub half_width_return_pending: bool,
    /// 条件8: 未確定文字列（composition）がある。
    pub composing: bool,
}

/// 発動しない理由。判定の順に並べる（先に当たったものを返す）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LateKeymapSkip {
    /// エンジンの非活性の理由が IME OFF でない（活性・UserDisabled・NotJapaneseIme・NotRomajiInput）。
    EngineNotImeOffInactive,
    /// エンジンが消費した（素通しでない）。
    EngineConsumed,
    /// 自動リピートの Down。
    Repeat,
    /// 注入されたイベント。
    Injected,
    /// Alt なりすまし由来。
    Impersonated,
    /// このキーは IME の機能（従来の4源のどれか）を持つ。
    KeyHasImeRole,
    /// 直接入力状態でこのキーに IME の機能がある、または判断できない。
    DirectInputHasFunctionOrUnknown,
    /// InputRelay。
    InputRelay,
    /// ADR-245 の戻り待ち。
    HalfWidthReturnPending,
    /// 未確定文字列がある。
    Composing,
}

/// 判断の結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LateKeymapPlan {
    /// 遅いルールを発動する（消費に格上げし、`to` を effects に積み、`record_shell_consumed` を呼ぶ）。
    Fire,
    /// 発動しない（今までどおり素通し）。
    Skip(LateKeymapSkip),
}

/// 遅いルールが一致したキーについて、発動してよいかを決める。
#[must_use]
pub const fn plan_late_keymap(facts: LateKeymapFacts) -> LateKeymapPlan {
    use LateKeymapPlan::{Fire, Skip};
    use LateKeymapSkip as S;
    if !facts.engine_inactive_by_ime_off {
        return Skip(S::EngineNotImeOffInactive);
    }
    if !facts.decision_passed_through {
        return Skip(S::EngineConsumed);
    }
    if facts.was_down {
        return Skip(S::Repeat);
    }
    if facts.injected {
        return Skip(S::Injected);
    }
    if facts.impersonated {
        return Skip(S::Impersonated);
    }
    if facts.has_bare_ime_action
        || facts.has_thumb_role_open_action
        || facts.has_dedicated_fn_key
        || facts.has_sync_direction
    {
        return Skip(S::KeyHasImeRole);
    }
    if !matches!(facts.direct_input_effect, KeyDirectInputEffect::NoFunction) {
        return Skip(S::DirectInputHasFunctionOrUnknown);
    }
    if facts.input_relay {
        return Skip(S::InputRelay);
    }
    if facts.half_width_return_pending {
        return Skip(S::HalfWidthReturnPending);
    }
    if facts.composing {
        return Skip(S::Composing);
    }
    Fire
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 発動する基準の事実（すべての条件を満たす）。
    const fn fire_facts() -> LateKeymapFacts {
        LateKeymapFacts {
            engine_inactive_by_ime_off: true,
            decision_passed_through: true,
            was_down: false,
            injected: false,
            impersonated: false,
            has_bare_ime_action: false,
            has_thumb_role_open_action: false,
            has_dedicated_fn_key: false,
            has_sync_direction: false,
            direct_input_effect: KeyDirectInputEffect::NoFunction,
            input_relay: false,
            half_width_return_pending: false,
            composing: false,
        }
    }

    #[test]
    fn fires_when_every_condition_holds() {
        assert_eq!(plan_late_keymap(fire_facts()), LateKeymapPlan::Fire);
    }

    /// 各条件を 1 つだけ崩すと、対応する理由で発動しない（全数）。
    #[test]
    fn each_single_violation_skips_with_its_reason() {
        use LateKeymapSkip as S;
        type Break = fn(&mut LateKeymapFacts);
        let cases: [(Break, LateKeymapSkip); 14] = [
            (
                |f| f.engine_inactive_by_ime_off = false,
                S::EngineNotImeOffInactive,
            ),
            (|f| f.decision_passed_through = false, S::EngineConsumed),
            (|f| f.was_down = true, S::Repeat),
            (|f| f.injected = true, S::Injected),
            (|f| f.impersonated = true, S::Impersonated),
            (|f| f.has_bare_ime_action = true, S::KeyHasImeRole),
            (|f| f.has_thumb_role_open_action = true, S::KeyHasImeRole),
            (|f| f.has_dedicated_fn_key = true, S::KeyHasImeRole),
            (|f| f.has_sync_direction = true, S::KeyHasImeRole),
            (
                |f| f.direct_input_effect = KeyDirectInputEffect::HasFunction,
                S::DirectInputHasFunctionOrUnknown,
            ),
            (
                |f| f.direct_input_effect = KeyDirectInputEffect::Unknown,
                S::DirectInputHasFunctionOrUnknown,
            ),
            (|f| f.input_relay = true, S::InputRelay),
            (
                |f| f.half_width_return_pending = true,
                S::HalfWidthReturnPending,
            ),
            (|f| f.composing = true, S::Composing),
        ];
        for (i, (breaker, reason)) in cases.into_iter().enumerate() {
            let mut facts = fire_facts();
            breaker(&mut facts);
            assert_eq!(
                plan_late_keymap(facts),
                LateKeymapPlan::Skip(reason),
                "case {i}"
            );
        }
    }

    /// 複数が崩れたら、判定の順で先のものが理由になる。
    #[test]
    fn earlier_condition_wins_the_reason() {
        let facts = LateKeymapFacts {
            engine_inactive_by_ime_off: false,
            composing: true,
            input_relay: true,
            ..fire_facts()
        };
        assert_eq!(
            plan_late_keymap(facts),
            LateKeymapPlan::Skip(LateKeymapSkip::EngineNotImeOffInactive)
        );
        let facts = LateKeymapFacts {
            composing: true,
            input_relay: true,
            ..fire_facts()
        };
        assert_eq!(
            plan_late_keymap(facts),
            LateKeymapPlan::Skip(LateKeymapSkip::InputRelay)
        );
    }

    /// 決定3: 観測の鮮度・品質を事実に持たない（IME の状態を読めないアプリでも発動できる）。
    #[test]
    fn facts_have_exactly_the_documented_fields() {
        // LateKeymapFacts のフィールドを網羅する分解。観測の鮮度・品質のフィールドを足すと、
        // このテストがコンパイルエラーになり、決定3（belief だけで判定）の見直しを促す。
        let LateKeymapFacts {
            engine_inactive_by_ime_off: _,
            decision_passed_through: _,
            was_down: _,
            injected: _,
            impersonated: _,
            has_bare_ime_action: _,
            has_thumb_role_open_action: _,
            has_dedicated_fn_key: _,
            has_sync_direction: _,
            direct_input_effect: _,
            input_relay: _,
            half_width_return_pending: _,
            composing: _,
        } = fire_facts();
    }
}
