//! IME 変換モード (`ImmGetConversionStatus`) の型安全な表現と分類ロジック。
//!
//! `ConvMode` は Windows IMM32 の u32 ビットフィールドを意味のある 2 軸
//! （`Charset` + `romaji`）に変換した値型。Win32 API を呼ばないため
//! Linux でもコンパイル・テスト可能。

use std::fmt;

use crate::engine::InputModeState;

// ImmGetConversionStatus のビット定数 (imm.h)
const IME_CMODE_NATIVE: u32 = 0x0001;
const IME_CMODE_KATAKANA: u32 = 0x0002;
const IME_CMODE_FULLSHAPE: u32 = 0x0008;
const IME_CMODE_ROMAN: u32 = 0x0010;

/// IME が出力する文字の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charset {
    /// ひらがな (NATIVE|FULLSHAPE)
    Hiragana,
    /// 全角カタカナ (NATIVE|KATAKANA|FULLSHAPE)
    ZenkakuKatakana,
    /// 半角カタカナ (NATIVE|KATAKANA)
    HankakuKatakana,
    /// 全角英数 (FULLSHAPE のみ)
    ZenkakuAlpha,
    /// 半角英数 (フラグなし)
    HankakuAlpha,
}

impl Charset {
    /// カタカナ系（全角・半角）かどうか。
    pub fn is_katakana(self) -> bool {
        matches!(self, Self::ZenkakuKatakana | Self::HankakuKatakana)
    }
}

impl fmt::Display for Charset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Hiragana => "Hiragana",
            Self::ZenkakuKatakana => "ZenKata",
            Self::HankakuKatakana => "HanKata",
            Self::ZenkakuAlpha => "ZenAlpha",
            Self::HankakuAlpha => "HanAlpha",
        })
    }
}

impl fmt::Display for ConvMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.charset, if self.romaji { "roma" } else { "kana" })
    }
}

/// IME 変換モードを文字種と入力方式の 2 軸で表す値型。
///
/// `romaji` フィールドは Hiragana/Katakana 系のみ意味を持つ。
/// - `true`  = ローマ字入力 (ROMAN ビットあり)
/// - `false` = JISかな直接入力 (ROMAN ビットなし)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConvMode {
    pub charset: Charset,
    pub romaji: bool,
}

impl ConvMode {
    /// `ImmGetConversionStatus` の raw conv 値から生成する。
    pub fn from_u32(conv: u32) -> Self {
        let has_native = conv & IME_CMODE_NATIVE != 0;
        let has_katakana = conv & IME_CMODE_KATAKANA != 0;
        let has_fullshape = conv & IME_CMODE_FULLSHAPE != 0;
        let has_roman = conv & IME_CMODE_ROMAN != 0;

        let charset = if !has_native {
            if has_fullshape { Charset::ZenkakuAlpha } else { Charset::HankakuAlpha }
        } else if has_katakana {
            if has_fullshape { Charset::ZenkakuKatakana } else { Charset::HankakuKatakana }
        } else {
            Charset::Hiragana
        };

        ConvMode { charset, romaji: has_roman }
    }

    /// 英数モード (NATIVE=0 かつ ROMAN=0) かどうか。
    pub fn is_eisu(self) -> bool {
        !self.romaji && matches!(self.charset, Charset::HankakuAlpha | Charset::ZenkakuAlpha)
    }

    /// `ImmSetConversionStatus` の目標 conv 値を返す。
    ///
    /// カタカナ系は KATAKANA/FULLSHAPE ビットを明示的に復元する必要があるため `Some(conv)` を返す。
    /// それ以外は `current_conv | ROMAN` で十分なため `None`。
    pub fn imm_conv_target(self) -> Option<u32> {
        match self.charset {
            Charset::ZenkakuKatakana => Some(
                IME_CMODE_NATIVE | IME_CMODE_KATAKANA | IME_CMODE_FULLSHAPE | IME_CMODE_ROMAN,
            ),
            Charset::HankakuKatakana => {
                Some(IME_CMODE_NATIVE | IME_CMODE_KATAKANA | IME_CMODE_ROMAN)
            }
            _ => None,
        }
    }

    /// idle 中の conv ポーリング値から belief の `InputModeState` を分類する。
    ///
    /// `classify_idle_conv(u32, ...)` の `ConvMode` 版。Win32 API を呼ばない純粋関数。
    ///
    /// # 引数
    /// - `is_cold_start`: `output_in_flight_ms() == u64::MAX`（ROMAN ビット未確定期間）
    /// - `current`: 現在の `InputModeState` belief
    pub fn classify_idle(self, is_cold_start: bool, current: InputModeState) -> Option<InputModeState> {
        use InputModeState::{ObservedKana, ObservedRomaji};

        // 英数モード: cold start でも ROMAN=0 なので確実に判定可能
        if self.is_eisu() {
            return current.is_romaji_capable().then_some(ObservedKana);
        }
        // ROMAN ビットは cold start 中は信頼できない
        if self.romaji && is_cold_start {
            return None;
        }
        if self.romaji {
            // ローマ字モード
            return (!current.is_romaji_capable()).then_some(ObservedRomaji);
        }
        // ROMAN=0 かつ NATIVE=1
        match self.charset {
            Charset::ZenkakuKatakana | Charset::HankakuKatakana => {
                // カタカナ (NICOLA は romaji-capable として扱う)
                (!current.is_romaji_capable()).then_some(ObservedRomaji)
            }
            _ => {
                // JISかな / ひらがな直接入力
                current.is_romaji_capable().then_some(ObservedKana)
            }
        }
    }

