//! ADR-191 決定3: 「打鍵の時点で、表(状態, キー)→効果からbeliefを予測する」ための純粋な表。
//!
//! awaseはIMEへ書かない。生キーはそのままIMEへ通り、ここでは**その結果を先取りして**beliefへ
//! 反映するための予測だけを返す（観測は後から確認・訂正する。`ime_model.rs`の`KeyEffectPredicted`）。
//!
//! # 出所（静的な初期仮説）
//!
//! Mozc公開キーマップ（`atok.tsv` / `ms-ime.tsv`）を、`keyevent_handler.cc`のVK→`KeyEvent`
//! 対応と`key_parser.cc`のトークン別名（`kana`=`hiragana`、`hankaku`=`zenkaku`=`hankaku/zenkaku`）で
//! 引いた結果。Mozcの`status`は次のとおり（awaseが分かる範囲に丸める）:
//! IME閉 → `DirectInput` / 開・入力中でない → `Precomposition` / 開・入力中 → `Composition`か
//! `Conversion`（区別できないので、両者の効果が一致するときだけ予測する）。
//!
//! **カスタムキーマップ・overlayが対象キーの行を上書きしている場合は予測しない**（`None`、観測に任せる）。
//! 学習（ADR-191 決定4の較正）で置き換わるまでの、静的な初期仮説である。
//!
//! # 予測しないもの
//!
//! - 表に無い・非決定のセル（`Unknown`）は`None`（観測が唯一の信号になる）。
//! - `IMEOn`が復元する入力モード（直前のモードを覚えている）は予測しない（`open`だけ）。
//! - ADR-189の固定セット（`VK_KANJI`0x19・半角/全角0xF3/0xF4）: 0xF3/0xF4はここに行を持つが、
//!   ADR-189のbeliefトグル（`shadow_action`）が有効なときは呼び出し側が`shadow_action.is_some()`で
//!   除外する（二重に効かせない）。0x19は表に無い。

use awase::engine::{AssumedReason, InputModeState};

/// 予測に使うキーマップの系統。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeymapPreset {
    Atok,
    MsIme,
}

/// Mozcの`status`をawaseが分かる範囲に丸めたもの。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStatus {
    /// IMEが閉じている（Mozcの`DirectInput`）。
    Direct,
    /// IMEが開いていて入力中でない（`Precomposition`）。
    Pre,
    /// IMEが開いていて入力中（`Composition`または`Conversion`）。
    Composing,
}

/// 1打鍵の（IME側の）効果。Mozcのコマンドを、awaseのbeliefが持つ軸だけに畳んだもの。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEffect {
    /// 開閉も入力モードも変わらない（未割り当て、変換系など）。
    NoChange,
    /// IMEを開く（`IMEOn`）。入力モードは変えない（直前のモードが復元される）。
    ImeOn,
    /// IMEを閉じる（`IMEOff`/`CancelAndIMEOff`）。
    ImeOff,
    /// 半角英数⇔かなの切り替え（`ToggleAlphanumericMode`）。開閉は変えない。
    ToggleAlnum,
    /// かな系の入力モードに固定する（`CompositionModeHiragana`/`FullKatakana`）。
    SetKana,
    /// 効果が状態・文脈で決まらない（`Reconvert`/`SwitchKanaType`等）。予測しない。
    Unknown,
}

/// 表から予測した、beliefへの反映内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct PredictedEffect {
    /// 予測される開閉。`None` = 変えない。
    pub open: Option<bool>,
    /// 予測される入力モード。`None` = 変えない。
    pub mode: Option<InputModeState>,
}

impl PredictedEffect {
    /// 何も変わらない予測（beliefを書き換えない）。
    #[must_use]
    pub const fn is_noop(&self) -> bool {
        self.open.is_none() && self.mode.is_none()
    }
}

