//! 設定の読み込み時の診断(ADR-201 決定2): 未知のキーの検出と、近い名前の提案。
//!
//! `serde_ignored` が集めた「無視されたキーのパス」(`general.no_such`、`keymapz` 等)を、
//! ユーザー向けの警告文にする。VK の値・OS 依存は持たない(ADR-019)。

/// 撤去済みで、旧 `config.toml` に残っていても警告しないキー(パス)。
/// 撤去の経緯は `config.rs` 冒頭の NOTE と ADR-191(`apply_calibrated_mode_keys`・
/// `[[calibration]]`・`dbe_mode_key_policy`・`gji_thumb_key_ime_toggle`)。
/// 「わざと無視される」ことは `config.rs` の `test_removed_*` が確かめている。
const REMOVED_KEYS: &[&str] = &[
    "general.output_mode",
    "general.hook_mode",
    "general.conv_mode_policy",
    "general.apply_calibrated_mode_keys",
    "general.dbe_mode_key_policy",
    "general.gji_thumb_key_ime_toggle",
    "calibration",
];

/// 撤去済みのキーか。
#[must_use]
pub fn is_removed_key(path: &str) -> bool {
    REMOVED_KEYS.contains(&path)
}

/// 2つの文字列の編集距離(Levenshtein、文字単位)。
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// `name` に近い既知の名前(編集距離2以内、または一方がもう一方の接頭辞)を返す。
#[must_use]
pub fn suggest<'a>(name: &str, known: &'a [String]) -> Option<&'a str> {
    let lower = name.to_ascii_lowercase();
    known
        .iter()
        .filter(|k| {
            let kl = k.to_ascii_lowercase();
            kl != lower
                && (edit_distance(&lower, &kl) <= 2
                    || (lower.len() >= 3 && (kl.starts_with(&lower) || lower.starts_with(&kl))))
        })
        .min_by_key(|k| edit_distance(&lower, &k.to_ascii_lowercase()))
        .map(String::as_str)
}

/// 無視されたキーの警告文を作る。`known_siblings` は同じ階層の既知のキー名(提案用、空でもよい)。
#[must_use]
pub fn unknown_key_message(path: &str, known_siblings: &[String]) -> String {
    let leaf = path.rsplit('.').next().unwrap_or(path);
    suggest(leaf, known_siblings).map_or_else(
        || format!("config.toml の \"{path}\" は未知のキーのため無視されます"),
        |s| {
            format!(
                "config.toml の \"{path}\" は未知のキーのため無視されます(\"{s}\" の間違いではありませんか)"
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn edit_distance_basic() {
        assert_eq!(edit_distance("keymap", "keymaps"), 1);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("same", "same"), 0);
    }

    #[test]
    fn suggests_close_names_only() {
        let known = names(&["general", "keys", "keymaps", "post_bypass"]);
        assert_eq!(suggest("keymapz", &known), Some("keymaps"));
        assert_eq!(suggest("post_bypas", &known), Some("post_bypass"));
        assert_eq!(suggest("generl", &known), Some("general"));
        assert_eq!(suggest("futuresection", &known), None);
        // 大文字小文字だけの違いも(完全一致でなければ)提案する
        assert_eq!(
            suggest("KEYS", &known),
            None,
            "同一の名前(大小無視)は提案しない"
        );
    }

    #[test]
    fn removed_keys_are_recognized() {
        assert!(is_removed_key("general.apply_calibrated_mode_keys"));
        assert!(is_removed_key("calibration"));
        assert!(!is_removed_key("general.no_such"));
    }

    #[test]
    fn message_includes_suggestion_when_close() {
        let m = unknown_key_message("keymapz", &names(&["keymaps"]));
        assert!(m.contains("keymapz") && m.contains("keymaps"), "{m}");
        let m = unknown_key_message("general.zzz", &names(&["threshold"]));
        assert!(!m.contains("間違い"), "{m}");
    }
}
