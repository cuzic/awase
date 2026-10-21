//! ADR-223: 入力言語(HKL)の判定。純粋関数のみ(Win32 API は呼ばない)。
//!
//! 読み取り(`GetKeyboardLayout`)は `runtime/lang_check.rs` が行い、ここでは値の解釈だけを持つ。
//! `crate::imm`(Windows 限定)には依存せず、Linux のテストでも検証できるようにしてある。

use crate::vk::LANGID_JAPANESE;

/// HKL の下位 16 ビット(`LANGID`)。
#[must_use]
pub const fn lang_id(hkl: u32) -> u32 {
    hkl & 0xFFFF
}

/// HKL から「日本語レイアウトか」を返す。`hkl == 0`(スレッド終了・不明)は `None`(書き込まない)。
///
/// 旧 IMM 形式の IME の HKL(`0xE0010411`)も、TSF の MS-IME の HKL(`0x04110411`)も、下位 16 ビットの
/// `LANGID` で比べる(ADR-223 の事実欄)。
#[must_use]
pub const fn classify_layout_language(hkl: u32) -> Option<bool> {
    if hkl == 0 {
        return None;
    }
    Some(lang_id(hkl) == LANGID_JAPANESE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_hkl_is_unknown() {
        assert_eq!(classify_layout_language(0), None);
    }

    #[test]
    fn japanese_hkl_in_both_forms() {
        // TSF の MS-IME(GetKeyboardLayout が返す形)と旧 IMM 形式の IME の HKL
        assert_eq!(classify_layout_language(0x0411_0411), Some(true));
        assert_eq!(classify_layout_language(0xE001_0411), Some(true));
    }

    #[test]
    fn non_japanese_hkl() {
        assert_eq!(classify_layout_language(0x0409_0409), Some(false)); // en-US
        assert_eq!(classify_layout_language(0x0419_0419), Some(false)); // ru-RU
    }

    #[test]
    fn lang_id_is_low_16_bits() {
        assert_eq!(lang_id(0xE001_0411), 0x0411);
        assert_eq!(lang_id(0x0419_0419), 0x0419);
    }
}
