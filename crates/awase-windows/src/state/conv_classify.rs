//! idle-conv-check の conv ビット解釈を集約する純粋関数。
//!
//! `kp_stage_idle_conv_check` は IMM32 の変換モード (NATIVE/KATAKANA/ROMAN/FULLSHAPE)
//! を読み、belief の `input_mode` 更新と NICOLA engine の ON/OFF 同期を決めていた。
//! 従来この判断は手続きの中にインライン展開され、`handle_engine_set_open` が 5 箇所に
//! 散っていたため、ビット組合せの見落としバグ（ROMAN 見落とし `fc18cc7`、KATAKANA 喪失
//! `109b4c9`、HanKata→ZenKata 誤ダウングレード `1544d3f`、HanAlpha→Hiragana で Engine
//! OFF のまま `ea3da7f`）が繰り返し発生していた。
//!
//! この関数は分岐を 1 箇所に集約し、Win32 API・時刻取得・`with_app` 呼び出しを一切
//! 行わない純粋関数として全数テスト可能にする。時間依存のガード
//! (`should_run_idle_conv_check`) は呼び出し元で評価済みとする。

use awase::engine::{ConvMode, InputModeState};

/// engine ON 同期の理由。ログとテストの両方で「どの規則が発火したか」を固定する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ConvSyncReason {
    /// カタカナ (NATIVE+KATAKANA) を観測し shadow=OFF → engine ON 同期。
    KatakanaShadowOff,
    /// belief が romaji 不可→可 に回復 かつ shadow=ON → engine 再起動。
    RomajiRecovered,
    /// ひらがな/カタカナ (NATIVE) への切替を観測し shadow=OFF → engine ON 同期。
    NativeToggleShadowOff,
}

/// engine 同期アクション。従来 5 箇所に散っていた `handle_engine_set_open` 呼び出しを
/// 統一表現し、呼び出し元が 1 経路で dispatch できるようにする。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EngineSync {
    /// engine への働きかけなし。
    None,
    /// engine を ON にする (`handle_engine_set_open(true)`)。
    SetOpen(ConvSyncReason),
    /// `ObservedEisu` 観測 → engine OFF + DirectInput。conv の英数モードは IME-ON の
    /// 確証（conv=0x10 は ROMAN ビット付き半角英数）のため、`effective_open=true` の
    /// belief を直接注入して apply する。
    DirectInput,
}

/// idle-conv-check の判断結果。input_mode belief の更新と engine 同期を分離して表す。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConvTransition {
    /// belief.input_mode の更新 (`None` = 変更なし / ダウングレード抑制)。
    /// `Some` の場合、呼び出し元が `InputModeObserved` を dispatch する。
    pub input_mode_update: Option<InputModeState>,
    /// engine ON/OFF 同期アクション。
    pub engine: EngineSync,
}

