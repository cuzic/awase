//! IME 変換モード (`ImmGetConversionStatus`) の型安全な表現と分類ロジック。
//!
//! `ConvMode` は Windows IMM32 の u32 ビットフィールドのうち、awase が
//! 追跡する2軸（`eisu`: 英数かどうか、`romaji`: ローマ字入力かどうか）
//! だけを取り出した値型。かな形状（ひらがな/カタカナ・全角/半角）の軸は
//! ADR-094 で追跡を撤去した（ADR-091 決定3 §D3.1「charset 軸について
//! awase が特定の状態を予測して belief 化することはしない」の実装）。
//! Win32 API を呼ばないため Linux でもコンパイル・テスト可能。

use std::fmt;

use crate::engine::{AssumedReason, InputModeState};

// ImmGetConversionStatus のビット定数 (imm.h)
const IME_CMODE_NATIVE: u32 = 0x0001;
const IME_CMODE_ROMAN: u32 = 0x0010;

impl fmt::Display for ConvMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{}",
            if self.eisu { "Eisu" } else { "Kana" },
            if self.romaji { "roma" } else { "kana" }
        )
    }
}

/// IME 変換モードのうち awase が追跡する2軸を表す値型。
///
/// - `eisu`: 英数モードかどうか（NATIVE ビット無し）。親指シフトエンジンの
///   活性判断（`ObservedEisu`、`state/eisu_recovery.rs`）に使う。
/// - `romaji`: ローマ字入力かどうか（ROMAN ビットあり）。`false` は JIS
///   かな直接入力を意味する。awase はこの入力方式を非サポートとしており
///   （ADR-091 決定2、BUG-61 で「Win32 に外部から切り替える公式 API が
///   存在しない」と実機確定済み）、`swallow_alt_kana_input_method_switch`
///   が遷移そのものを予防的に遮断する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConvMode {
    pub eisu: bool,
    pub romaji: bool,
}

impl ConvMode {
    /// `ImmGetConversionStatus` の raw conv 値から生成する。
    #[must_use]
    pub const fn from_u32(conv: u32) -> Self {
        Self {
            eisu: conv & IME_CMODE_NATIVE == 0,
            romaji: conv & IME_CMODE_ROMAN != 0,
        }
    }

    /// 英数モード (NATIVE=0) かどうか。ROMAN ビットの有無は関係ない。
    ///
    /// MS-IME は半角英数モードでも ROMAN ビット (0x10) をセットしたまま返す場合がある
    /// (conv=0x0010)。
    #[must_use]
    pub const fn is_eisu(self) -> bool {
        self.eisu
    }

    /// `conv` が「ユーザーが英数モードを選んだ」証拠として使えるかどうかを判定する。
    ///
    /// IME が閉じている(`ime_on == Some(false)`)窓では conv=0（= NATIVE ビット無し
    /// = `is_eisu()` 真）が自明に成立し、これはユーザーの選択を反映したものではない
    /// （フォーカスが一瞬通り過ぎただけの無関係な窓でも同じ値になる）。したがって
    /// `ime_on` が明確に false のときは判定不能として `None` を返し、呼び出し元に
    /// `input_mode` を書き換えさせない。`ime_on` が `Some(true)` または `None`
    /// （TsfNative 等、open 状態不明）のときは従来どおり `conv` から判定する
    /// （2026-08-07 実機: Pushbullet 通知ポップアップが一瞬フォーカスを奪った際、
    /// その窓の conv=0 が `ObservedEisu` として belief に書き込まれ、フォーカスが
    /// 元のウィンドウへ戻った後も残留し、最初のキー入力がリテラル化した）。
    #[must_use]
    pub fn is_eisu_evidence(ime_on: Option<bool>, conv: Option<u32>) -> Option<bool> {
        if ime_on == Some(false) {
            return None;
        }
        conv.map(|c| Self::from_u32(c).is_eisu())
    }