    /// conv モードの前後差分から belief の `InputModeState` を分類する。
    ///
    /// `classify_conv_transition(u32, u32, ...)` の `ConvMode` 版。Win32 API を呼ばない純粋関数。
    ///
    /// # 注意: 英数遷移の特殊ケース
    /// `self` が英数モードかつ `prev` が非英数だった場合、
    /// `current` に関わらず `Some(ObservedKana)` を返す（belief を強制補正）。
    pub fn classify_transition(self, prev: ConvMode, current: InputModeState) -> Option<InputModeState> {
        use InputModeState::{ObservedKana, ObservedRomaji};

        // 英数モードへの遷移 → 常に ObservedKana
        if self.is_eisu() && !prev.is_eisu() {
            return Some(ObservedKana);
        }
        // ROMAN ビット変化 かつ NATIVE あり → ひらがな↔ローマ字切り替え
        let roman_changed = prev.romaji != self.romaji;
        let curr_has_native = !matches!(self.charset, Charset::HankakuAlpha | Charset::ZenkakuAlpha);
        if !(roman_changed && curr_has_native) {
            return None;
        }
        // belief が既に新方向と一致していれば更新不要
        if current.is_romaji_capable() == self.romaji {
            return None;
        }
        Some(if self.romaji { ObservedRomaji } else { ObservedKana })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use InputModeState::{AssumedRomaji, ObservedKana, ObservedRomaji, Unknown};
    use crate::engine::AssumedReason;

    // ── テスト用ヘルパー ──────────────────────────────────────────────────────

    fn assumed() -> InputModeState {
        AssumedRomaji { reason: AssumedReason::ImmBridgeBroken }
    }

    fn cm(conv: u32) -> ConvMode {
        ConvMode::from_u32(conv)
    }

    // 代表的な conv 値
    const CONV_EISUU: u32 = 0x0000;   // 半角英数
    const CONV_ZENALPHA: u32 = 0x0008; // 全角英数 (FULLSHAPE)
    const CONV_HIRAGANA: u32 = 0x0019; // ひらがなローマ字 (NATIVE|FULLSHAPE|ROMAN)
    const CONV_JISAKANA: u32 = 0x0009; // JISかな (NATIVE|FULLSHAPE)
    const CONV_ZENKATA: u32 = 0x000B;  // 全角カタカナ (NATIVE|KATAKANA|FULLSHAPE)
    const CONV_HANKATA: u32 = 0x0003;  // 半角カタカナ (NATIVE|KATAKANA)

    // ── from_u32 ────────────────────────────────────────────────────────────
    #[test]
    fn from_u32_hiragana_romaji() {
        let m = cm(CONV_HIRAGANA);
        assert_eq!(m.charset, Charset::Hiragana);
        assert!(m.romaji);
    }

    #[test]
    fn from_u32_jisakana() {
        let m = cm(CONV_JISAKANA);
        assert_eq!(m.charset, Charset::Hiragana);
        assert!(!m.romaji);
    }

    #[test]
    fn from_u32_zenkata() {
        let m = cm(CONV_ZENKATA);
        assert_eq!(m.charset, Charset::ZenkakuKatakana);
        assert!(!m.romaji);
    }

    #[test]
    fn from_u32_hankata() {
        let m = cm(CONV_HANKATA);
        assert_eq!(m.charset, Charset::HankakuKatakana);
        assert!(!m.romaji);
    }

    #[test]
    fn from_u32_eisuu() {
        let m = cm(CONV_EISUU);
        assert_eq!(m.charset, Charset::HankakuAlpha);
        assert!(!m.romaji);
        assert!(m.is_eisu());
    }

    #[test]
    fn from_u32_zenalpha() {
        let m = cm(CONV_ZENALPHA);
        assert_eq!(m.charset, Charset::ZenkakuAlpha);
        assert!(!m.romaji);
        assert!(m.is_eisu());
    }

    // ── imm_conv_target ──────────────────────────────────────────────────────
    #[test]
    fn imm_conv_target_zenkata() {
        assert_eq!(cm(CONV_ZENKATA).imm_conv_target(), Some(0x000B | 0x0010)); // NATIVE|KATA|FULL|ROMAN
    }

    #[test]
    fn imm_conv_target_hankata() {
        assert_eq!(cm(CONV_HANKATA).imm_conv_target(), Some(0x0003 | 0x0010)); // NATIVE|KATA|ROMAN
    }

    #[test]
    fn imm_conv_target_hiragana_none() {
        assert_eq!(cm(CONV_HIRAGANA).imm_conv_target(), None);
    }

    // ── classify_idle ────────────────────────────────────────────────────────

    // 英数
    #[test]
    fn idle_eisuu_from_romaji_yields_kana() {
        assert_eq!(cm(CONV_EISUU).classify_idle(false, ObservedRomaji), Some(ObservedKana));
    }

    #[test]
    fn idle_eisuu_from_assumed_yields_kana() {
        assert_eq!(cm(CONV_EISUU).classify_idle(false, assumed()), Some(ObservedKana));
    }

    #[test]
    fn idle_eisuu_from_kana_yields_none() {
        assert_eq!(cm(CONV_EISUU).classify_idle(false, ObservedKana), None);
    }

    #[test]
    fn idle_eisuu_cold_start_still_classifies() {
        assert_eq!(cm(CONV_EISUU).classify_idle(true, ObservedRomaji), Some(ObservedKana));
    }

    // ひらがなローマ字
    #[test]
    fn idle_hiragana_from_kana_yields_romaji() {
        assert_eq!(cm(CONV_HIRAGANA).classify_idle(false, ObservedKana), Some(ObservedRomaji));
    }

    #[test]
    fn idle_hiragana_from_romaji_yields_none() {
        assert_eq!(cm(CONV_HIRAGANA).classify_idle(false, ObservedRomaji), None);
    }

    #[test]
    fn idle_hiragana_cold_start_skips() {
        assert_eq!(cm(CONV_HIRAGANA).classify_idle(true, ObservedKana), None);
        assert_eq!(cm(CONV_HIRAGANA).classify_idle(true, Unknown), None);
    }

    // JISかな
    #[test]
    fn idle_jisakana_from_romaji_yields_kana() {
        assert_eq!(cm(CONV_JISAKANA).classify_idle(false, ObservedRomaji), Some(ObservedKana));
    }

    #[test]
    fn idle_jisakana_cold_start_classifies() {
        assert_eq!(cm(CONV_JISAKANA).classify_idle(true, ObservedRomaji), Some(ObservedKana));
    }

    // 全角カタカナ (NICOLA)
    #[test]
    fn idle_zenkata_from_kana_yields_romaji() {
        assert_eq!(cm(CONV_ZENKATA).classify_idle(false, ObservedKana), Some(ObservedRomaji));
    }

    #[test]
    fn idle_zenkata_from_romaji_yields_none() {
        assert_eq!(cm(CONV_ZENKATA).classify_idle(false, ObservedRomaji), None);
    }

    #[test]
    fn idle_zenkata_cold_start_classifies() {
        assert_eq!(cm(CONV_ZENKATA).classify_idle(true, ObservedKana), Some(ObservedRomaji));
    }

    // 半角カタカナ
    #[test]
    fn idle_hankata_from_kana_yields_romaji() {
        assert_eq!(cm(CONV_HANKATA).classify_idle(false, ObservedKana), Some(ObservedRomaji));
    }

    // ── classify_transition ──────────────────────────────────────────────────

    // 英数遷移
    #[test]
    fn tr_hiragana_to_eisu_always_kana() {
        assert_eq!(
            cm(CONV_EISUU).classify_transition(cm(CONV_HIRAGANA), ObservedRomaji),
            Some(ObservedKana)
        );
        // belief が Kana でも Some を返す（強制補正）
        assert_eq!(
            cm(CONV_EISUU).classify_transition(cm(CONV_JISAKANA), ObservedKana),
            Some(ObservedKana)
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

    #[test]
    fn tr_no_roman_change_yields_none() {
        // JISかな → 全角カタカナ: どちらも ROMAN=0 → 変化なし
        assert_eq!(
            cm(CONV_ZENKATA).classify_transition(cm(CONV_JISAKANA), ObservedKana),
            None
        );
    }

    #[test]
    fn tr_hiragana_to_zenkata_yields_kana() {
        // 0x19 → 0x0B: ROMAN 1→0, NATIVE=1 → Kana
        assert_eq!(
            cm(CONV_ZENKATA).classify_transition(cm(CONV_HIRAGANA), ObservedRomaji),
            Some(ObservedKana)
        );
    }

    // ── 全モード網羅 ──────────────────────────────────────────────────────────
    #[test]
    fn all_kana_modes_from_romaji_yield_kana_on_idle() {
        for conv in [CONV_EISUU, CONV_JISAKANA] {
            assert_eq!(
                cm(conv).classify_idle(false, ObservedRomaji),
                Some(ObservedKana),
                "conv=0x{conv:08X}"
            );
        }
    }

    #[test]
    fn all_romaji_modes_from_kana_yield_romaji_on_idle() {
        for conv in [CONV_HIRAGANA, CONV_ZENKATA, CONV_HANKATA] {
            assert_eq!(
                cm(conv).classify_idle(false, ObservedKana),
                Some(ObservedRomaji),
                "conv=0x{conv:08X}"
            );
        }
    }
}
