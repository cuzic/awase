//! `[[keymap]]` の遅いルール（`ime = "off"`、ADR-255）を発動してよいかの判断の核。
//!
//! 照合の位置は `kp_run_inner` の `engine.on_input` の直後で、エンジンが打鍵を素通しにしたときだけ。
//! ここは「ルールが一致した後、発動してよいか」を決める純粋関数で、事実はすべて殻が集めて渡す。
//! 発動しない理由は journal・debug ログに出すため enum で返す（ADR-255 決定2「素通しの理由は debug ログに出す」）。
//! 条件は ADR-255 決定2 の「発動条件（すべて AND）」に対応する（判定の順は理由の優先順で、決定2 の番号順とは違う。結果は AND なので変わらない）。**不確かなときは今までどおり素通し**。

use awase::types::{ImeRelevance, KeyClassification, KeyEventType, RawKeyEvent, ScanCode, VkCode};
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
    /// 条件6-i: エンジンが知る IME の機能（`keys.ime_*`・専用 Fn キー・強制 open 軸操作・役割由来の開閉・
    /// `sync_direction`）をこのキーが持つ（`Engine::key_has_ime_function`。4源の集約はエンジンに閉じる）。
    pub has_ime_function: bool,
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

/// 発動したとき effects に積む、`to` の各キーの Down→Up の再注入イベント（`InputEffect::ReinjectKey` 用）。
///
/// **`SendKeys` ではなく `ReinjectKey` にする理由**（Opus #598 B1・M1）: (1) `KeyAction::Key(vk)` は Down 1 個だけで、
/// Up が足されない（エンジン自身は `output_history` で物理 Up のときに足す。遅いルールにはそれが無い）。
/// (2) 先行の素通し文字は `ReinjectKey`（`spawn_local` で後から SendInput）で運ばれるので、同期で実行される
/// `SendKeys` の Space はそれを追い越しうる。物理 Space の素通し（`ReinjectKey`）と同じ経路に乗せれば、
/// 先行の文字と FIFO で並び、`OUTPUT_GATE`・composition 確定の後処理（`on_reinject_key`）も物理 Space と同じになる。
/// 元の打鍵（`template`）から作るが、`vk`・`scan`・種別以外は素通しの通常キーとして初期化する
/// （`scan=0` は物理 Space の素通しと同じ。`reinject_scan_code` が IME モードキー以外は 0 にする）。
#[must_use]
pub fn late_keymap_reinject_events(
    template: &RawKeyEvent,
    send_vks: &[VkCode],
) -> Vec<RawKeyEvent> {
    send_vks
        .iter()
        .flat_map(|&vk| [(vk, KeyEventType::KeyDown), (vk, KeyEventType::KeyUp)])
        .map(|(vk, event_type)| RawKeyEvent {
            vk_code: vk,
            scan_code: ScanCode(0),
            event_type,
            key_classification: KeyClassification::Passthrough,
            physical_pos: None,
            ime_relevance: ImeRelevance::default(),
            modifier_key: None,
            injected: false,
            was_down: false,
            press_id: None,
            impersonated: false,
            ..*template
        })
        .collect()
}

/// 殻が集めた「IME の種別」と「GJI の表からの判定」から、`LateKeymapFacts::direct_input_effect` に渡す値を決める
/// （Opus #597 S3: TIP 未同定・MS-IME 本体・GJI 以外・表が読めない場合を `Unknown` に倒す規則を型の外の
/// 呼び出し側に任せない）。`gji_effect` は GJI の表が読めたときだけ `Some`。
#[must_use]
pub const fn resolve_direct_input_effect(
    tip: Option<crate::state::ime_kind::ImeKindId>,
    gji_effect: Option<KeyDirectInputEffect>,
) -> KeyDirectInputEffect {
    match (tip, gji_effect) {
        (Some(crate::state::ime_kind::ImeKindId::Gji), Some(effect)) => effect,
        _ => KeyDirectInputEffect::Unknown,
    }
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
    if facts.has_ime_function {
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
            has_ime_function: false,
            direct_input_effect: KeyDirectInputEffect::NoFunction,
            input_relay: false,
            half_width_return_pending: false,
            composing: false,
        }
    }

    #[test]
    fn reinject_events_are_down_up_pairs_in_order_and_plain_keys() {
        use awase::types::KeyEventType::{KeyDown, KeyUp};
        let template = RawKeyEvent {
            vk_code: VkCode(0x1D),
            scan_code: ScanCode(0x7B),
            event_type: KeyDown,
            extra_info: 0,
            timestamp: 7,
            key_classification: KeyClassification::LeftThumb,
            physical_pos: None,
            ime_relevance: ImeRelevance::default(),
            modifier_key: None,
            modifier_snapshot: awase::engine::ModifierState::default(),
            left_thumb_down_snapshot: None,
            right_thumb_down_snapshot: None,
            injected: false,
            was_down: false,
            press_id: None,
            foreign_ctrl: false,
            impersonated: false,
        };
        let space = VkCode(0x20);
        let f7 = VkCode(0x76);
        let events = late_keymap_reinject_events(&template, &[space, f7]);
        let shape: Vec<_> = events.iter().map(|e| (e.vk_code, e.event_type)).collect();
        assert_eq!(
            shape,
            [(space, KeyDown), (space, KeyUp), (f7, KeyDown), (f7, KeyUp)],
            "各キーを Down→Up の対にして並べる(Up が欠けると OS で押されたままになる)"
        );
        for e in &events {
            assert_eq!(e.scan_code, ScanCode(0));
            assert!(matches!(
                e.key_classification,
                KeyClassification::Passthrough
            ));
            assert!(!e.injected && !e.was_down && !e.impersonated);
        }
        assert!(late_keymap_reinject_events(&template, &[]).is_empty());
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
        let cases: [(Break, LateKeymapSkip); 11] = [
            (
                |f| f.engine_inactive_by_ime_off = false,
                S::EngineNotImeOffInactive,
            ),
            (|f| f.decision_passed_through = false, S::EngineConsumed),
            (|f| f.was_down = true, S::Repeat),
            (|f| f.injected = true, S::Injected),
            (|f| f.impersonated = true, S::Impersonated),
            (|f| f.has_ime_function = true, S::KeyHasImeRole),
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
    fn direct_input_effect_is_unknown_unless_tip_is_gji_and_table_was_read() {
        use crate::state::ime_kind::ImeKindId;
        use KeyDirectInputEffect::{HasFunction, NoFunction, Unknown};
        for effect in [NoFunction, HasFunction, Unknown] {
            assert_eq!(
                resolve_direct_input_effect(Some(ImeKindId::Gji), Some(effect)),
                effect
            );
            // TIP 未同定・MS-IME 本体は GJI の設定ファイルが残っていても Unknown。
            assert_eq!(resolve_direct_input_effect(None, Some(effect)), Unknown);
            assert_eq!(
                resolve_direct_input_effect(Some(ImeKindId::MsIme), Some(effect)),
                Unknown
            );
        }
        // 表が読めない。
        assert_eq!(
            resolve_direct_input_effect(Some(ImeKindId::Gji), None),
            Unknown
        );
    }

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
            has_ime_function: _,
            direct_input_effect: _,
            input_relay: _,
            half_width_return_pending: _,
            composing: _,
        } = fire_facts();
    }
}
