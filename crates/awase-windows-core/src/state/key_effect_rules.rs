//! 学習表(セルの集合)から、キーごとの宣言的なルールを導く純関数。
//!
//! 学習(ADR-191/195)が作るのは「状態×キー→結果」の表で、その表が実際にどんなルールを
//! 表すかは、これまで個別の判定コードの中にだけあった
//! (`key_effect_table::classify_cells` は全セルを横断した開閉の分類、
//! `toggle_contradiction` は ADR-199 決定6-2 の「トグルと矛盾するか」だけ)。
//! ここではそれを、開閉・変換モード・入力中の行方の3軸の**ルール**として一箇所で導く。
//! 導いたルールは、IME ごとの oracle(期待する挙動の仕様)として、`ImeModel` や予測器との
//! 突き合わせテストに使える。実行時の判断にはまだ配線していない(読み取り専用の導出)。
//!
//! 方針: **証拠が片側だけなら主張しない**。開閉の4ルール(`SetOn`/`SetOff`/`Toggle`/`Identity`)は、
//! 閉と開の両方の未入力セルがあるときだけ名乗る。片側だけだと複数のルールが同時に当てはまる
//! (閉で開く1セルだけなら `SetOn` も `Toggle` も真)ので、`Insufficient` にする。

use super::key_effect_predictor::{Cell, Conv, Disp, Stage, TableKey};

/// 未入力(`Stage::None`)のときの開閉のルール。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenRule {
    /// どの状態から押しても開になる(ON 専用キー)。
    SetOn,
    /// どの状態から押しても閉になる(OFF 専用キー)。
    SetOff,
    /// どの状態から押しても開閉が反転する。
    Toggle,
    /// 開閉を変えない。
    Identity,
    /// 上のどれでもない(状態によって結果が違う)。
    StateDependent,
    /// このキーの未入力セルが無い。
    Unmeasured,
    /// 閉・開の片方しか測っておらず、ルールを名乗れない。
    Insufficient,
}

/// 開のまま変わらない押下(開→開)での変換モードのルール。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeRule {
    /// 開→開のセルが無い、またはどれも押下後の変換モードが不明。
    Unknown,
    /// 押下前の変換モードを保つ(2種類以上の変換モードから測って全て保った)。
    Preserves,
    /// どの変換モードから押しても、その変換モードになる。
    Sets(Conv),
    /// 変換モードによって違う、または一部が不明。
    Varies,
}

/// 入力中(`Stage::None` 以外)の行方のルール。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositionRule {
    /// 入力中・変換中のセルが無い。
    Unmeasured,
    /// 測った全ての入力中の状態で、行方が同じ。
    Same(Disp),
    /// 状態によって行方が違う。
    Varies,
}

/// 1キー分の導出結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyRules {
    pub key: TableKey,
    pub open_when_idle: OpenRule,
    pub mode_when_staying_open: ModeRule,
    pub composition: CompositionRule,
}

/// 表が持つキーの全種類。`TableKey` に variant が増えたら [`all_keys_is_exhaustive`] が
/// コンパイルエラーで知らせる。
pub const ALL_KEYS: [TableKey; 13] = [
    TableKey::Bs,
    TableKey::Eisu,
    TableKey::Enter,
    TableKey::Esc,
    TableKey::HankakuZenkaku,
    TableKey::Henkan,
    TableKey::Hiragana,
    TableKey::ImeOff,
    TableKey::ImeOn,
    TableKey::Kanji,
    TableKey::Katakana,
    TableKey::Muhenkan,
    TableKey::Space,
];

#[cfg(test)]
const fn all_keys_is_exhaustive(k: TableKey) {
    // variant を足したらここがコンパイルエラーになる。`ALL_KEYS` にも足すこと。
    match k {
        TableKey::Bs
        | TableKey::Eisu
        | TableKey::Enter
        | TableKey::Esc
        | TableKey::HankakuZenkaku
        | TableKey::Henkan
        | TableKey::Hiragana
        | TableKey::ImeOff
        | TableKey::ImeOn
        | TableKey::Kanji
        | TableKey::Katakana
        | TableKey::Muhenkan
        | TableKey::Space => {}
    }
}