    /// idle 中の conv ポーリング値から belief の `InputModeState` を分類する。
    ///
    /// `classify_idle_conv(u32, ...)` の `ConvMode` 版。Win32 API を呼ばない純粋関数。
    ///
    /// # 引数
    /// - `is_cold_start`: `output_in_flight_ms() == u64::MAX`（ROMAN ビット未確定期間）
    /// - `current`: 現在の `InputModeState` belief
    /// - `is_roman_reliable`: ROMAN ビット (0x10) が信頼できるかどうか。
    ///   通常の IMM32 ウィンドウでは `true`。TsfNative (WezTerm 等) では ROMAN ビットが
    ///   常に 0 のため `false` を渡す。`false` の場合、ひらがな conv で ObservedKana への
    ///   downgrade を行わず、非 romaji-capable なら `AssumedRomaji` に回復する。
    #[must_use]
    pub fn classify_idle(
        self,
        is_cold_start: bool,
        current: InputModeState,
        is_roman_reliable: bool,
    ) -> Option<InputModeState> {
        use InputModeState::{ObservedEisu, ObservedKana, ObservedRomaji};

        // 英数モード: cold start でも ROMAN=0 なので確実に判定可能
        if self.is_eisu() {
            return (!matches!(current, ObservedEisu)).then_some(ObservedEisu);
        }
        // ROMAN ビットは cold start 中は信頼できない
        if self.romaji && is_cold_start {
            return None;
        }
        if self.romaji {
            // ローマ字モード
            return (!current.is_romaji_capable()).then_some(ObservedRomaji);
        }
        // ROMAN=0 かつ NATIVE=1（charset 軸は追跡しないため、かな形状に関わらず
        // 同一に扱う。ADR-091 決定2 により JIS かな入力は非サポートのため、この
        // 分岐は「JIS かな直接入力を観測した」という単一の意味になる）。
        if is_roman_reliable {
            // ROMAN=0 が信頼できる: JISかな。romaji-capable なら訂正。
            current.is_romaji_capable().then_some(ObservedKana)
        } else {
            // ROMAN=0 が信頼できない (TsfNative): ローマ字/JISかな不明。
            // romaji-capable なら変更なし。そうでなければ AssumedRomaji に回復。
            if current.is_romaji_capable() {
                None
            } else {
                Some(InputModeState::AssumedRomaji {
                    reason: AssumedReason::ImmBridgeBroken,
                })
            }
        }
    }

