//! ADR-195 段階0: 設定の読み取りラウンド（既存3経路の統合）。
//!
//! develop に既にある3つの部分的なキーマップ読み取り経路——経路1
//! （[`crate::state::key_effect_predictor::KeyEffectKeymap::from_config`]のプリセット選択）・
//! 経路2（[`awase_gji_config::keymap::extract_ime_keys`]/
//! [`awase_gji_config::keymap::extract_mode_keys`]、`custom_keymap_table`実測）・
//! 経路3（[`crate::msime_key_assignment`]、MS-IME本体のレジストリ再割り当て検出）——の
//! 出力を、キー単位の1つの構造体（初期仮説表S）にまとめる。新規の検出ロジックは追加しない
//! （既存の抽出結果を集約するだけの薄い層）。
//!
//! # 暫定実装（ADR-195/ADR-192の所有権分担）
//!
//! 「キーマップから状態依存キーを機械的に検出する」ロジック自体は本来ADR-192決定1が
//! 所有する予定だが、develop未実装（2026-09-23時点、grep確認済み）のため、ここでは
//! `extract_mode_keys`を直接呼ぶ暫定実装とする（ADR-195本文が明記する方針）。ADR-192
//! 実装後、その出力（`state_dependent_key_warning.rs`等が公開する判定結果）へ差し替える。

use std::collections::{BTreeMap, BTreeSet};

use awase_gji_config::command::GjiCompositionMode;
use awase_gji_config::keymap::{
    extract_ime_keys, extract_mode_keys, set_mode_keys_confirmed_by_input_progress_status,
};

use super::key_effect_predictor::KeymapPreset;

/// 初期仮説表Sの1キー分。段階1（学習）を省略できる、コマンド名だけから直接確定する予測。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyHypothesis {
    /// このキー単独でIMEをONにする。
    ImeOn,
    /// このキー単独でIMEをOFFにする。
    ImeOff,
    /// このキーでIMEのON/OFFがトグルする。
    ImeToggle,
    /// 絶対設定系コマンド（`SetMode`）が入力中（Composition/Conversion）のstatusに
    /// 束縛されており、遷移先モードが確定している。
    SetMode(GjiCompositionMode),
}

/// ADR-195段階0の出力: キー単位の初期仮説表S。
///
/// `known`は初期仮説が判明しているキー（VK名→仮説、学習不要）。`needs_learning`は
/// 「不明、要学習」のまま残ったキー（VK名。段階1=ADR195-T1が測定すべき対象）。両者は
/// 互いに素（同じキーが両方に入ることはない）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InitialHypothesisTable {
    known: BTreeMap<String, KeyHypothesis>,
    needs_learning: BTreeSet<String>,
}

impl InitialHypothesisTable {
    /// 初期仮説が判明しているキーの一覧（VK名→仮説）。
    #[must_use]
    pub fn known(&self) -> &BTreeMap<String, KeyHypothesis> {
        &self.known
    }

    /// 段階1が測定すべき、初期仮説が不明のまま残ったキー（VK名）の一覧。
    #[must_use]
    pub fn needs_learning(&self) -> &BTreeSet<String> {
        &self.needs_learning
    }

    /// `known`・`needs_learning`のどちらにもキーが無い（=何も分類できなかった）か。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.known.is_empty() && self.needs_learning.is_empty()
    }

    fn insert_known(&mut self, vk_name: String, hypothesis: KeyHypothesis) {
        self.needs_learning.remove(&vk_name);
        self.known.insert(vk_name, hypothesis);
    }

    fn insert_needs_learning(&mut self, vk_name: String) {
        if !self.known.contains_key(&vk_name) {
            self.needs_learning.insert(vk_name);
        }
    }
}

/// 経路1〜2（GJI）を統合し、初期仮説表Sを構築する。
///
/// `preset`は経路1（[`KeyEffectKeymap::from_config`](crate::state::key_effect_predictor::KeyEffectKeymap::from_config)）
/// が選んだ内蔵プリセットの有無——情報として受け取るのみで、`None`（CUSTOM/MOBILE等、
/// 内蔵表なし）でも`custom_keymap_table`の実測（経路2）は独立に行える。`custom_table`が
/// `None`（GJI未導入、または`custom_keymap_table`が空）ならSは空。
#[must_use]
pub fn build_gji_initial_hypothesis(
    preset: Option<KeymapPreset>,
    custom_table: Option<&str>,
) -> InitialHypothesisTable {
    let _ = preset;
    let mut table = InitialHypothesisTable::default();
    let Some(custom_table) = custom_table else {
        return table;
    };

    let ime_keys = extract_ime_keys(custom_table);
    for vk_name in ime_keys.on {
        table.insert_known(vk_name, KeyHypothesis::ImeOn);
    }
    for vk_name in ime_keys.off {
        table.insert_known(vk_name, KeyHypothesis::ImeOff);
    }
    for vk_name in ime_keys.toggle {
        table.insert_known(vk_name, KeyHypothesis::ImeToggle);
    }

    let mode_keys = extract_mode_keys(custom_table);
    let confirmed = set_mode_keys_confirmed_by_input_progress_status(custom_table);
    for (vk_name, mode) in mode_keys.set_mode {
        if confirmed.contains(&vk_name) {
            table.insert_known(vk_name, KeyHypothesis::SetMode(mode));
        } else {
            table.insert_needs_learning(vk_name);
        }
    }
    for vk_name in mode_keys
        .toggle_alphanumeric
        .into_iter()
        .chain(mode_keys.toggle_kana_type)
    {
        table.insert_needs_learning(vk_name);
    }

    table
}

