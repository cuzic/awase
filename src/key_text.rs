//! 設定のキー名の文字列を揃える、VK の値を持たない純粋な関数(ADR-201 決定1)。
//!
//! コア `awase` の設定検証(`AppConfig::validate`)は、キーの意味(かな・F15〜F24・
//! 変換・無変換)を文字列で判定する。名前 → VK の表は Windows 側(`awase-windows/src/vk.rs`
//! の `VkCodeExt::from_name`)にあり、コアは持てない(ADR-019)。そこで、両側が同じ
//! 正規化 [`canonical_key_text`] を通して比べることで、規則を1か所にそろえる。
//!
//! - `from_name` は最初に [`canonical_key_text`] を呼び、正規化した名前で表を引く。
//! - コアの検証は [`key_identity`](= [`canonical_key_text`] + 少数の別名)の完全一致で比べる。
//! - 組み合わせの文字列(`"Ctrl+Shift+変換"`)は [`split_combo`] で区切る。Windows 側の
//!   修飾キー解釈(`vk::parse_key_combo`)も同じ関数を使い、区切りの規則を1つにする。

/// キー名を正規化する。順序は **前後の空白を除く → ASCII を大文字にする → 先頭の
/// `VK_` を除く**(この順序が重要。先に `VK_` を除くと `"vk_f15"` の `vk_` が大文字でなく
/// 残り、`"F15"` と揃わない。ADR-201 R3-2)。
///
/// 日本語名(`変換` 等)は `to_ascii_uppercase` で変わらない。`str::trim` は全角空白
/// (U+3000)も除くが、寛容にする方向なので許容する。
#[must_use]
pub fn canonical_key_text(s: &str) -> String {
    let upper = s.trim().to_ascii_uppercase();
    let stripped = upper.strip_prefix("VK_").map(str::to_string);
    stripped.unwrap_or(upper)
}

/// 組み合わせの文字列を `+` で区切り、`(修飾キー部分のトークン, 主キーのトークン)` を返す。
/// 各トークンは前後の空白を除く。文字列だけを扱い、修飾キーの意味は解釈しない。
///
/// 端の場合: `"F12"` → `([], "F12")`、`"Ctrl+"` → `(["Ctrl"], "")`、`"+"` → `([""], "")`、
/// `""` → `([], "")`。`+` の文字そのものは `VK_OEM_PLUS` の名前で書くので区切りとぶつからない。
#[must_use]
pub fn split_combo(s: &str) -> (Vec<&str>, &str) {
    let mut parts: Vec<&str> = s.split('+').map(str::trim).collect();
    // `split` は空文字列でも要素を1つ返すので `pop` は必ず `Some`。
    let main = parts.pop().unwrap_or("");
    (parts, main)
}

/// [`canonical_key_text`] では揃わない、コアの検証が意味を問うキーの別名(正規化後の形 → 組の名前)。
/// `Nonconvert` は正規化で `NONCONVERT` になり `VK_NONCONVERT` と揃うので載せない。
/// `from_name` 側にこの組へ入る新しい名前を足したら、ここにも足す
/// (`awase-windows` の `core_key_identity_covers_from_name` が漏れを検出する)。
const KEY_IDENTITY_ALIASES: &[(&str, &str)] = &[
    ("変換", "CONVERT"),
    ("無変換", "NONCONVERT"),
    ("MUHENKAN", "NONCONVERT"),
    ("かな", "KANA"),
    ("カナ", "KANA"),
];

/// キー名が指す「組」を返す。[`canonical_key_text`] の結果を、別名の表で組の名前へ寄せる。
/// 検証の比較はこの結果の完全一致で行う(組み合わせ全体への `contains` は使わない)。
#[must_use]
pub fn key_identity(s: &str) -> String {
    let canonical = canonical_key_text(s);
    KEY_IDENTITY_ALIASES
        .iter()
        .find(|(alias, _)| *alias == canonical)
        .map_or(canonical, |(_, group)| (*group).to_string())
}

/// 組み合わせの主キーの [`key_identity`]。修飾キーの有無は問わない。
#[must_use]
pub fn combo_main_identity(s: &str) -> String {
    key_identity(split_combo(s).1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_order_is_trim_upper_then_strip_prefix() {
        assert_eq!(canonical_key_text("  vk_f15 "), "F15");
        assert_eq!(canonical_key_text("VK_F15"), "F15");
        assert_eq!(canonical_key_text("F15"), "F15");
        assert_eq!(canonical_key_text("Nonconvert"), "NONCONVERT");
        assert_eq!(canonical_key_text("無変換"), "無変換");
        assert_eq!(canonical_key_text("IMEオン"), "IMEオン");
        // 接頭辞は1回だけ除く。
        assert_eq!(canonical_key_text("VK_VK_F12"), "VK_F12");
        assert_eq!(canonical_key_text(""), "");
    }

    #[test]
    fn split_combo_edge_cases() {
        assert_eq!(split_combo("F12"), (vec![], "F12"));
        assert_eq!(split_combo(" F12 "), (vec![], "F12"));
        assert_eq!(split_combo("Ctrl+"), (vec!["Ctrl"], ""));
        assert_eq!(split_combo("+"), (vec![""], ""));
        assert_eq!(split_combo(""), (vec![], ""));
        assert_eq!(
            split_combo(" Ctrl + Shift + 変換 "),
            (vec!["Ctrl", "Shift"], "変換")
        );
        assert_eq!(
            split_combo("Ctrl+VK_OEM_PLUS"),
            (vec!["Ctrl"], "VK_OEM_PLUS")
        );
    }

    #[test]
    fn key_identity_groups() {
        for s in ["Kana", "VK_KANA", "vk_kana", "かな", "カナ", " kana "] {
            assert_eq!(key_identity(s), "KANA", "{s}");
        }
        for s in ["変換", "Convert", "VK_CONVERT", "vk_convert"] {
            assert_eq!(key_identity(s), "CONVERT", "{s}");
        }
        for s in ["無変換", "Nonconvert", "VK_NONCONVERT", "VK_MUHENKAN"] {
            assert_eq!(key_identity(s), "NONCONVERT", "{s}");
        }
        assert_eq!(key_identity("vk_f15"), "F15");
        assert_eq!(combo_main_identity("Ctrl+Shift+変換"), "CONVERT");
        assert_eq!(combo_main_identity("F15"), "F15");
    }
}