/// `key` の未入力セルから開閉のルールを導く。
fn deduce_open(cells: &[Cell], key: TableKey) -> OpenRule {
    let idle: Vec<&Cell> = cells
        .iter()
        .filter(|c| c.key() == key && c.stage() == Stage::None)
        .collect();
    if idle.is_empty() {
        return OpenRule::Unmeasured;
    }
    let has_closed = idle.iter().any(|c| !c.open());
    let has_open = idle.iter().any(|c| c.open());
    if !(has_closed && has_open) {
        return OpenRule::Insufficient;
    }
    if idle.iter().all(|c| c.after_open()) {
        OpenRule::SetOn
    } else if idle.iter().all(|c| !c.after_open()) {
        OpenRule::SetOff
    } else if idle.iter().all(|c| c.after_open() != c.open()) {
        OpenRule::Toggle
    } else if idle.iter().all(|c| c.after_open() == c.open()) {
        OpenRule::Identity
    } else {
        OpenRule::StateDependent
    }
}

/// `key` の未入力の開→開セルから、変換モードのルールを導く。
fn deduce_mode(cells: &[Cell], key: TableKey) -> ModeRule {
    let stays: Vec<(Conv, Option<Conv>)> = cells
        .iter()
        .filter(|c| c.key() == key && c.stage() == Stage::None && c.open() && c.after_open())
        .filter_map(|c| c.conv().map(|from| (from, c.after_conv())))
        .collect();
    if stays.is_empty() || stays.iter().all(|(_, to)| to.is_none()) {
        return ModeRule::Unknown;
    }
    let Some(targets) = stays.iter().map(|(_, to)| *to).collect::<Option<Vec<_>>>() else {
        // 一部だけ押下後の変換モードが不明。
        return ModeRule::Varies;
    };
    let mut froms: Vec<Conv> = stays.iter().map(|(from, _)| *from).collect();
    froms.dedup();
    let preserves_all = stays.iter().all(|(from, to)| Some(*from) == *to);
    if preserves_all && froms.len() >= 2 {
        return ModeRule::Preserves;
    }
    if targets.iter().all(|t| *t == targets[0]) {
        return ModeRule::Sets(targets[0]);
    }
    ModeRule::Varies
}

/// `key` の入力中・変換中の(開状態の)セルから、行方のルールを導く。
fn deduce_composition(cells: &[Cell], key: TableKey) -> CompositionRule {
    let mut disps = cells
        .iter()
        .filter(|c| c.key() == key && c.stage() != Stage::None)
        .map(Cell::disp);
    let Some(first) = disps.next() else {
        return CompositionRule::Unmeasured;
    };
    if disps.all(|d| d == first) {
        CompositionRule::Same(first)
    } else {
        CompositionRule::Varies
    }
}

/// 1キーのルールを導く。
#[must_use]
pub fn deduce_key_rules(cells: &[Cell], key: TableKey) -> KeyRules {
    KeyRules {
        key,
        open_when_idle: deduce_open(cells, key),
        mode_when_staying_open: deduce_mode(cells, key),
        composition: deduce_composition(cells, key),
    }
}

