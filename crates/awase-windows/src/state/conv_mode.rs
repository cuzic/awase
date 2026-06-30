//! IME 変換モードの内部表現と管理コンポーネント。
//!
//! `ImmGetConversionStatus` の生 conv 値を、awase が扱いやすい型安全な形に変換する。
//! `kp_stage_idle_conv_check` が更新し、warmup コードが参照する。

// ─── 文字種 ──────────────────────────────────────────────────────────────────

/// IME が出力する文字の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Charset {
    /// ひらがな（NATIVE|FULLSHAPE）
    Hiragana,
    /// 全角カタカナ（NATIVE|KATAKANA|FULLSHAPE）
    ZenkakuKatakana,
    /// 半角カタカナ（NATIVE|KATAKANA）
    HankakuKatakana,
    /// 全角英数（FULLSHAPE のみ）
    ZenkakuAlpha,
    /// 半角英数（フラグなし）
    HankakuAlpha,
}

impl Charset {
    pub(crate) fn is_katakana(self) -> bool {
        matches!(self, Self::ZenkakuKatakana | Self::HankakuKatakana)
    }
}

// ─── 変換モード ───────────────────────────────────────────────────────────────

/// IME 変換モードを文字種と入力方式の2軸で表す。
///
/// `romaji` フィールドは Hiragana/Katakana 系のみ意味を持つ。
/// - `true` = ローマ字入力（ROMAN ビットあり）
/// - `false` = JISかな直接入力（ROMAN ビットなし）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConvMode {
    pub(crate) charset: Charset,
    pub(crate) romaji: bool,
}

impl ConvMode {
    /// `ImmGetConversionStatus` の raw conv 値から生成する。
    pub(crate) fn from_conv(conv: u32) -> Self {
        use crate::imm::{IME_CMODE_FULLSHAPE, IME_CMODE_KATAKANA, IME_CMODE_NATIVE, IME_CMODE_ROMAN};
        let has_native = conv & IME_CMODE_NATIVE != 0;
        let has_katakana = conv & IME_CMODE_KATAKANA != 0;
        let has_fullshape = conv & IME_CMODE_FULLSHAPE != 0;
        let has_roman = conv & IME_CMODE_ROMAN != 0;

        let charset = if !has_native {
            if has_fullshape {
                Charset::ZenkakuAlpha
            } else {
                Charset::HankakuAlpha
            }
        } else if has_katakana {
            if has_fullshape {
                Charset::ZenkakuKatakana
            } else {
                Charset::HankakuKatakana
            }
        } else {
            Charset::Hiragana
        };

        ConvMode { charset, romaji: has_roman }
    }

    /// `ImmSetConversionStatus` の目標 conv 値（ROMAN ビット込み）を返す。
    ///
    /// カタカナ系は KATAKANA/FULLSHAPE ビットを明示的に指定して復元する必要があるため
    /// `Some(conv)` を返す。それ以外は `current_conv | ROMAN` で十分なため `None`。
    pub(crate) fn imm_conv_target(self) -> Option<u32> {
        use crate::imm::{IME_CMODE_FULLSHAPE, IME_CMODE_KATAKANA, IME_CMODE_NATIVE, IME_CMODE_ROMAN};
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
}

// ─── 管理コンポーネント ────────────────────────────────────────────────────────

/// IME 変換モードを一元管理するコンポーネント。
///
/// `kp_stage_idle_conv_check` が `update_from_conv` でモードを更新し、
/// warmup コードが `get` でモードを参照して先頭 VK と ImmSetConversionStatus 目標値を決定する。
#[derive(Debug, Default)]
pub(crate) struct ConvModeMgr {
    mode: std::cell::Cell<Option<ConvMode>>,
}

impl ConvModeMgr {
    /// `ImmGetConversionStatus` の raw conv 値からモードを更新する。
    ///
    /// 変化があった場合のみ `info` ログを出力する。
    pub(crate) fn update_from_conv(&self, conv: u32) {
        let new = ConvMode::from_conv(conv);
        let old = self.mode.get();
        if old != Some(new) {
            log::info!(
                "[conv-mode] {:?} → {:?} (conv=0x{conv:08X})",
                old.as_ref().map(|m| format!("{:?}/{}", m.charset, if m.romaji { "roma" } else { "kana" })),
                format!("{:?}/{}", new.charset, if new.romaji { "roma" } else { "kana" }),
            );
            self.mode.set(Some(new));
        }
    }

    /// 現在のモードを返す。`None` = まだ `update_from_conv` が呼ばれていない。
    pub(crate) fn get(&self) -> Option<ConvMode> {
        self.mode.get()
    }
}