/// conv ポーリング値・現在の belief・engine 状態から idle-conv-check の同期判断を導く。
///
/// I/O・時刻取得・`with_app` 呼び出しを一切行わない純粋関数。
///
/// # 引数
/// - `conv`: `ImmGetConversionStatus` の raw 値。
/// - `current`: 現在の `input_mode` belief。
/// - `is_cold`: `output_in_flight_ms() == u64::MAX`（ROMAN ビット未確定期間）。
/// - `effective_open`: 現在の engine open 状態 (`effective_open()`)。
/// - `conv_mode_changed`: `ConvModeMgr::update_from_conv` が変化を検出したか。
/// - `is_roman_reliable`: ROMAN ビット (0x10) が信頼できるか。TsfNative の idle 経路では
///   常に `false`。
pub fn classify_conv_transition(
    conv: u32,
    current: InputModeState,
    is_cold: bool,
    effective_open: bool,
    conv_mode_changed: bool,
    is_roman_reliable: bool,
) -> ConvTransition {
    let cm = ConvMode::from_u32(conv);
    let input_mode_update = cm.classify_idle(is_cold, current, is_roman_reliable);
    // NATIVE=0 ⟺ 英数モード (is_eisu)。KATAKANA は Charset で判定する。
    let has_native = !cm.is_eisu();
    let has_katakana = cm.charset.is_katakana();
    let was_romaji_capable = current.is_romaji_capable();

    let engine = match input_mode_update {
        None => {
            // belief 変化なし。
            // - conv 不変: カタカナ(NATIVE+KATAKANA)+shadow=OFF のみが唯一の回復経路
            //   (AssumedRomaji は常に classify_idle=None を返すため)。
            // - conv 変化: NATIVE(ひらがな/カタカナ)切替+shadow=OFF を engine ON 同期。
            if !conv_mode_changed {
                if has_katakana && has_native && !effective_open {
                    EngineSync::SetOpen(ConvSyncReason::KatakanaShadowOff)
                } else {
                    EngineSync::None
                }
            } else if has_native && !effective_open {
                EngineSync::SetOpen(ConvSyncReason::NativeToggleShadowOff)
            } else {
                EngineSync::None
            }
        }
        Some(new_mode) => {
            // belief を更新した上で engine を同期する。従来コードは複数の if を順に
            // 評価していたが、発火するアクションは互いに排他（対象 open が衝突しない）
            // なので単一アクションに集約できる。ObservedEisu (NATIVE=0) は NativeToggle
            // 系と、`!effective_open` を要求する分岐は `effective_open` を要求する
            // romaji 回復分岐と排他になる。
            if matches!(new_mode, InputModeState::ObservedEisu) {
                EngineSync::DirectInput
            } else if matches!(new_mode, InputModeState::ObservedRomaji)
                && has_katakana
                && !effective_open
            {
                EngineSync::SetOpen(ConvSyncReason::KatakanaShadowOff)
            } else if !was_romaji_capable && new_mode.is_romaji_capable() && effective_open {
                EngineSync::SetOpen(ConvSyncReason::RomajiRecovered)
            } else if conv_mode_changed && has_native && !effective_open {
                EngineSync::SetOpen(ConvSyncReason::NativeToggleShadowOff)
            } else {
                EngineSync::None
            }
        }
    };

    ConvTransition {
        input_mode_update,
        engine,
    }
}

// ── ジャーナル・リプレイ回帰基盤（P1）───────────────────────────────────────────

/// 実機ジャーナル由来（または手作り）の `classify_conv_transition` 呼び出し1件を
/// 表す固定フィクスチャ。`tests/journals/*.json` に配列として保存し、
/// `tests/journal_replay.rs` が読み込んで再実行・照合する。
///
/// `journal.rs::JournalEntry::ConvClassifyCall` が実機ダンプで記録する
/// フィールドと同じ形（conv/current/is_cold/effective_open/conv_mode_changed/
/// is_roman_reliable → result）だが、こちらは往復可能な独立フォーマットとして
/// 定義する（`JournalEntry` 全体は `KeyEventSummary` に `&'static str` を含み
/// 単純には `Deserialize` できないため、リプレイ専用に切り出している）。
///
/// フィクスチャの追加手順は `docs/journal-replay-guide.md` を参照。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConvClassifyFixture {
    /// 何が起きたバグ/シナリオの記録か（人間可読な短い説明）。
    pub name: String,
    /// 参考: 実機で発生した既知のバグの説明・関連コミット等（任意）。
    #[serde(default)]
    pub note: String,
    pub conv: u32,
    pub current: InputModeState,
    pub is_cold: bool,
    pub effective_open: bool,
    pub conv_mode_changed: bool,
    pub is_roman_reliable: bool,
    /// 期待される `ConvTransition`。実機ダンプをそのまま転記した直後はバグを
    /// 含む「実際の」出力になっていることがあるため、修正後は必ず「あるべき」
    /// 出力に手で書き換えてからコミットすること。
    pub expected: ConvTransition,
}

#[cfg(test)]
mod tests {
    use super::*;
    use awase::engine::{AssumedReason, InputModeState};

    // ── conv ビット定数（IMM32 変換モード）─────────────────────────────────────
    const NATIVE: u32 = 0x0001;
    const KATAKANA: u32 = 0x0002;
    const FULLSHAPE: u32 = 0x0008;
    const ROMAN: u32 = 0x0010;

