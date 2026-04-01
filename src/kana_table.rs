use std::collections::HashMap;

/// ローマ字→ひらがな逆引きテーブルを構築する（n-gram スコアリング用）。
///
/// 訓令式ローマ字を基本とし、ヘボン式の一部（"fu"→'ふ' 等）も含む。
/// 拗音（"kya" 等）は n-gram のバイグラム文脈で先頭文字が重要なため、
/// 代表文字（'き' 等）にマッピングする。
#[allow(clippy::too_many_lines)]
#[must_use]
pub fn build_romaji_to_kana() -> HashMap<String, char> {
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

/// かな→ローマ字の逆引きテーブルを構築する。
///
/// `build_romaji_to_kana` の逆方向マッピング。
/// 同一かなに複数のローマ字が対応する場合、最も短いものを採用する。
/// 拗音の代表文字マッピング（"kya"→'き' 等）は基本マッピング（"ki"→'き'）より
/// 長いため自動的に除外される。
#[must_use]
pub fn build_kana_to_romaji() -> HashMap<char, String> {
    let forward = build_romaji_to_kana();
    let mut reverse: HashMap<char, String> = HashMap::with_capacity(forward.len());
    for (romaji, &kana) in &forward {
        reverse
            .entry(kana)
            .and_modify(|existing| {
                // 短い方を優先（"ki" > "kya"、"hu" vs "fu" は先勝ち）
                if romaji.len() < existing.len() {
                    *existing = romaji.clone();
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
    fn build_romaji_to_kana_non_empty() {
        let table = build_romaji_to_kana();
        assert!(!table.is_empty());
    }

    #[test]
    fn vowel_mappings() {
        let table = build_romaji_to_kana();
        assert_eq!(table.get("a"), Some(&'あ'));
        assert_eq!(table.get("i"), Some(&'い'));
        assert_eq!(table.get("u"), Some(&'う'));
        assert_eq!(table.get("e"), Some(&'え'));
        assert_eq!(table.get("o"), Some(&'お'));
    }

    #[test]
    fn common_romaji_mappings() {
        let table = build_romaji_to_kana();
        assert_eq!(table.get("ka"), Some(&'か'));
        assert_eq!(table.get("si"), Some(&'し'));
        assert_eq!(table.get("tu"), Some(&'つ'));
        assert_eq!(table.get("nn"), Some(&'ん'));
    }

    #[test]
    fn nicola_special_romaji() {
        let table = build_romaji_to_kana();
        assert_eq!(table.get("wo"), Some(&'を'));
        assert_eq!(table.get("vu"), Some(&'ゔ'));
    }

    #[test]
    fn fu_and_hu_both_map_to_fu() {
        let table = build_romaji_to_kana();
        assert_eq!(table.get("fu"), Some(&'ふ'));
        assert_eq!(table.get("hu"), Some(&'ふ'));
    }

    #[test]
    fn youon_maps_to_representative_char() {
        let table = build_romaji_to_kana();
        assert_eq!(table.get("kya"), Some(&'き'));
        assert_eq!(table.get("sya"), Some(&'し'));
        assert_eq!(table.get("nya"), Some(&'に'));
    }

    // ── kana_to_romaji (逆引き) ──

    #[test]
    fn kana_to_romaji_basic() {
        let table = build_kana_to_romaji();
        assert_eq!(table.get(&'あ'), Some(&"a".to_string()));
        assert_eq!(table.get(&'か'), Some(&"ka".to_string()));
        assert_eq!(table.get(&'ん'), Some(&"nn".to_string()));
    }

    #[test]
    fn kana_to_romaji_prefers_shorter() {
        let table = build_kana_to_romaji();
        // 'き' has "ki" (2) and "kya"/"kyu"/"kyo" (3) → "ki" wins
        assert_eq!(table.get(&'き'), Some(&"ki".to_string()));
        assert_eq!(table.get(&'し'), Some(&"si".to_string()));
    }

    #[test]
    fn kana_to_romaji_dakuon() {
        let table = build_kana_to_romaji();
        assert_eq!(table.get(&'が'), Some(&"ga".to_string()));
        assert_eq!(table.get(&'ぱ'), Some(&"pa".to_string()));
    }
}