/// `(プリセット, キー, status)`ごとの効果。`vk`は`vk.rs::is_followed_mode_key`の集合。
/// 行は`atok.tsv`/`ms-ime.tsv`から生成（並びは`[Direct, Pre, Composition, Conversion]`）。
const fn lookup(preset: KeymapPreset, vk: u16) -> Option<[KeyEffect; 4]> {
    use KeyEffect::{ImeOff, ImeOn, NoChange, SetKana, ToggleAlnum, Unknown};
    let row = match (preset, vk) {
        // 0xF0 VK_DBE_ALPHANUMERIC → Eisu
        (KeymapPreset::Atok, 0xF0) => [NoChange, ToggleAlnum, ToggleAlnum, ToggleAlnum],
        (KeymapPreset::MsIme, 0xF0) => [ImeOn, ToggleAlnum, ToggleAlnum, ToggleAlnum],
        // 0xF1 VK_DBE_KATAKANA → Katakana
        (KeymapPreset::Atok, 0xF1) => [NoChange, NoChange, NoChange, NoChange],
        (KeymapPreset::MsIme, 0xF1) => [ImeOn, SetKana, SetKana, SetKana],
        // 0xF2 VK_DBE_HIRAGANA → Kana(=Hiragana)
        (KeymapPreset::Atok, 0xF2) => [NoChange, ToggleAlnum, ToggleAlnum, ToggleAlnum],
        (KeymapPreset::MsIme, 0xF2) => [ImeOn, SetKana, SetKana, SetKana],
        // 0xF3/0xF4 VK_DBE_SBCSCHAR/DBCSCHAR → Hankaku/Zenkaku
        (KeymapPreset::Atok | KeymapPreset::MsIme, 0xF3 | 0xF4) => [ImeOn, ImeOff, ImeOff, ImeOff],
        // 0x1C VK_CONVERT → Henkan
        (KeymapPreset::Atok, 0x1C) => [ImeOn, ImeOff, NoChange, NoChange],
        (KeymapPreset::MsIme, 0x1C) => [Unknown, Unknown, NoChange, NoChange],
        // 0x1D VK_NONCONVERT → Muhenkan
        (KeymapPreset::Atok, 0x1D) => [ImeOn, ImeOff, ToggleAlnum, NoChange],
        (KeymapPreset::MsIme, 0x1D) => [NoChange, Unknown, Unknown, Unknown],
        _ => return None,
    };
    Some(row)
}

/// `config1.db`から得た、予測に使うキーマップ（プリセット+カスタム上書きの検出材料）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEffectKeymap {
    preset: KeymapPreset,
    custom_table: Option<String>,
    has_overlay: bool,
}

/// Mozc `SessionKeymap`: `NONE=-1, CUSTOM=0, ATOK=1, MSIME=2`（`awase-gji-config`の定数と同じ値）。
const SESSION_KEYMAP_NONE: i64 = -1;
const SESSION_KEYMAP_ATOK: i64 = 1;
const SESSION_KEYMAP_MSIME: i64 = 2;

impl KeyEffectKeymap {
    /// `config1.db`の`session_keymap`（不在=`None`）・`custom_keymap_table`・`overlay_keymaps`から作る。
    /// プリセットがATOK/MSIME（不在/NONEはWindows版GJIの既定でMSIME相当）以外（CUSTOM・MOBILE等）は
    /// 基準の表が無いので`None`（予測しない）。
    #[must_use]
    pub fn from_config(
        session_keymap: Option<i64>,
        custom_table: Option<String>,
        overlay_keymaps: &[i64],
    ) -> Option<Self> {
        let preset = match session_keymap {
            Some(SESSION_KEYMAP_ATOK) => KeymapPreset::Atok,
            None | Some(SESSION_KEYMAP_NONE | SESSION_KEYMAP_MSIME) => KeymapPreset::MsIme,
            Some(_) => return None,
        };
        Some(Self {
            preset,
            custom_table,
            has_overlay: !overlay_keymaps.is_empty(),
        })
    }

    /// このキーマップでの、`vk`の打鍵の予測。カスタム表がそのキーの行を持つ、または overlay がある
    /// （無変換/変換は overlay `HENKAN_MUHENKAN_TO_IME_ON_OFF` が上書きしうる）ときは`None`。
    #[must_use]
    pub fn predict(
        &self,
        vk: u16,
        status: KeyStatus,
        current_mode: InputModeState,
    ) -> Option<PredictedEffect> {
        if self
            .custom_table
            .as_deref()
            .is_some_and(|t| custom_table_overrides(t, vk))
        {
            return None;
        }
        if self.has_overlay && matches!(vk, 0x1C | 0x1D) {
            return None;
        }
        predict(self.preset, vk, status, current_mode)
    }
}

/// このVKのMozcキーイベント名（カスタムキーマップに同名の行があるかの判定用）。
/// 別名を含む（`key_parser.cc`）。
const fn mozc_tokens(vk: u16) -> &'static [&'static str] {
    match vk {
        0xF0 => &["eisu"],
        0xF1 => &["katakana"],
        0xF2 => &["kana", "hiragana"],
        0xF3 | 0xF4 => &["hankaku", "zenkaku", "hankaku/zenkaku"],
        0x1C => &["henkan"],
        0x1D => &["muhenkan"],
        _ => &[],
    }
}