    // 代表的な conv 値
    const CONV_HANALPHA: u32 = 0x0000; // 半角英数 (全ビット 0)
    const CONV_EISU_ROMAN: u32 = ROMAN; // 0x0010: MS-IME 半角英数 (ROMAN 付き) fc18cc7
    const CONV_ZENALPHA: u32 = FULLSHAPE; // 0x0008: 全角英数
    const CONV_HIRAGANA: u32 = NATIVE | FULLSHAPE | ROMAN; // 0x0019: ひらがなローマ字
    const CONV_JISKANA: u32 = NATIVE | FULLSHAPE; // 0x0009: JISかな (ROMAN なし)
    const CONV_ZENKATA: u32 = NATIVE | KATAKANA | FULLSHAPE; // 0x000B: 全角カタカナ
    const CONV_HANKATA: u32 = NATIVE | KATAKANA; // 0x0003: 半角カタカナ 1544d3f

    fn assumed() -> InputModeState {
        InputModeState::AssumedRomaji {
            reason: AssumedReason::ImmBridgeBroken,
        }
    }

    /// idle 経路のデフォルト引数で分類する（is_cold=false, is_roman_reliable=false）。
    fn classify(
        conv: u32,
        current: InputModeState,
        effective_open: bool,
        conv_mode_changed: bool,
    ) -> ConvTransition {
        classify_conv_transition(conv, current, false, effective_open, conv_mode_changed, false)
    }

    // ── 英数モード検出（ObservedEisu → DirectInput）──────────────────────────────

    #[test]
    fn hanalpha_detected_as_eisu_direct_input() {
        let t = classify(CONV_HANALPHA, assumed(), true, true);
        assert_eq!(t.input_mode_update, Some(InputModeState::ObservedEisu));
        assert_eq!(t.engine, EngineSync::DirectInput);
    }

    /// fc18cc7 回帰: ROMAN ビット付き半角英数 (conv=0x0010) も英数モードとして扱う。
    #[test]
    fn eisu_with_roman_bit_0x10_is_still_eisu() {
        let t = classify(CONV_EISU_ROMAN, assumed(), true, true);
        assert_eq!(t.input_mode_update, Some(InputModeState::ObservedEisu));
        assert_eq!(t.engine, EngineSync::DirectInput);
    }

    #[test]
    fn zenalpha_detected_as_eisu_direct_input() {
        let t = classify(CONV_ZENALPHA, assumed(), false, true);
        assert_eq!(t.input_mode_update, Some(InputModeState::ObservedEisu));
        // ObservedEisu は NATIVE=0 なので NativeToggle 系とは排他 → DirectInput のみ。
        assert_eq!(t.engine, EngineSync::DirectInput);
    }

    #[test]
    fn eisu_when_belief_already_eisu_no_input_mode_update_but_still_direct_input() {
        // classify_idle は既に ObservedEisu の場合 None を返すが、それは belief 変化なし
        // であって engine 同期の必要性とは別。conv 不変なら engine も触らない。
        let t = classify(CONV_HANALPHA, InputModeState::ObservedEisu, false, false);
        assert_eq!(t.input_mode_update, None);
        assert_eq!(t.engine, EngineSync::None);
    }

    // ── カタカナ（NATIVE+KATAKANA）──────────────────────────────────────────────

    /// 1544d3f 回帰: 半角カタカナ (HanKata, conv=0x0003) を認識する。
    /// TsfNative では ROMAN=0 だが KATAKANA は romaji-capable 扱い → ObservedRomaji。
    #[test]
    fn hankata_from_non_romaji_recovers_to_observed_romaji() {
        let t = classify(CONV_HANKATA, InputModeState::ObservedKana, false, true);
        assert_eq!(t.input_mode_update, Some(InputModeState::ObservedRomaji));
        // shadow=OFF + カタカナ検出 → engine ON 同期
        assert_eq!(
            t.engine,
            EngineSync::SetOpen(ConvSyncReason::KatakanaShadowOff)
        );
    }

    #[test]
    fn zenkata_shadow_off_engine_on() {
        let t = classify(CONV_ZENKATA, InputModeState::ObservedKana, false, true);
        assert_eq!(t.input_mode_update, Some(InputModeState::ObservedRomaji));
        assert_eq!(
            t.engine,
            EngineSync::SetOpen(ConvSyncReason::KatakanaShadowOff)
        );
    }

    #[test]
    fn zenkata_shadow_on_no_engine_change() {
        // 既に romaji_capable なら input_mode 変化なし、effective_open=true なら engine も不変。
        let t = classify(CONV_ZENKATA, assumed(), true, true);
        assert_eq!(t.input_mode_update, None);
        assert_eq!(t.engine, EngineSync::None);
    }

