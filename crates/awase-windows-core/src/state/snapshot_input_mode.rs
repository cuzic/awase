//! IME の読み(`ImeSnapshot`)から、`input_mode` の belief をどう更新するかを決める純粋関数(ADR-239)。
//!
//! `observer/ime_observer.rs::classify_ime_snapshot` の `new_input_mode` を決める枝を、`#[cfg(windows)]` の `observer/` から
//! cfg の無い `state/` へ移したもの(Linux のテストで固定するため)。順序は元の else-if の鎖そのまま:
//! 1. force-on guard 中で `is_romaji` が不明 → 何も採らない(**(2) より先**。英数の conv は必ず `is_romaji=None` なので、
//!    guard 中は `ime_on=Some(true)`・conv=0 でも ObservedEisu を出さない)。
//! 2. 英数の証拠(`ConvMode::is_eisu_evidence`)→ ObservedEisu。**ここで終わる**(現在が既に ObservedEisu なら `None`。
//!    romaji フラグや stale 回復へは落ちない)。
//! 3. `is_romaji` が分かる → ObservedRomaji/ObservedKana(romaji フラグ)。
//! 4. `is_romaji` が不明で現在が ObservedEisu、かつ conv が英数でない → AssumedRomaji(ObservedEisu の stale 回復。
//!    GJI 等 ROMAN ビットを使わない IME で、英数→ひらがなに戻っても belief が ObservedEisu に固まるのを防ぐ)。
//!
//! 以前はここに「前回の conv との差分」(`ConvMode::classify_transition`)の枝があったが、`prev_conversion_mode` が
//! 読み取りのたびにリセットされて refresh の経路で一度も結果を返しておらず(約 6 か月)、返せる結果は他の場所で採らないと
//! 決めた形(閉じた IME の conv=0 を英数とみなす=BUG-57、ROMAN ビットなしを ObservedKana とみなす)だけなので撤去した(ADR-239)。
//!
//! `trust_input_mode`(awase 自身の UI の読みを採らない)は呼び出し元が見る(false なら、この関数を呼ばずに `None`)。

use awase::engine::{AssumedReason, ConvMode, InputModeState};

/// [`decide_input_mode`] の入力(読みと現在の状態)。
#[derive(Debug, Clone, Copy)]
pub struct SnapshotModeInput {
    /// この読みの `ime_on`(`None` = open プローブが失敗/時間切れ、または TsfNative)。
    pub ime_on: Option<bool>,
    /// この読みの ROMAN ビット由来のローマ字入力か(`None` = 不明。英数の conv は NATIVE=0 なので必ず `None`)。
    pub is_romaji: Option<bool>,
    /// この読みの conv(`None` = 取れなかった/TsfNative)。
    pub conv: Option<u32>,
    /// 現在の belief の input_mode。
    pub current: InputModeState,
    /// force-on guard が有効か。
    pub guard_active: bool,
}

/// どの枝が結果を決めたか(呼び出し元のログ用)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeSource {
    /// 何も採らなかった(guard 中、または romaji フラグ・英数の証拠・stale 回復のどれも当たらない)。
    None,
    /// 英数の証拠(`is_eisu_evidence`)。結果が `None` のこと(現在が既に ObservedEisu)もある。
    Eisu,
    /// romaji フラグ(`is_romaji` が分かった)。
    RomajiFlag,
    /// ObservedEisu の stale 回復。
    StaleRecovery,
}

