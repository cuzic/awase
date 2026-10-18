//! 英数モードの「候補」と確認(ADR-238、BUG-190)。
//!
//! MS-IME の OS ポーリングは稀に一過性の `conv=0x00000000` を返す(前後の読みは 0x19)。これを 1 回の観測で
//! `ObservedEisu` と採用すると、Engine が `Inactive(NotRomajiInput)` で次の読みまで止まり、その間の打鍵が
//! 生のまま IME に渡る(手元の CI の MS-IME で 5/約 300 run、打鍵中の除外で捨てられた読みを含めると約 5102 読み中 3 件の孤立)。
//!
//! そこで「新たに `ObservedEisu` へ変わる結果」は、1 回目を採らず**候補**として持ち、確認の読みでも英数なら確定する。
//! 本物の英数への切替は 1 回の確認で確定する(確認の読みは `reschedule_ime_refresh` が 60ms 間隔で予約する)。
//! 予測(`KeyEffectPredicted` の mode=Eisu)が先に belief を `ObservedEisu` にしている場合は、そもそも「新たに変わる」
//! 結果ではないので、素通しにして候補を作らない。
//!
//! 判定は副作用の無い純粋関数にし、`observer/`(`#[cfg(windows)]`)の外に置いて Linux のテストで固定する。

use awase::engine::InputModeState;

/// 英数の候補(1 回目の英数の読み)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EisuCandidate {
    /// 候補にした読みの時刻(`GetTickCount64` 由来の ms)。
    pub at_ms: u64,
    /// そのときの conv。
    pub conv: u32,
}

/// 候補の更新命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateUpdate {
    /// 候補をそのまま保つ。
    Keep,
    /// 候補を置く(上書き)。
    Set(EisuCandidate),
    /// 候補を捨てる。
    Clear,
}

/// [`filter_eisu_adoption`] の入力(読みの結果と現在の状態)。
#[derive(Debug, Clone, Copy)]
pub struct EisuFilterInput {
    /// 分類の結果(`classify_ime_snapshot` が返そうとしている `new_input_mode`)。
    pub proposed: Option<InputModeState>,
    /// 現在の belief の input_mode。
    pub current_mode: InputModeState,
    /// この読みの `ime_on`(`None` = open プローブが失敗/時間切れ)。
    pub ime_on: Option<bool>,
    /// この読みの conv(`None` = 取れなかった/TsfNative)。
    pub conv: Option<u32>,
    /// この読みが英数の conv か(`conv` が `Some` のときだけ意味を持つ)。
    pub conv_is_eisu: bool,
    /// 現在の時刻(ms)。
    pub now_ms: u64,
    /// 現在の候補。
    pub candidate: Option<EisuCandidate>,
    /// 候補の寿命(ms)。`tuning::EISU_CANDIDATE_LIFETIME_MS`。
    pub lifetime_ms: u64,
}

/// 英数の採用に確認を足す。戻り値は (実際に採用する `new_input_mode`, 候補の更新)。
///
/// 規則:
/// 1. `proposed != Some(ObservedEisu)`、または `current_mode` が既に `ObservedEisu`(予測が先に動かした場合を含む)なら、
///    素通し。conv が非英数と分かる読み、または英数以外の mode を採る結果は、候補を捨てる。
/// 2. `ime_on == None`(open の読みが時間切れ)の読みでは、英数を採らず候補も作らない(B。wx 型)。既存の候補は保つ。
/// 3. 生きた候補(`now - at <= lifetime`)があれば確定(採用し、候補を捨てる)。
/// 4. 候補が無い/寿命切れなら、採らずに候補を置く。
#[must_use]
pub fn filter_eisu_adoption(i: &EisuFilterInput) -> (Option<InputModeState>, CandidateUpdate) {
    let is_new_eisu = i.proposed == Some(InputModeState::ObservedEisu)
        && i.current_mode != InputModeState::ObservedEisu;
    if !is_new_eisu {
        // 英数でない読み(conv が分かって非英数)や、英数以外の mode を採る結果は、候補を捨てる。
        let non_eisu_evidence = i.conv.is_some() && !i.conv_is_eisu;
        let other_mode = i.proposed.is_some() && i.proposed != Some(InputModeState::ObservedEisu);
        let update = if non_eisu_evidence || other_mode {
            CandidateUpdate::Clear
        } else {
            CandidateUpdate::Keep
        };
        return (i.proposed, update);
    }
    // 以下は「新たに ObservedEisu へ変わる結果」。
    if i.ime_on.is_none() {
        return (None, CandidateUpdate::Keep);
    }
    let conv = i.conv.unwrap_or(0);
    match i.candidate {
        Some(c) if i.now_ms.saturating_sub(c.at_ms) <= i.lifetime_ms => {
            (Some(InputModeState::ObservedEisu), CandidateUpdate::Clear)
        }
        _ => (
            None,
            CandidateUpdate::Set(EisuCandidate {
                at_ms: i.now_ms,
                conv,
            }),
        ),
    }
}