    /// 0f75b5b 回帰: カタカナ + shadow=OFF + conv 不変でも engine を復帰させる唯一の経路。
    #[test]
    fn katakana_shadow_off_conv_unchanged_still_recovers_engine() {
        let t = classify(CONV_ZENKATA, assumed(), false, false);
        assert_eq!(t.input_mode_update, None);
        assert_eq!(
            t.engine,
            EngineSync::SetOpen(ConvSyncReason::KatakanaShadowOff)
        );
    }

    // ── ひらがな（NATIVE, ROMAN 有無）──────────────────────────────────────────

    /// ひらがなローマ字 (ROMAN 付き, 0x19) は belief が非 romaji_capable のとき
    /// ObservedRomaji に訂正され、shadow=ON なら engine 再起動 (RomajiRecovered)。
    #[test]
    fn hiragana_roman_recovers_romaji_and_restarts_engine_when_shadow_on() {
        let t = classify(CONV_HIRAGANA, InputModeState::ObservedKana, true, true);
        assert_eq!(t.input_mode_update, Some(InputModeState::ObservedRomaji));
        assert_eq!(t.engine, EngineSync::SetOpen(ConvSyncReason::RomajiRecovered));
    }

    /// 同上だが shadow=OFF: RomajiRecovered は effective_open を要求するため発火せず、
    /// 代わりに NativeToggle (NATIVE+shadow=OFF) で engine ON 同期する。
    #[test]
    fn hiragana_roman_recovers_romaji_and_syncs_engine_on_when_shadow_off() {
        let t = classify(CONV_HIRAGANA, InputModeState::ObservedKana, false, true);
        assert_eq!(t.input_mode_update, Some(InputModeState::ObservedRomaji));
        assert_eq!(
            t.engine,
            EngineSync::SetOpen(ConvSyncReason::NativeToggleShadowOff)
        );
    }

    /// ea3da7f 回帰: HanAlpha→Hiragana(ROMAN なし, JISかな conv) で belief が非
    /// romaji_capable のとき、TsfNative (is_roman_reliable=false) では ObservedKana への
    /// downgrade をせず AssumedRomaji { ImmBridgeBroken } に回復する。
    /// shadow=ON なら engine 再起動 (RomajiRecovered)。
    #[test]
    fn jiskana_recovers_to_assumed_romaji_and_restarts_engine_when_shadow_on() {
        let t = classify(CONV_JISKANA, InputModeState::ObservedKana, true, true);
        assert_eq!(
            t.input_mode_update,
            Some(InputModeState::AssumedRomaji {
                reason: AssumedReason::ImmBridgeBroken
            })
        );
        assert_eq!(t.engine, EngineSync::SetOpen(ConvSyncReason::RomajiRecovered));
    }

    #[test]
    fn hiragana_belief_already_romaji_capable_no_change() {
        // AssumedRomaji は romaji_capable → classify_idle=None。
        // effective_open=true なら engine も不変。
        let t = classify(CONV_HIRAGANA, assumed(), true, true);
        assert_eq!(t.input_mode_update, None);
        assert_eq!(t.engine, EngineSync::None);
    }

    #[test]
    fn hiragana_belief_romaji_capable_shadow_off_syncs_engine() {
        // input_mode 変化なし (None) だが conv 変化 + NATIVE + shadow=OFF → engine ON。
        let t = classify(CONV_HIRAGANA, assumed(), false, true);
        assert_eq!(t.input_mode_update, None);
        assert_eq!(
            t.engine,
            EngineSync::SetOpen(ConvSyncReason::NativeToggleShadowOff)
        );
    }

    /// d41ba86 回帰: JISかな (ROMAN なし) でも NATIVE 切替として engine ON 同期する
    /// (is_roman_reliable=false のためひらがなへの downgrade はしない)。
    #[test]
    fn jiskana_native_toggle_shadow_off_syncs_engine() {
        let t = classify(CONV_JISKANA, assumed(), false, true);
        assert_eq!(t.input_mode_update, None);
        assert_eq!(
            t.engine,
            EngineSync::SetOpen(ConvSyncReason::NativeToggleShadowOff)
        );
    }

    // ── ダウングレード抑制 (ed862bb) ───────────────────────────────────────────