    /// conv モードの前後差分から belief の `InputModeState` を分類する。
    ///
    /// `classify_conv_transition(u32, u32, ...)` の `ConvMode` 版。Win32 API を呼ばない純粋関数。
    ///
    /// # 注意: 英数遷移の特殊ケース
    /// `self` が英数モードかつ `prev` が非英数だった場合、
    /// `current` に関わらず `Some(ObservedEisu)` を返す（belief を強制補正）。
    #[must_use]
    pub const fn classify_transition(
        self,
        prev: Self,
        current: InputModeState,
    ) -> Option<InputModeState> {
        use InputModeState::{ObservedEisu, ObservedKana, ObservedRomaji};

        // 英数モードへの遷移 → 常に ObservedEisu
        if self.is_eisu() && !prev.is_eisu() {
            return Some(ObservedEisu);
        }
        // ROMAN ビット変化 かつ NATIVE あり → ひらがな↔ローマ字切り替え
        let roman_changed = prev.romaji != self.romaji;
        let curr_has_native = !self.eisu;
        if !(roman_changed && curr_has_native) {
            return None;
        }
        // belief が既に新方向と一致していれば更新不要
        if current.is_romaji_capable() == self.romaji {
            return None;
        }
        Some(if self.romaji {
            ObservedRomaji
        } else {
            ObservedKana
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::AssumedReason;
    use InputModeState::{AssumedRomaji, ObservedEisu, ObservedKana, ObservedRomaji, Unknown};

    // ── テスト用ヘルパー ──────────────────────────────────────────────────────

    fn assumed() -> InputModeState {
        AssumedRomaji {
            reason: AssumedReason::ImmBridgeBroken,
        }
    }

    fn cm(conv: u32) -> ConvMode {
        ConvMode::from_u32(conv)
    }

    // 代表的な conv 値
    const CONV_EISUU: u32 = 0x0000; // 半角英数
    const CONV_ZENALPHA: u32 = 0x0008; // 全角英数 (FULLSHAPE)
    const CONV_HIRAGANA: u32 = 0x0019; // ひらがなローマ字 (NATIVE|FULLSHAPE|ROMAN)
    const CONV_JISAKANA: u32 = 0x0009; // JISかな (NATIVE|FULLSHAPE)

    // ── from_u32 ────────────────────────────────────────────────────────────
    #[test]
    fn from_u32_hiragana_romaji() {
        let m = cm(CONV_HIRAGANA);
        assert!(!m.eisu);
        assert!(m.romaji);
    }

    #[test]
    fn from_u32_jisakana() {
        let m = cm(CONV_JISAKANA);
        assert!(!m.eisu);
        assert!(!m.romaji);
    }

    #[test]
    fn from_u32_eisuu() {
        let m = cm(CONV_EISUU);
        assert!(m.eisu);
        assert!(!m.romaji);
        assert!(m.is_eisu());
    }

    #[test]
    fn from_u32_zenalpha() {
        let m = cm(CONV_ZENALPHA);
        assert!(m.eisu);
        assert!(!m.romaji);
        assert!(m.is_eisu());
    }

    #[test]
    fn from_u32_hanalpha_roma() {
        // MS-IME が半角英数モードで ROMAN ビット (0x10) をセットする場合
        let m = cm(0x0010);
        assert!(m.eisu);
        assert!(m.romaji);
        assert!(m.is_eisu()); // NATIVE=0 なら ROMAN ビット不問
    }

    #[test]
    fn from_u32_zenalpha_roma() {
        // 全角英数 + ROMAN ビット (0x0018)
        let m = cm(0x0018);
        assert!(m.eisu);
        assert!(m.romaji);
        assert!(m.is_eisu());
    }

    // ── is_eisu_evidence ─────────────────────────────────────────────────────
    #[test]
    fn is_eisu_evidence_ignores_conv_zero_when_ime_off() {
        // IME が閉じている窓の conv=0 は「英数選択」の証拠にならない(BUG-57)。
        assert_eq!(
            ConvMode::is_eisu_evidence(Some(false), Some(CONV_EISUU)),
            None
        );
    }

    #[test]
    fn is_eisu_evidence_true_when_ime_on() {
        // トレイの半角英数コマンド等、IME が開いた状態での英数選択は従来どおり有効。
        assert_eq!(
            ConvMode::is_eisu_evidence(Some(true), Some(CONV_EISUU)),
            Some(true)
        );
    }

    #[test]
    fn is_eisu_evidence_false_when_ime_on_and_not_eisu() {
        assert_eq!(
            ConvMode::is_eisu_evidence(Some(true), Some(CONV_HIRAGANA)),
            Some(false)
        );
    }

    #[test]
    fn is_eisu_evidence_uses_conv_when_ime_on_unknown() {
        // TsfNative 等、open 状態不明時は従来どおり conv から判定する。
        assert_eq!(
            ConvMode::is_eisu_evidence(None, Some(CONV_EISUU)),
            Some(true)
        );
    }

    #[test]
    fn is_eisu_evidence_none_when_conv_unknown() {
        assert_eq!(ConvMode::is_eisu_evidence(Some(true), None), None);
    }

    // ── classify_idle (is_roman_reliable=true: 通常 IMM32) ────────────────────

    // 英数
    #[test]
    fn idle_eisuu_from_romaji_yields_eisu() {
        assert_eq!(
            cm(CONV_EISUU).classify_idle(false, ObservedRomaji, true),
            Some(ObservedEisu)
        );
    }

    #[test]
    fn idle_eisuu_from_assumed_yields_eisu() {
        assert_eq!(
            cm(CONV_EISUU).classify_idle(false, assumed(), true),
            Some(ObservedEisu)
        );
    }

    #[test]
    fn idle_eisuu_from_kana_yields_eisu() {
        // ObservedKana（かな）から英数に変わったので ObservedEisu に更新
        assert_eq!(
            cm(CONV_EISUU).classify_idle(false, ObservedKana, true),
            Some(ObservedEisu)
        );
    }

    #[test]
    fn idle_eisuu_from_eisu_yields_none() {
        // 既に ObservedEisu なら変更なし
        assert_eq!(
            cm(CONV_EISUU).classify_idle(false, ObservedEisu, true),
            None
        );
    }

    #[test]
    fn idle_eisuu_cold_start_still_classifies() {
        assert_eq!(
            cm(CONV_EISUU).classify_idle(true, ObservedRomaji, true),
            Some(ObservedEisu)
        );
    }

    // ひらがなローマ字
    #[test]
    fn idle_hiragana_from_kana_yields_romaji() {
        assert_eq!(
            cm(CONV_HIRAGANA).classify_idle(false, ObservedKana, true),
            Some(ObservedRomaji)
        );
    }

    #[test]
    fn idle_hiragana_from_romaji_yields_none() {
        assert_eq!(
            cm(CONV_HIRAGANA).classify_idle(false, ObservedRomaji, true),
            None
        );
    }

    #[test]
    fn idle_hiragana_cold_start_skips() {
        assert_eq!(
            cm(CONV_HIRAGANA).classify_idle(true, ObservedKana, true),
            None
        );
        assert_eq!(cm(CONV_HIRAGANA).classify_idle(true, Unknown, true), None);
    }

    // JISかな (is_roman_reliable=true)
    #[test]
    fn idle_jisakana_from_romaji_yields_kana() {
        assert_eq!(
            cm(CONV_JISAKANA).classify_idle(false, ObservedRomaji, true),
            Some(ObservedKana)
        );
    }

    #[test]
    fn idle_jisakana_cold_start_classifies() {
        assert_eq!(
            cm(CONV_JISAKANA).classify_idle(true, ObservedRomaji, true),
            Some(ObservedKana)
        );
    }

    // 半角英数/ローマ字 (conv=0x0010): ROMAN ビットがあっても英数モード
    #[test]
    fn idle_hanalpha_roma_from_assumed_yields_eisu() {
        // is_roman_reliable に関わらず ObservedEisu を返す
        assert_eq!(
            cm(0x0010).classify_idle(false, assumed(), true),
            Some(ObservedEisu)
        );
        assert_eq!(
            cm(0x0010).classify_idle(false, assumed(), false),
            Some(ObservedEisu)
        );
    }

    #[test]
    fn idle_hanalpha_roma_from_romaji_yields_eisu() {
        assert_eq!(
            cm(0x0010).classify_idle(false, ObservedRomaji, true),
            Some(ObservedEisu)
        );
        assert_eq!(
            cm(0x0010).classify_idle(false, ObservedRomaji, false),
            Some(ObservedEisu)
        );
    }

    #[test]
    fn idle_hanalpha_roma_from_kana_yields_eisu() {
        // ObservedKana（かな）から英数に変わったので ObservedEisu に更新
        assert_eq!(
            cm(0x0010).classify_idle(false, ObservedKana, true),
            Some(ObservedEisu)
        );
        assert_eq!(
            cm(0x0010).classify_idle(false, ObservedKana, false),
            Some(ObservedEisu)
        );
    }

    #[test]
    fn idle_hanalpha_roma_from_eisu_yields_none() {
        // 既に ObservedEisu なら変更なし
        assert_eq!(cm(0x0010).classify_idle(false, ObservedEisu, true), None);
        assert_eq!(cm(0x0010).classify_idle(false, ObservedEisu, false), None);
    }

    #[test]
    fn idle_hanalpha_roma_cold_start_still_classifies() {
        // is_eisu() ブランチは cold start でも早期リターンする
        assert_eq!(
            cm(0x0010).classify_idle(true, assumed(), true),
            Some(ObservedEisu)
        );
    }

    // ── classify_idle (is_roman_reliable=false: TsfNative) ────────────────────

    // ひらがな系 conv (CONV_JISAKANA: NATIVE=1, ROMAN=0)
    #[test]
    fn idle_jisakana_tsf_assumed_yields_none() {
        // TsfNative: AssumedRomaji は変更なし（downgrade 抑制）
        assert_eq!(
            cm(CONV_JISAKANA).classify_idle(false, assumed(), false),
            None
        );
    }

    #[test]
    fn idle_jisakana_tsf_romaji_yields_none() {
        // TsfNative: ObservedRomaji も変更なし（ROMAN=0 は信頼できない）
        assert_eq!(
            cm(CONV_JISAKANA).classify_idle(false, ObservedRomaji, false),
            None
        );
    }

    #[test]
    fn idle_jisakana_tsf_kana_recovers_assumed() {
        // TsfNative: ObservedKana → AssumedRomaji に回復
        assert_eq!(
            cm(CONV_JISAKANA).classify_idle(false, ObservedKana, false),
            Some(assumed())
        );
    }

    #[test]
    fn idle_jisakana_tsf_unknown_recovers_assumed() {
        // TsfNative: Unknown → AssumedRomaji に回復
        assert_eq!(
            cm(CONV_JISAKANA).classify_idle(false, Unknown, false),
            Some(assumed())
        );
    }

    // 英数 conv は is_roman_reliable に依存しない
    #[test]
    fn idle_eisuu_tsf_from_assumed_yields_eisu() {
        // HanAlpha は ROMAN bit 関係なく英数モード確定 → ObservedEisu
        assert_eq!(
            cm(CONV_EISUU).classify_idle(false, assumed(), false),
            Some(ObservedEisu)
        );
    }

    #[test]
    fn idle_eisuu_tsf_from_eisu_yields_none() {
        // 既に ObservedEisu なら変更なし
        assert_eq!(
            cm(CONV_EISUU).classify_idle(false, ObservedEisu, false),
            None
        );
    }

    #[test]
    fn idle_jisakana_tsf_from_eisu_recovers_assumed() {
        // TsfNative: ObservedEisu → AssumedRomaji に回復（かなモードへの復帰）
        assert_eq!(
            cm(CONV_JISAKANA).classify_idle(false, ObservedEisu, false),
            Some(assumed())
        );
    }

    // ── classify_transition ──────────────────────────────────────────────────

    // 英数遷移
    #[test]
    fn tr_hiragana_to_eisu_always_eisu() {
        assert_eq!(
            cm(CONV_EISUU).classify_transition(cm(CONV_HIRAGANA), ObservedRomaji),
            Some(ObservedEisu)
        );
        // belief が ObservedKana でも ObservedEisu を返す（強制補正）
        assert_eq!(
            cm(CONV_EISUU).classify_transition(cm(CONV_JISAKANA), ObservedKana),
            Some(ObservedEisu)
        );
    }

    #[test]
    fn tr_eisu_to_eisu_yields_none() {
        assert_eq!(
            cm(CONV_EISUU).classify_transition(cm(CONV_EISUU), ObservedRomaji),
            None
        );
    }

    // ROMAN bit 変化
    #[test]
    fn tr_jisakana_to_hiragana_yields_romaji() {
        assert_eq!(
            cm(CONV_HIRAGANA).classify_transition(cm(CONV_JISAKANA), ObservedKana),
            Some(ObservedRomaji)
        );
    }

    #[test]
    fn tr_hiragana_to_jisakana_yields_kana() {
        assert_eq!(
            cm(CONV_JISAKANA).classify_transition(cm(CONV_HIRAGANA), ObservedRomaji),
            Some(ObservedKana)
        );
    }

    #[test]
    fn tr_already_matches_yields_none() {
        // JISかな → ひらがな だが belief が既に Romaji
        assert_eq!(
            cm(CONV_HIRAGANA).classify_transition(cm(CONV_JISAKANA), ObservedRomaji),
            None
        );
    }

    /// `&&`→`||` の反転を殺すテスト。`roman_changed=false, curr_has_native=true`
    /// を使うが、`current` に `ObservedKana`（`self.romaji=false` と一致）を
    /// 渡していたため、mutants で `||` に反転しても後続の第2ガード（belief
    /// 一致判定）が偶然 `None` を返し、結果が変わらず検知できなかった。
    /// ここでは `self.romaji=false` と *不一致* な `ObservedRomaji` を渡す
    /// ことで、第2ガードでは None にならないケースを作り、第1ガード
    /// (`roman_changed && curr_has_native`) 自体の反転を露出させる。
    #[test]
    fn tr_no_roman_change_yields_none_even_with_mismatched_belief() {
        // JISAKANA(romaji=false) ← JISAKANA(romaji=false): roman_changed=false,
        // curr_has_native=true。
        assert_eq!(
            cm(CONV_JISAKANA).classify_transition(cm(CONV_JISAKANA), ObservedRomaji),
            None
        );
    }

    /// 上記の対称ケース: `roman_changed=true, curr_has_native=false`。
    /// `curr_has_native=false` にするには self が eisu である必要があるが、
    /// 単純に eisu へ遷移すると最初の分岐（`self.is_eisu() && !prev.is_eisu()`）が
    /// 先に発火してしまうため、`prev` も eisu にして最初の分岐を回避する
    /// （HankakuAlpha は romaji ビットの有無を問わず eisu = true。
    /// `from_u32_hanalpha_roma` 参照）。
    #[test]
    fn tr_roman_change_without_native_yields_none() {
        // HankakuAlpha+romaji(0x0010) ← HankakuAlpha(CONV_EISUU, romaji なし):
        // どちらも eisu なので最初の分岐は通らない。roman_changed=true,
        // curr_has_native=false。
        assert_eq!(
            cm(0x0010).classify_transition(cm(CONV_EISUU), ObservedKana),
            None
        );
    }

    // ── 全モード網羅 ──────────────────────────────────────────────────────────
    #[test]
    fn all_eisu_modes_from_romaji_yield_eisu_on_idle() {
        for conv in [CONV_EISUU, CONV_ZENALPHA, 0x0010, 0x0018] {
            assert_eq!(
                cm(conv).classify_idle(false, ObservedRomaji, true),
                Some(ObservedEisu),
                "conv=0x{conv:08X}"
            );
        }
    }

    #[test]
    fn jisakana_from_romaji_yields_kana_on_idle() {
        assert_eq!(
            cm(CONV_JISAKANA).classify_idle(false, ObservedRomaji, true),
            Some(ObservedKana)
        );
    }

    #[test]
    fn hiragana_from_kana_yields_romaji_on_idle() {
        assert_eq!(
            cm(CONV_HIRAGANA).classify_idle(false, ObservedKana, true),
            Some(ObservedRomaji)
        );
    }
}