/// 全キーのルールを [`ALL_KEYS`] の順に導く。
#[must_use]
pub fn deduce_all(cells: &[Cell]) -> Vec<KeyRules> {
    ALL_KEYS
        .iter()
        .map(|k| deduce_key_rules(cells, *k))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::key_effect_predictor::{bundled_table, cell, KeymapPreset, TableKey as K};
    use super::super::key_effect_table::toggle_contradiction;
    use super::*;

    const HZ: TableKey = TableKey::HankakuZenkaku;

    fn idle(
        open: bool,
        conv: Option<Conv>,
        key: TableKey,
        after: bool,
        after_conv: Option<Conv>,
    ) -> Cell {
        cell(open, conv, Stage::None, key, after, after_conv, Disp::None)
    }

    #[test]
    fn closed_only_evidence_claims_no_rule() {
        let cells = [idle(false, None, HZ, true, None)];
        assert_eq!(deduce_open(&cells, HZ), OpenRule::Insufficient);
    }

    #[test]
    fn no_idle_cells_is_unmeasured() {
        assert_eq!(deduce_open(&[], HZ), OpenRule::Unmeasured);
    }

    #[test]
    fn four_open_rules_from_both_sides() {
        let c = Some(Conv::C19);
        let mk = |closed_after, open_after| {
            [
                idle(false, None, HZ, closed_after, None),
                idle(true, c, HZ, open_after, c),
            ]
        };
        assert_eq!(deduce_open(&mk(true, false), HZ), OpenRule::Toggle);
        assert_eq!(deduce_open(&mk(true, true), HZ), OpenRule::SetOn);
        assert_eq!(deduce_open(&mk(false, false), HZ), OpenRule::SetOff);
        assert_eq!(deduce_open(&mk(false, true), HZ), OpenRule::Identity);
    }

    #[test]
    fn open_state_dependence_is_reported() {
        // 閉→開、C19 で開→閉、C10 で開→開 は、トグルでも固定でもない。
        let cells = [
            idle(false, None, HZ, true, None),
            idle(true, Some(Conv::C19), HZ, false, None),
            idle(true, Some(Conv::C10), HZ, true, Some(Conv::C10)),
        ];
        assert_eq!(deduce_open(&cells, HZ), OpenRule::StateDependent);
    }

    #[test]
    fn composition_rule_needs_agreement() {
        let typing = |disp| {
            cell(
                true,
                Some(Conv::C19),
                Stage::Typing,
                K::Esc,
                true,
                Some(Conv::C19),
                disp,
            )
        };
        assert_eq!(deduce_composition(&[], K::Esc), CompositionRule::Unmeasured);
        assert_eq!(
            deduce_composition(&[typing(Disp::Discarded)], K::Esc),
            CompositionRule::Same(Disp::Discarded)
        );
        assert_eq!(
            deduce_composition(&[typing(Disp::Discarded), typing(Disp::Kept)], K::Esc),
            CompositionRule::Varies
        );
    }

    #[test]
    fn mode_rule_distinguishes_set_from_preserve() {
        let (a, b) = (Some(Conv::C19), Some(Conv::C10));
        // どの変換モードからでも C19 になる。
        let sets = [
            idle(true, a, K::Hiragana, true, a),
            idle(true, b, K::Hiragana, true, a),
        ];
        assert_eq!(deduce_mode(&sets, K::Hiragana), ModeRule::Sets(Conv::C19));
        // 2種類の変換モードから測って、どちらも保った。
        let keeps = [
            idle(true, a, K::Space, true, a),
            idle(true, b, K::Space, true, b),
        ];
        assert_eq!(deduce_mode(&keeps, K::Space), ModeRule::Preserves);
        // 一部が不明。
        let part = [
            idle(true, a, K::Eisu, true, a),
            idle(true, b, K::Eisu, true, None),
        ];
        assert_eq!(deduce_mode(&part, K::Eisu), ModeRule::Varies);
        assert_eq!(deduce_mode(&[], K::Eisu), ModeRule::Unknown);
    }

    /// 既存の `toggle_contradiction`(ADR-199 決定6-2)との差分テスト。両側を測ったキーでは
    /// 「`Toggle` と導かれる ⇔ トグルと矛盾しない」が、同梱の全表・全キーで成り立つ。
    /// 食い違えば、どちらかの判定が間違っている。
    #[test]
    fn open_rule_agrees_with_toggle_contradiction_on_bundled_tables() {
        for preset in [
            KeymapPreset::Atok,
            KeymapPreset::MsIme,
            KeymapPreset::MsImeNative,
        ] {
            let cells = bundled_table(preset);
            for key in ALL_KEYS {
                let rule = deduce_open(cells, key);
                let contradiction = toggle_contradiction(cells, key);
                match rule {
                    OpenRule::Unmeasured | OpenRule::Insufficient => {}
                    OpenRule::Toggle => assert_eq!(
                        contradiction, None,
                        "{preset:?}/{key:?}: Toggle と導いたが矛盾が報告された"
                    ),
                    other => assert!(
                        contradiction.is_some(),
                        "{preset:?}/{key:?}: {other:?} と導いたが、矛盾が報告されない"
                    ),
                }
            }
        }
    }

    #[test]
    fn deduce_all_covers_every_key_in_order() {
        let all = deduce_all(bundled_table(KeymapPreset::Atok));
        assert_eq!(all.len(), ALL_KEYS.len());
        for (r, k) in all.iter().zip(ALL_KEYS) {
            assert_eq!(r.key, k);
        }
        all_keys_is_exhaustive(TableKey::Bs);
    }
}