/// 経路3（[`crate::msime_key_assignment`]）: Microsoft IME本体の初期仮説Sは常に空。
///
/// Microsoft IME本体には、経路2（`custom_keymap_table`のキー単位抽出）に相当する
/// 情報源が無い。レジストリはキー再割り当ての「有無」（`IsKeyAssignmentEnabled`/
/// `KeyAssignmentHenkan`/`KeyAssignmentMuhenkan`）だけを示し、割り当て後の効果
/// （何が起きるか）までは読み取れないため、初期仮説Sは常に空（=全キー要学習）になる
/// （ADR-195 段階0 決定4）。レジストリの読み取り結果に依らず常に空なので、
/// この関数は引数を取らない。
#[must_use]
pub fn build_msime_native_initial_hypothesis() -> InitialHypothesisTable {
    InitialHypothesisTable::default()
}

#[cfg(test)]
mod tests {
    use super::{
        build_gji_initial_hypothesis, build_msime_native_initial_hypothesis, KeyHypothesis,
    };
    use awase_gji_config::command::GjiCompositionMode;

    #[test]
    fn none_custom_table_yields_empty_hypothesis() {
        let table = build_gji_initial_hypothesis(None, None);
        assert!(table.is_empty());
    }

    #[test]
    fn ime_on_off_toggle_keys_become_known() {
        let text = "status\tkey\tcommand
DirectInput\tF21\tIMEOn
Precomposition\tF22\tIMEOff
Composition\tF22\tIMEOff
Conversion\tF22\tIMEOff
DirectInput\tHenkan\tIMEOn
Precomposition\tHenkan\tCancelAndIMEOff
";
        let table = build_gji_initial_hypothesis(None, Some(text));
        assert_eq!(table.known().get("VK_F21"), Some(&KeyHypothesis::ImeOn));
        assert_eq!(table.known().get("VK_F22"), Some(&KeyHypothesis::ImeOff));
        assert_eq!(
            table.known().get("VK_CONVERT"),
            Some(&KeyHypothesis::ImeToggle)
        );
        assert!(table.needs_learning().is_empty());
    }

    /// ADR-195 段階0 決定3: `SetMode`がComposition/Conversionのstatusに束縛されて
    /// いれば学習不要（`known`）、DirectInput/Precompositionのみなら「不明、要学習」
    /// （`needs_learning`）のまま。相対トグル系は常に`needs_learning`。
    #[test]
    fn set_mode_confirmed_vs_unconfirmed_and_relative_toggles_need_learning() {
        let text = "status\tkey\tcommand
DirectInput\tF6\tCompositionModeHiragana
Precomposition\tF6\tCompositionModeHiragana
Composition\tF6\tCompositionModeHiragana
DirectInput\tF9\tCompositionModeFullKatakana
Precomposition\tF9\tCompositionModeFullKatakana
Composition\tEisu\tToggleAlphanumericMode
Conversion\tEisu\tToggleAlphanumericMode
Precomposition\tF8\tSwitchKanaType
";
        let table = build_gji_initial_hypothesis(None, Some(text));
        assert_eq!(
            table.known().get("VK_F6"),
            Some(&KeyHypothesis::SetMode(GjiCompositionMode::Hiragana))
        );
        assert!(table.needs_learning().contains("VK_F9"));
        assert!(table.needs_learning().contains("VK_DBE_ALPHANUMERIC"));
        assert!(table.needs_learning().contains("VK_F8"));
        assert!(!table.known().contains_key("VK_F9"));
    }

    /// ADR-195 段階0 決定4: Microsoft IME本体は経路2に相当する抽出元が無いため、
    /// 初期仮説Sは常に空（全キー要学習）。
    #[test]
    fn msime_native_initial_hypothesis_is_always_empty() {
        let table = build_msime_native_initial_hypothesis();
        assert!(table.known().is_empty());
        assert!(table.needs_learning().is_empty());
        assert!(table.is_empty());
    }
}
