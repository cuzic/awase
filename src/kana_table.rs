use rustc_hash::{FxBuildHasher, FxHashMap};

/// ローマ字↔かな双方向ルックアップテーブル。
///
/// `build()` で両方向を一括構築し、`kana_for_romaji` / `romaji_for_kana` で参照する。
#[derive(Debug)]
pub struct KanaTable {
    romaji_to_kana: FxHashMap<String, char>,
    kana_to_romaji: FxHashMap<char, String>,
}

impl KanaTable {
    /// 両方向テーブルを構築する。
    #[must_use]
    pub fn build() -> Self {
        let romaji_to_kana = build_romaji_map();
        let kana_to_romaji = build_reverse_map(&romaji_to_kana);
        Self {
            romaji_to_kana,
            kana_to_romaji,
        }
    }

    /// ローマ字に対応するかな文字を返す。
    #[must_use]
    pub fn kana_for_romaji(&self, romaji: &str) -> Option<char> {
        self.romaji_to_kana.get(romaji).copied()
    }

    /// かな文字に対応するローマ字を返す。
    pub fn romaji_for_kana(&self, kana: char) -> Option<&str> {
        self.kana_to_romaji.get(&kana).map(String::as_str)
    }
}

#[expect(clippy::too_many_lines)]
fn build_romaji_map() -> FxHashMap<String, char> {
    let entries: &[(&str, char)] = &[
        // 母音
        ("a", 'あ'),
        ("i", 'い'),
        ("u", 'う'),
        ("e", 'え'),
        ("o", 'お'),
        // カ行
        ("ka", 'か'),
        ("ki", 'き'),
        ("ku", 'く'),
        ("ke", 'け'),
        ("ko", 'こ'),
        // サ行
        ("sa", 'さ'),
        ("si", 'し'),
        ("su", 'す'),
        ("se", 'せ'),
        ("so", 'そ'),
        // タ行
        ("ta", 'た'),
        ("ti", 'ち'),
        ("tu", 'つ'),
        ("te", 'て'),
        ("to", 'と'),
        // ナ行
        ("na", 'な'),
        ("ni", 'に'),
        ("nu", 'ぬ'),
        ("ne", 'ね'),
        ("no", 'の'),
        // ハ行
        ("ha", 'は'),
        ("hi", 'ひ'),
        ("hu", 'ふ'),
        ("fu", 'ふ'),
        ("he", 'へ'),
        ("ho", 'ほ'),
        // マ行
        ("ma", 'ま'),
        ("mi", 'み'),
        ("mu", 'む'),
        ("me", 'め'),
        ("mo", 'も'),
        // ヤ行
        ("ya", 'や'),
        ("yu", 'ゆ'),
        ("yo", 'よ'),
        // ラ行
        ("ra", 'ら'),
        ("ri", 'り'),
        ("ru", 'る'),
        ("re", 'れ'),
        ("ro", 'ろ'),
        // ワ行
        ("wa", 'わ'),
        ("wo", 'を'),
        // ン
        ("nn", 'ん'),
        // 濁音
        ("ga", 'が'),
        ("gi", 'ぎ'),
        ("gu", 'ぐ'),
        ("ge", 'げ'),
        ("go", 'ご'),
        ("za", 'ざ'),
        ("zi", 'じ'),
        ("zu", 'ず'),
        ("ze", 'ぜ'),
        ("zo", 'ぞ'),
        ("da", 'だ'),
        ("di", 'ぢ'),
        ("du", 'づ'),
        ("de", 'で'),
        ("do", 'ど'),
        ("ba", 'ば'),
        ("bi", 'び'),
        ("bu", 'ぶ'),
        ("be", 'べ'),
        ("bo", 'ぼ'),
        // 半濁音
        ("pa", 'ぱ'),
        ("pi", 'ぴ'),
        ("pu", 'ぷ'),
        ("pe", 'ぺ'),
        ("po", 'ぽ'),
        // 拗音（先頭文字で代表）
        ("kya", 'き'),
        ("kyu", 'き'),
        ("kyo", 'き'),
        ("sya", 'し'),
        ("syu", 'し'),
        ("syo", 'し'),
        ("tya", 'ち'),
        ("tyu", 'ち'),
        ("tyo", 'ち'),
        ("nya", 'に'),
        ("nyu", 'に'),
        ("nyo", 'に'),
        ("hya", 'ひ'),
        ("hyu", 'ひ'),
        ("hyo", 'ひ'),
        ("mya", 'み'),
        ("myu", 'み'),
        ("myo", 'み'),
        ("rya", 'り'),
        ("ryu", 'り'),
        ("ryo", 'り'),
        ("gya", 'ぎ'),
        ("gyu", 'ぎ'),
        ("gyo", 'ぎ'),
        ("zya", 'じ'),
        ("zyu", 'じ'),
        ("zyo", 'じ'),
        ("dya", 'ぢ'),
        ("dyu", 'ぢ'),
        ("dyo", 'ぢ'),
        ("bya", 'び'),
        ("byu", 'び'),
        ("byo", 'び'),
        ("pya", 'ぴ'),
        ("pyu", 'ぴ'),
        ("pyo", 'ぴ'),
        // 小書き
        ("la", 'ぁ'),
        ("li", 'ぃ'),
        ("lu", 'ぅ'),
        ("le", 'ぇ'),
        ("lo", 'ぉ'),
        ("lya", 'ゃ'),
        ("lyu", 'ゅ'),
        ("lyo", 'ょ'),
        ("ltu", 'っ'),
        // 特殊
        ("vu", 'ゔ'),
    ];
    entries.iter().map(|&(k, v)| (k.to_string(), v)).collect()
}