/// 候補の寿命の残り(ms)。候補が無い/寿命切れなら `None`。確認の読み直しの予約(`reschedule_ime_refresh`)が使う。
#[must_use]
pub const fn candidate_remaining_ms(
    candidate: Option<EisuCandidate>,
    now_ms: u64,
    lifetime_ms: u64,
) -> Option<u64> {
    match candidate {
        Some(c) => {
            let age = now_ms.saturating_sub(c.at_ms);
            if age <= lifetime_ms {
                Some(lifetime_ms - age)
            } else {
                None
            }
        }
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use InputModeState::{ObservedEisu, ObservedKana, ObservedRomaji};

    const LIFE: u64 = 1500;

    fn input(proposed: Option<InputModeState>) -> EisuFilterInput {
        EisuFilterInput {
            proposed,
            current_mode: ObservedRomaji,
            ime_on: Some(true),
            conv: Some(0),
            conv_is_eisu: true,
            now_ms: 10_000,
            candidate: None,
            lifetime_ms: LIFE,
        }
    }

    /// BUG-190: 孤立した 1 回の conv=0 は採らず、候補にする。
    #[test]
    fn first_eisu_reading_becomes_a_candidate_not_an_adoption() {
        let (mode, upd) = filter_eisu_adoption(&input(Some(ObservedEisu)));
        assert_eq!(mode, None);
        assert_eq!(
            upd,
            CandidateUpdate::Set(EisuCandidate {
                at_ms: 10_000,
                conv: 0
            })
        );
    }

    /// 確認の読みでも英数なら確定する(本物の切替)。
    #[test]
    fn second_eisu_reading_within_lifetime_confirms() {
        let mut i = input(Some(ObservedEisu));
        i.candidate = Some(EisuCandidate {
            at_ms: 9_940,
            conv: 0,
        });
        let (mode, upd) = filter_eisu_adoption(&i);
        assert_eq!(mode, Some(ObservedEisu));
        assert_eq!(upd, CandidateUpdate::Clear);
    }

    /// 寿命の境界: ちょうど寿命なら確定、1ms 超えたら新しい候補。
    #[test]
    fn candidate_lifetime_boundary() {
        let mut i = input(Some(ObservedEisu));
        i.candidate = Some(EisuCandidate {
            at_ms: 10_000 - LIFE,
            conv: 0,
        });
        assert_eq!(filter_eisu_adoption(&i).0, Some(ObservedEisu));
        i.candidate = Some(EisuCandidate {
            at_ms: 10_000 - LIFE - 1,
            conv: 0,
        });
        let (mode, upd) = filter_eisu_adoption(&i);
        assert_eq!(mode, None, "寿命切れの候補では確定しない");
        assert!(matches!(upd, CandidateUpdate::Set(c) if c.at_ms == 10_000));
    }

    /// 確認の読みが英数でなければ(0x19 に戻った)候補を捨てる。
    #[test]
    fn non_eisu_reading_clears_the_candidate() {
        let mut i = input(None);
        i.conv = Some(0x19);
        i.conv_is_eisu = false;
        i.candidate = Some(EisuCandidate {
            at_ms: 9_990,
            conv: 0,
        });
        assert_eq!(filter_eisu_adoption(&i), (None, CandidateUpdate::Clear));
    }

    /// 英数以外の mode を採る結果(ObservedRomaji 等)も候補を捨てる。素通し。
    #[test]
    fn other_mode_result_passes_through_and_clears() {
        let mut i = input(Some(ObservedRomaji));
        i.conv = Some(0x19);
        i.conv_is_eisu = false;
        assert_eq!(
            filter_eisu_adoption(&i),
            (Some(ObservedRomaji), CandidateUpdate::Clear)
        );
        let i = input(Some(ObservedKana));
        assert_eq!(filter_eisu_adoption(&i).0, Some(ObservedKana));
    }

    /// B: open の読みが時間切れ(`ime_on=None`)の読みでは英数を採らず、候補も作らない。既存の候補は保つ。
    #[test]
    fn ime_on_none_never_adopts_and_never_sets_a_candidate() {
        let mut i = input(Some(ObservedEisu));
        i.ime_on = None;
        assert_eq!(filter_eisu_adoption(&i), (None, CandidateUpdate::Keep));
        i.candidate = Some(EisuCandidate {
            at_ms: 9_990,
            conv: 0,
        });
        assert_eq!(
            filter_eisu_adoption(&i),
            (None, CandidateUpdate::Keep),
            "ime_on=None の読みは確認にも数えない(wx 型: open も conv も時間切れ寸前の読み)"
        );
    }

    /// 既に ObservedEisu のとき(予測が先に belief を動かした場合を含む)は素通し。候補を作らない。
    #[test]
    fn already_eisu_passes_through_without_a_candidate() {
        let mut i = input(Some(ObservedEisu));
        i.current_mode = ObservedEisu;
        assert_eq!(
            filter_eisu_adoption(&i),
            (Some(ObservedEisu), CandidateUpdate::Keep)
        );
    }

    /// 結果が None(分類が何も採らない)で conv が不明なら、候補を保つ。
    #[test]
    fn no_proposal_and_unknown_conv_keeps_the_candidate() {
        let mut i = input(None);
        i.conv = None;
        i.conv_is_eisu = false;
        assert_eq!(filter_eisu_adoption(&i), (None, CandidateUpdate::Keep));
    }

    #[test]
    fn remaining_ms_follows_the_lifetime() {
        let c = Some(EisuCandidate {
            at_ms: 1_000,
            conv: 0,
        });
        assert_eq!(candidate_remaining_ms(c, 1_000, LIFE), Some(LIFE));
        assert_eq!(candidate_remaining_ms(c, 1_000 + LIFE, LIFE), Some(0));
        assert_eq!(candidate_remaining_ms(c, 1_001 + LIFE, LIFE), None);
        assert_eq!(candidate_remaining_ms(None, 1_000, LIFE), None);
    }
}