/// `input_mode` の belief に採る値を決める。戻り値は (採る値, どの枝か)。
#[must_use]
pub fn decide_input_mode(i: &SnapshotModeInput) -> (Option<InputModeState>, ModeSource) {
    if i.guard_active && i.is_romaji.is_none() {
        return (None, ModeSource::None);
    }
    if ConvMode::is_eisu_evidence(i.ime_on, i.conv) == Some(true) {
        let new = (!matches!(i.current, InputModeState::ObservedEisu))
            .then_some(InputModeState::ObservedEisu);
        return (new, ModeSource::Eisu);
    }
    if let Some(romaji) = i.is_romaji {
        let mode = if romaji {
            InputModeState::ObservedRomaji
        } else {
            InputModeState::ObservedKana
        };
        return (Some(mode), ModeSource::RomajiFlag);
    }
    if matches!(i.current, InputModeState::ObservedEisu)
        && i.conv.is_some_and(|c| !ConvMode::from_u32(c).is_eisu())
    {
        return (
            Some(InputModeState::AssumedRomaji {
                reason: AssumedReason::AppKindExcluded,
            }),
            ModeSource::StaleRecovery,
        );
    }
    (None, ModeSource::None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use InputModeState::{AssumedRomaji, ObservedEisu, ObservedKana, ObservedRomaji, Unknown};

    fn input(
        ime_on: Option<bool>,
        is_romaji: Option<bool>,
        conv: Option<u32>,
        current: InputModeState,
    ) -> SnapshotModeInput {
        SnapshotModeInput {
            ime_on,
            is_romaji,
            conv,
            current,
            guard_active: false,
        }
    }

    const STALE: InputModeState = AssumedRomaji {
        reason: AssumedReason::AppKindExcluded,
    };

    /// (i) 場合 E(BUG-57): IME が閉じた窓の conv=0 は英数の証拠にならず、prev が無くても(構造で)採らない。
    /// 以前は `classify_transition` の英数遷移が prev に依存して、prev が生きていればこれを ObservedEisu にしていた。
    #[test]
    fn closed_ime_conv_zero_is_not_adopted() {
        let i = input(Some(false), None, Some(0x0000), ObservedRomaji);
        assert_eq!(decide_input_mode(&i), (None, ModeSource::None));
        let i = input(Some(false), None, Some(0x0010), ObservedRomaji);
        assert_eq!(decide_input_mode(&i), (None, ModeSource::None));
    }

    /// (ii) 場合 K: ROMAN ビットなしの conv(0x09)で `is_romaji=None`(直接の読みの失敗)なら、ObservedKana にしない。
    /// ImmCross 抑制・`classify_idle` の `is_roman_reliable=false` が決めた方針と同じ。
    #[test]
    fn conv_without_roman_bit_and_unknown_flag_is_not_adopted_as_kana() {
        let i = input(Some(true), None, Some(0x0009), ObservedRomaji);
        assert_eq!(decide_input_mode(&i), (None, ModeSource::None));
    }

    /// (iii) stale 回復: ObservedEisu のとき、英数でない conv(0x09)で `is_romaji=None` なら AssumedRomaji に戻す。
    #[test]
    fn stale_eisu_recovers_to_assumed_romaji() {
        let i = input(Some(true), None, Some(0x0009), ObservedEisu);
        assert_eq!(
            decide_input_mode(&i),
            (Some(STALE), ModeSource::StaleRecovery)
        );
        // 境界: `is_romaji=Some(false)` なら romaji フラグが ObservedKana を返して終わる(stale 回復へは落ちない)。
        let i = input(Some(true), Some(false), Some(0x0009), ObservedEisu);
        assert_eq!(
            decide_input_mode(&i),
            (Some(ObservedKana), ModeSource::RomajiFlag)
        );
    }

    /// (iv) guard 中で `is_romaji` が不明なら何も採らない。
    #[test]
    fn guard_with_unknown_romaji_adopts_nothing() {
        let mut i = input(Some(true), None, Some(0x0009), ObservedEisu);
        i.guard_active = true;
        assert_eq!(decide_input_mode(&i), (None, ModeSource::None));
    }

    /// (v) guard は英数の証拠より先: force-on guard 中は `ime_on=Some(true)`・conv=0 でも ObservedEisu を出さない。
    #[test]
    fn guard_beats_eisu_evidence() {
        let mut i = input(Some(true), None, Some(0), ObservedRomaji);
        i.guard_active = true;
        assert_eq!(decide_input_mode(&i), (None, ModeSource::None));
        i.guard_active = false;
        assert_eq!(
            decide_input_mode(&i),
            (Some(ObservedEisu), ModeSource::Eisu)
        );
    }

    /// (vi) 英数の証拠が真で現在が既に ObservedEisu なら `None` で終わる(romaji フラグ・stale 回復へ落ちない)。
    #[test]
    fn eisu_branch_ends_even_when_already_eisu() {
        let i = input(Some(true), None, Some(0), ObservedEisu);
        assert_eq!(decide_input_mode(&i), (None, ModeSource::Eisu));
        // `is_romaji` が埋まる将来の変更でも、英数の証拠が先に終わらせる(else-if の分岐で、`or_else` の連鎖ではない)。
        let i = input(Some(true), Some(true), Some(0), ObservedEisu);
        assert_eq!(decide_input_mode(&i), (None, ModeSource::Eisu));
    }

    /// romaji フラグ: ローマ字→ObservedRomaji、かな→ObservedKana(現在の mode に関わらず毎回採る)。
    #[test]
    fn romaji_flag_is_adopted_every_time() {
        let i = input(Some(true), Some(true), Some(0x19), ObservedRomaji);
        assert_eq!(
            decide_input_mode(&i),
            (Some(ObservedRomaji), ModeSource::RomajiFlag)
        );
        let i = input(Some(true), Some(false), Some(0x09), ObservedRomaji);
        assert_eq!(
            decide_input_mode(&i),
            (Some(ObservedKana), ModeSource::RomajiFlag)
        );
    }

    /// 移動の前後で一致することの表テスト: 元の `classify_ime_snapshot` の枝(prev=None のとき、`classify_transition` の枝は常に
    /// `None`)を、独立に書き直した期待値と全組合せで突き合わせる。
    #[test]
    fn exhaustive_table_matches_the_original_else_if_chain() {
        let ime_ons = [None, Some(true), Some(false)];
        let romajis = [None, Some(true), Some(false)];
        let convs = [None, Some(0x00), Some(0x10), Some(0x09), Some(0x19)];
        let currents = [ObservedRomaji, ObservedKana, ObservedEisu, STALE, Unknown];
        let mut cases = 0;
        for ime_on in ime_ons {
            for is_romaji in romajis {
                for conv in convs {
                    for current in currents {
                        for guard_active in [false, true] {
                            let i = SnapshotModeInput {
                                ime_on,
                                is_romaji,
                                conv,
                                current,
                                guard_active,
                            };
                            // 元の else-if の鎖(prev=None なので classify_transition の枝は None)を、そのまま書き下した期待値。
                            let eisu_ev = if ime_on == Some(false) {
                                None
                            } else {
                                conv.map(|c| ConvMode::from_u32(c).is_eisu())
                            };
                            let expected = if guard_active && is_romaji.is_none() {
                                None
                            } else if eisu_ev == Some(true) {
                                (!matches!(current, ObservedEisu)).then_some(ObservedEisu)
                            } else if let Some(r) = is_romaji {
                                Some(if r { ObservedRomaji } else { ObservedKana })
                            } else if matches!(current, ObservedEisu)
                                && conv.is_some_and(|c| !ConvMode::from_u32(c).is_eisu())
                            {
                                Some(STALE)
                            } else {
                                None
                            };
                            assert_eq!(decide_input_mode(&i).0, expected, "{i:?}");
                            cases += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(cases, 3 * 3 * 5 * 5 * 2);
    }
}