fn build_reverse_map(forward: &FxHashMap<String, char>) -> FxHashMap<char, String> {
    let mut reverse: FxHashMap<char, String> =
        FxHashMap::with_capacity_and_hasher(forward.len(), FxBuildHasher);
    for (romaji, &kana) in forward {
        reverse
            .entry(kana)
            .and_modify(|existing| {
                // 短い方を優先（"ki" > "kya"、"hu" vs "fu" は先勝ち）
                if romaji.len() < existing.len() {
                    existing.clone_from(romaji);
                }
            })
            .or_insert_with(|| romaji.clone());
    }
    reverse
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kana_table_non_empty() {
        let t = KanaTable::build();
        assert!(t.kana_for_romaji("a").is_some());
    }

    #[test]
    fn vowel_mappings() {
        let t = KanaTable::build();
        assert_eq!(t.kana_for_romaji("a"), Some('あ'));
        assert_eq!(t.kana_for_romaji("i"), Some('い'));
        assert_eq!(t.kana_for_romaji("u"), Some('う'));
        assert_eq!(t.kana_for_romaji("e"), Some('え'));
        assert_eq!(t.kana_for_romaji("o"), Some('お'));
    }

    #[test]
    fn common_romaji_mappings() {
        let t = KanaTable::build();
        assert_eq!(t.kana_for_romaji("ka"), Some('か'));
        assert_eq!(t.kana_for_romaji("si"), Some('し'));
        assert_eq!(t.kana_for_romaji("tu"), Some('つ'));
        assert_eq!(t.kana_for_romaji("nn"), Some('ん'));
    }

    #[test]
    fn nicola_special_romaji() {
        let t = KanaTable::build();
        assert_eq!(t.kana_for_romaji("wo"), Some('を'));
        assert_eq!(t.kana_for_romaji("vu"), Some('ゔ'));
    }

    #[test]
    fn fu_and_hu_both_map_to_fu() {
        let t = KanaTable::build();
        assert_eq!(t.kana_for_romaji("fu"), Some('ふ'));
        assert_eq!(t.kana_for_romaji("hu"), Some('ふ'));
    }

    #[test]
    fn youon_maps_to_representative_char() {
        let t = KanaTable::build();
        assert_eq!(t.kana_for_romaji("kya"), Some('き'));
        assert_eq!(t.kana_for_romaji("sya"), Some('し'));
        assert_eq!(t.kana_for_romaji("nya"), Some('に'));
    }

    // ── romaji_for_kana (逆引き) ──

    #[test]
    fn kana_to_romaji_basic() {
        let t = KanaTable::build();
        assert_eq!(t.romaji_for_kana('あ'), Some("a"));
        assert_eq!(t.romaji_for_kana('か'), Some("ka"));
        assert_eq!(t.romaji_for_kana('ん'), Some("nn"));
    }

    #[test]
    fn kana_to_romaji_prefers_shorter() {
        let t = KanaTable::build();
        // 'き' has "ki" (2) and "kya"/"kyu"/"kyo" (3) → "ki" wins
        assert_eq!(t.romaji_for_kana('き'), Some("ki"));
        assert_eq!(t.romaji_for_kana('し'), Some("si"));
    }

    #[test]
    fn kana_to_romaji_dakuon() {
        let t = KanaTable::build();
        assert_eq!(t.romaji_for_kana('が'), Some("ga"));
        assert_eq!(t.romaji_for_kana('ぱ'), Some("pa"));
    }
}