    /// ed862bb: TsfNative (is_roman_reliable=false) では AssumedRomaji → ObservedKana の
    /// downgrade を抑制する。classify_idle は None を返すため belief は維持される。
    #[test]
    fn tsf_native_suppresses_romaji_to_kana_downgrade() {
        let t = classify_conv_transition(CONV_JISKANA, assumed(), false, true, false, false);
        assert_eq!(t.input_mode_update, None);
    }

    /// 逆に is_roman_reliable=true（通常 IMM32）ではひらがな conv で ObservedKana に訂正する。
    #[test]
    fn roman_reliable_downgrades_to_kana() {
        let t = classify_conv_transition(CONV_JISKANA, assumed(), false, false, true, true);
        assert_eq!(t.input_mode_update, Some(InputModeState::ObservedKana));
    }

    // ── cold start ─────────────────────────────────────────────────────────────

    /// cold start 中 (is_cold=true) は ROMAN ビットが信頼できないため、ひらがなローマ字
    /// conv でも belief を変更しない（英数モードのみ確実に判定）。
    #[test]
    fn cold_start_hiragana_roman_no_input_mode_change() {
        let t = classify_conv_transition(CONV_HIRAGANA, InputModeState::Unknown, true, false, false, false);
        assert_eq!(t.input_mode_update, None);
    }

    #[test]
    fn cold_start_eisu_still_detected() {
        let t = classify_conv_transition(CONV_HANALPHA, InputModeState::Unknown, true, false, true, false);
        assert_eq!(t.input_mode_update, Some(InputModeState::ObservedEisu));
        assert_eq!(t.engine, EngineSync::DirectInput);
    }

    // ── engine None（何も同期しない）ケース ─────────────────────────────────────

    #[test]
    fn no_conv_change_no_belief_change_is_noop() {
        let t = classify(CONV_HIRAGANA, assumed(), true, false);
        assert_eq!(t.input_mode_update, None);
        assert_eq!(t.engine, EngineSync::None);
    }

    #[test]
    fn native_toggle_but_already_open_no_engine_change() {
        // conv 変化 + NATIVE だが effective_open=true → NativeToggle は !effective_open を要求 → None。
        let t = classify(CONV_JISKANA, assumed(), true, true);
        assert_eq!(t.input_mode_update, None);
        assert_eq!(t.engine, EngineSync::None);
    }

    // ── 全数に近い網羅: 代表 conv × belief × (open, changed) の組合せ ───────────

    /// 主要 conv 値 × 代表 belief で panic せず一貫した結果を返すことを確認する
    /// スモークテスト（enum 化により全組合せが型上網羅されていることの担保）。
    #[test]
    fn smoke_all_major_conv_belief_combinations() {
        let convs = [
            CONV_HANALPHA,
            CONV_EISU_ROMAN,
            CONV_ZENALPHA,
            CONV_HIRAGANA,
            CONV_JISKANA,
            CONV_ZENKATA,
            CONV_HANKATA,
        ];
        let beliefs = [
            InputModeState::ObservedRomaji,
            InputModeState::ObservedKana,
            InputModeState::ObservedEisu,
            assumed(),
            InputModeState::Unknown,
        ];
        for &conv in &convs {
            for &belief in &beliefs {
                for &open in &[false, true] {
                    for &changed in &[false, true] {
                        let t = classify(conv, belief, open, changed);
                        // 英数モードは常に ObservedEisu → DirectInput（belief が既に Eisu の
                        // 場合を除く）を返すという不変条件。
                        if ConvMode::from_u32(conv).is_eisu() {
                            match t.input_mode_update {
                                Some(m) => {
                                    assert_eq!(m, InputModeState::ObservedEisu);
                                    assert_eq!(t.engine, EngineSync::DirectInput);
                                }
                                None => {
                                    // belief が既に ObservedEisu のケースのみ。
                                    assert_eq!(belief, InputModeState::ObservedEisu);
                                }
                            }
                        }
                        // SetOpen は必ず !effective_open か RomajiRecovered(effective_open) の
                        // いずれかの整合した条件でのみ発火する。
                        if let EngineSync::SetOpen(reason) = t.engine {
                            match reason {
                                ConvSyncReason::RomajiRecovered => assert!(open),
                                _ => assert!(!open),
                            }
                        }
                    }
                }
            }
        }
    }
}