/// カスタムキーマップTSV（`custom_keymap_table`）が、このVKのキーイベントの行を持つか。
#[must_use]
pub fn custom_table_overrides(custom_table: &str, vk: u16) -> bool {
    let tokens = mozc_tokens(vk);
    custom_table.lines().any(|line| {
        let mut cols = line.split('\t');
        let (_status, Some(key)) = (cols.next(), cols.next()) else {
            return false;
        };
        let key = key.trim().to_ascii_lowercase();
        tokens.contains(&key.as_str())
    })
}

/// 表を引いて予測を返す。予測できない（表に無い・非決定・状態が曖昧）ときは`None`。
///
/// `current_mode`は`ToggleAlnum`の反転元。`ObservedEisu`なら「かな」へ、かな系なら「英数」へ。
/// それ以外（`Unknown`等）は反転先が決まらないので`None`。
#[must_use]
pub fn predict(
    preset: KeymapPreset,
    vk: u16,
    status: KeyStatus,
    current_mode: InputModeState,
) -> Option<PredictedEffect> {
    let row = lookup(preset, vk)?;
    let effect = match status {
        KeyStatus::Direct => row[0],
        KeyStatus::Pre => row[1],
        // Composition と Conversion は区別できない。効果が同じときだけ予測する。
        KeyStatus::Composing => {
            if row[2] == row[3] {
                row[2]
            } else {
                return None;
            }
        }
    };
    let kana = InputModeState::AssumedRomaji {
        reason: AssumedReason::KeyEffectPrediction,
    };
    let predicted = match effect {
        KeyEffect::NoChange => PredictedEffect {
            open: None,
            mode: None,
        },
        KeyEffect::ImeOn => PredictedEffect {
            open: Some(true),
            mode: None,
        },
        KeyEffect::ImeOff => PredictedEffect {
            open: Some(false),
            mode: None,
        },
        KeyEffect::SetKana => PredictedEffect {
            open: None,
            mode: Some(kana),
        },
        KeyEffect::ToggleAlnum => {
            let mode = match current_mode {
                InputModeState::ObservedEisu => kana,
                InputModeState::ObservedRomaji
                | InputModeState::ObservedKana
                | InputModeState::AssumedRomaji { .. } => InputModeState::ObservedEisu,
                InputModeState::Unknown => return None,
            };
            PredictedEffect {
                open: None,
                mode: Some(mode),
            }
        }
        KeyEffect::Unknown => return None,
    };
    Some(predicted)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROMAJI: InputModeState = InputModeState::ObservedRomaji;

    fn kana_pred() -> InputModeState {
        InputModeState::AssumedRomaji {
            reason: AssumedReason::KeyEffectPrediction,
        }
    }

    #[test]
    fn atok_hiragana_toggles_alnum_when_open() {
        // 実測: ATOK Precomposition Kana=ToggleAlphanumericMode（ひらがな→英数）。
        let p = predict(KeymapPreset::Atok, 0xF2, KeyStatus::Pre, ROMAJI).unwrap();
        assert_eq!(p.open, None);
        assert_eq!(p.mode, Some(InputModeState::ObservedEisu));
        // 英数から押すとかなへ戻る。
        let p = predict(
            KeymapPreset::Atok,
            0xF2,
            KeyStatus::Pre,
            InputModeState::ObservedEisu,
        )
        .unwrap();
        assert_eq!(p.mode, Some(kana_pred()));
    }

    #[test]
    fn atok_hiragana_in_direct_input_is_undefined_so_nothing_changes() {
        // ATOK DirectInput に Kana の行は無い（実測: IME OFF でひらがなを押しても開かない）。
        let p = predict(KeymapPreset::Atok, 0xF2, KeyStatus::Direct, ROMAJI).unwrap();
        assert!(p.is_noop());
    }

    #[test]
    fn atok_muhenkan_depends_on_status() {
        let mode = ROMAJI;
        let d = predict(KeymapPreset::Atok, 0x1D, KeyStatus::Direct, mode).unwrap();
        assert_eq!(d.open, Some(true));
        let p = predict(KeymapPreset::Atok, 0x1D, KeyStatus::Pre, mode).unwrap();
        assert_eq!(p.open, Some(false));
        // Composition は ToggleAlnum、Conversion は NoChange で食い違う → 予測しない。
        assert_eq!(
            predict(KeymapPreset::Atok, 0x1D, KeyStatus::Composing, mode),
            None
        );
    }

    #[test]
    fn atok_henkan_while_composing_is_no_change() {
        // Composition=Convert / Conversion=ConvertNextPage は、どちらも開閉・モードを変えない。
        let p = predict(KeymapPreset::Atok, 0x1C, KeyStatus::Composing, ROMAJI).unwrap();
        assert!(p.is_noop());
    }

    #[test]
    fn hankaku_zenkaku_opens_when_closed_and_closes_otherwise() {
        for vk in [0xF3, 0xF4] {
            for preset in [KeymapPreset::Atok, KeymapPreset::MsIme] {
                let d = predict(preset, vk, KeyStatus::Direct, ROMAJI).unwrap();
                assert_eq!(d.open, Some(true));
                let c = predict(preset, vk, KeyStatus::Composing, ROMAJI).unwrap();
                assert_eq!(c.open, Some(false));
            }
        }
    }

    #[test]
    fn msime_hiragana_sets_kana_and_opens_from_direct() {
        let p = predict(
            KeymapPreset::MsIme,
            0xF2,
            KeyStatus::Pre,
            InputModeState::ObservedEisu,
        )
        .unwrap();
        assert_eq!(p.mode, Some(kana_pred()));
        let d = predict(KeymapPreset::MsIme, 0xF2, KeyStatus::Direct, ROMAJI).unwrap();
        assert_eq!(d.open, Some(true));
    }

    #[test]
    fn non_deterministic_cells_return_none() {
        // MS-IME の Henkan は Precomposition で Reconvert（効果が文脈依存）。
        assert_eq!(
            predict(KeymapPreset::MsIme, 0x1C, KeyStatus::Pre, ROMAJI),
            None
        );
        // 入力モードが不明なときのトグルは反転先が決まらない。
        assert_eq!(
            predict(
                KeymapPreset::Atok,
                0xF2,
                KeyStatus::Pre,
                InputModeState::Unknown
            ),
            None
        );
        // 表に無いキー（VK_KANJI 0x19 = ADR-189 の固定セット、VK_IME_ON 等）。
        assert_eq!(
            predict(KeymapPreset::Atok, 0x19, KeyStatus::Pre, ROMAJI),
            None
        );
    }

    #[test]
    fn keymap_from_config_selects_preset_and_respects_overrides() {
        let atok = KeyEffectKeymap::from_config(Some(1), None, &[]).unwrap();
        assert!(atok.predict(0xF2, KeyStatus::Pre, ROMAJI).is_some());
        // 不在/NONE は MSIME 相当。
        let ms = KeyEffectKeymap::from_config(None, None, &[]).unwrap();
        assert_eq!(
            ms.predict(0xF2, KeyStatus::Direct, ROMAJI).unwrap().open,
            Some(true)
        );
        // CUSTOM・MOBILE 等は基準の表が無い。
        assert!(KeyEffectKeymap::from_config(Some(0), None, &[]).is_none());
        assert!(KeyEffectKeymap::from_config(Some(4), None, &[]).is_none());
        // ATOK + カスタム表が無変換の行を持つ → 無変換だけ予測しない（実機の構成: ATOK + custom、Kanaの行なし）。
        let table = "Precomposition\tMuhenkan\tIMEOn\n".to_string();
        let custom = KeyEffectKeymap::from_config(Some(1), Some(table), &[]).unwrap();
        assert_eq!(custom.predict(0x1D, KeyStatus::Pre, ROMAJI), None);
        assert!(custom.predict(0xF2, KeyStatus::Pre, ROMAJI).is_some());
        // overlay があると 無変換/変換 だけ予測しない。
        let ov = KeyEffectKeymap::from_config(Some(1), None, &[100]).unwrap();
        assert_eq!(ov.predict(0x1C, KeyStatus::Pre, ROMAJI), None);
        assert!(ov.predict(0xF2, KeyStatus::Pre, ROMAJI).is_some());
    }

    #[test]
    fn custom_table_rows_for_the_key_disable_prediction() {
        let table =
            "status\tkey\tcommand\nDirectInput\tF15\tIMEOn\nPrecomposition\tMuhenkan\tIMEOff\n";
        assert!(custom_table_overrides(table, 0x1D));
        assert!(!custom_table_overrides(table, 0x1C));
        // 別名（Hiragana と Kana は同じキーイベント）。
        let alias = "Composition\tHiragana\tCancel\n";
        assert!(custom_table_overrides(alias, 0xF2));
        assert!(!custom_table_overrides(alias, 0xF0));
    }
}
