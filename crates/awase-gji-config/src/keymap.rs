//! GJI のキー表記を awase の VK 名に変換する。
//!
//! Mozc `key_parser` 由来のトークン（例: `"F21"`, `"Hankaku/Zenkaku"`,
//! `"Kanji"`）を、awase の VK 名（`VkCode::from_name` が受理する文字列。
//! 例: `"VK_F21"`, `"VK_KANJI"`）へ変換し、GJI の `custom_keymap_table`
//! から IME ON/OFF に使われているキーの集合を抽出する。
//!
//! スコープ（stage 1）: 修飾キー付きの行（`"Ctrl Shift Insert"` 等）は
//! 対象外（ログのみ、無視）。awase 側の `ime_detect.on/off/toggle`
//! （`VkCode::from_name` 直読み、修飾キー非対応）にそのまま乗せられる
//! 単発キーのみを扱う。

use std::collections::{BTreeMap, BTreeSet};

use crate::tsv::{KeymapRow, parse_custom_keymap_table};

/// GJI の `IMEOn`/`IMEOff` コマンドが割り当てられている、GJI 内部の入力状態が
/// 「IME は実質 OFF（未起動）」を表すもの。この状態群でのみ `IMEOn` が割り当て
/// られているキーは「ONトリガー」の候補になる。
const STATUSES_WHEN_IME_OFF: &[&str] = &["DirectInput"];

/// 「IME は実質 ON（起動済み、入力前後を問わない）」を表す状態群。
/// この状態群でのみ `IMEOff` が割り当てられているキーは「OFFトリガー」の候補
/// になる。
const STATUSES_WHEN_IME_ON: &[&str] = &[
    "Precomposition",
    "Composition",
    "Conversion",
    "Prediction",
    "Suggestion",
];

/// Mozc キー名の別名テーブル。`F1`..`F24` は規則的なので別途扱う（下記
/// [`mozc_key_to_vk_name`] 参照）。
///
/// - `"Hankaku/Zenkaku"` は awase 側の `vk.rs` doc コメントに実測記載がある
///   通り、物理的には `VK_KANJI` と同じキー。
/// - `"ON"`/`"OFF"` は GJI 内部の擬似キー名で、`VK_IME_ON`/`VK_IME_OFF`
///   （SendInput 等で仮想的に送出される合成キー）に対応する。
/// - `"Eisu"` は英数キー（`VK_DBE_ALPHANUMERIC`）。
const MOZC_KEY_ALIASES: &[(&str, &str)] = &[
    ("Kanji", "VK_KANJI"),
    ("Hankaku/Zenkaku", "VK_KANJI"),
    ("ON", "VK_IME_ON"),
    ("OFF", "VK_IME_OFF"),
    ("Eisu", "VK_DBE_ALPHANUMERIC"),
];

/// Mozc のキートークンを awase の VK 名に変換する。対応表に無い、または
/// `F` に続く数値が VK_F1..VK_F24 の範囲外のトークンは `None`
/// （安全側に倒し、未知の VK を検出キーに使わない）。
#[must_use]
pub fn mozc_key_to_vk_name(key: &str) -> Option<String> {
    if let Some((_, vk_name)) = MOZC_KEY_ALIASES.iter().find(|(mozc_key, _)| *mozc_key == key) {
        return Some((*vk_name).to_string());
    }
    let digits = key.strip_prefix('F')?;
    let number: u8 = digits.parse().ok()?;
    if (1..=24).contains(&number) {
        Some(format!("VK_F{number}"))
    } else {
        None
    }
}

/// GJI の custom keymap から抽出した、IME ON/OFF 検出に使える VK 名の集合。
/// いずれも `awase::config::ImeDetectConfig.toggle/on/off` へそのまま
/// 反映できる形（`VkCode::from_name` が受理する文字列）。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GjiImeKeys {
    /// このキー単独で IME を ON にする（`ImeDetectConfig.on` 相当）。
    pub on: Vec<String>,
    /// このキー単独で IME を OFF にする（`ImeDetectConfig.off` 相当）。
    pub off: Vec<String>,
    /// このキーで ON/OFF がトグルする（`ImeDetectConfig.toggle` 相当）。
    pub toggle: Vec<String>,
}

/// キーごとに集計した「IMEOn が割り当てられている状態の集合」と
/// 「IMEOff が割り当てられている状態の集合」。
type StatusSetsByKey = BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)>;

/// `custom_keymap_table` の TSV 文字列から [`GjiImeKeys`] を構築する。
///
/// 手順:
/// 1. `command` が `IMEOn`/`IMEOff` の行だけを残す。
/// 2. `key` に空白を含む行（修飾キー付き）は stage 1 のスコープ外として除外
///    （`log::debug!` のみ、エラーにはしない）。
/// 3. 残った行をキー単位で集約し、[`STATUSES_WHEN_IME_OFF`]/
///    [`STATUSES_WHEN_IME_ON`] に基づいて toggle/on/off/矛盾（除外）に分類する。
#[must_use]
pub fn extract_ime_keys(custom_keymap_table: &str) -> GjiImeKeys {
    let rows = parse_custom_keymap_table(custom_keymap_table);
    let grouped = group_ime_rows_by_key(&rows);

    let mut result = GjiImeKeys::default();
    for (key, (on_statuses, off_statuses)) in grouped {
        let Some(vk_name) = mozc_key_to_vk_name(&key) else {
            log::warn!("gji-config: 未対応のキートークンをスキップしました: key={key}");
            continue;
        };
        classify_and_push(&key, &vk_name, &on_statuses, &off_statuses, &mut result);
    }

    result.on.sort_unstable();
    result.on.dedup();
    result.off.sort_unstable();
    result.off.dedup();
    result.toggle.sort_unstable();
    result.toggle.dedup();
    result
}

/// `IMEOn`/`IMEOff` 以外のコマンドの行・修飾キー付きの行を除外しつつ、
/// キーごとに「IMEOn が割り当てられている状態」「IMEOff が割り当てられている
/// 状態」を集計する。
fn group_ime_rows_by_key(rows: &[KeymapRow]) -> StatusSetsByKey {
    let mut grouped: StatusSetsByKey = BTreeMap::new();
    for row in rows {
        // command でまず絞る: IMEOn/IMEOff 以外の行（Backspace 等)は空白キー
        // 判定より先に捨てる。修飾キー付き行のログを IME 無関係コマンドで
        // 発火させないため。
        let is_ime_on = row.command == "IMEOn";
        let is_ime_off = row.command == "IMEOff";
        if !is_ime_on && !is_ime_off {
            continue;
        }
        if row.key.contains(char::is_whitespace) {
            log::debug!(
                "gji-config: 修飾キー付き行は stage 1 のスコープ外のため無視: key={}",
                row.key
            );
            continue;
        }
        let entry = if is_ime_on {
            &mut grouped.entry(row.key.clone()).or_default().0
        } else {
            &mut grouped.entry(row.key.clone()).or_default().1
        };
        entry.insert(row.status.clone());
    }
    grouped
}

/// 1キー分の on/off 状態集合を、toggle/on/off のいずれかに分類して
/// `result` に積む。どちらにも当てはまらない（状態間で矛盾する）場合は
/// 警告ログのみで取り込まない。
fn classify_and_push(
    key: &str,
    vk_name: &str,
    on_statuses: &BTreeSet<String>,
    off_statuses: &BTreeSet<String>,
    result: &mut GjiImeKeys,
) {
    let on_only_in_off_states =
        !on_statuses.is_empty() && on_statuses.iter().all(|s| STATUSES_WHEN_IME_OFF.contains(&s.as_str()));
    let off_only_in_on_states =
        !off_statuses.is_empty() && off_statuses.iter().all(|s| STATUSES_WHEN_IME_ON.contains(&s.as_str()));

    if on_only_in_off_states && off_only_in_on_states {
        result.toggle.push(vk_name.to_string());
    } else if !on_statuses.is_empty() && off_statuses.is_empty() {
        result.on.push(vk_name.to_string());
    } else if !off_statuses.is_empty() && on_statuses.is_empty() {
        result.off.push(vk_name.to_string());
    } else {
        log::warn!(
            "gji-config: 状態間で矛盾する割当のためスキップしました: key={key} on={on_statuses:?} off={off_statuses:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{GjiImeKeys, extract_ime_keys, mozc_key_to_vk_name};

    #[test]
    fn f_key_tokens_map_to_vk_names() {
        assert_eq!(mozc_key_to_vk_name("F1").as_deref(), Some("VK_F1"));
        assert_eq!(mozc_key_to_vk_name("F21").as_deref(), Some("VK_F21"));
        assert_eq!(mozc_key_to_vk_name("F24").as_deref(), Some("VK_F24"));
        assert_eq!(mozc_key_to_vk_name("F25"), None);
        assert_eq!(mozc_key_to_vk_name("F0"), None);
    }

    #[test]
    fn alias_tokens_map_to_vk_names() {
        assert_eq!(mozc_key_to_vk_name("Kanji").as_deref(), Some("VK_KANJI"));
        assert_eq!(
            mozc_key_to_vk_name("Hankaku/Zenkaku").as_deref(),
            Some("VK_KANJI")
        );
        assert_eq!(mozc_key_to_vk_name("ON").as_deref(), Some("VK_IME_ON"));
        assert_eq!(mozc_key_to_vk_name("OFF").as_deref(), Some("VK_IME_OFF"));
        assert_eq!(
            mozc_key_to_vk_name("Eisu").as_deref(),
            Some("VK_DBE_ALPHANUMERIC")
        );
    }

    #[test]
    fn unknown_token_maps_to_none() {
        assert_eq!(mozc_key_to_vk_name("Insert"), None);
        assert_eq!(mozc_key_to_vk_name(""), None);
    }

    /// 実機で取得した GJI カスタムキーマップの実データに現れたパターンを
    /// 反映したフィクスチャ（このセッションで実機確認済みの値そのものではなく、
    /// 同じ構造を持つ代表例として再構成したもの。個人設定の丸ごとコミットは
    /// 避ける）。
    const FIXTURE_TSV: &str = "status\tkey\tcommand
Composition\tHankaku/Zenkaku\tIMEOff
Conversion\tHankaku/Zenkaku\tIMEOff
DirectInput\tHankaku/Zenkaku\tIMEOn
Precomposition\tHankaku/Zenkaku\tIMEOff
Composition\tKanji\tIMEOff
Conversion\tKanji\tIMEOff
DirectInput\tKanji\tIMEOn
Precomposition\tKanji\tIMEOff
DirectInput\tF13\tIMEOn
DirectInput\tF21\tIMEOn
Precomposition\tF21\tIMEOn
Composition\tF21\tIMEOn
Conversion\tF21\tIMEOn
Precomposition\tF22\tIMEOff
Composition\tF22\tIMEOff
Conversion\tF22\tIMEOff
Composition\tON\tIMEOn
Composition\tOFF\tIMEOff
Conversion\tON\tIMEOn
Conversion\tOFF\tIMEOff
DirectInput\tON\tIMEOn
Precomposition\tON\tIMEOn
Precomposition\tOFF\tIMEOff
DirectInput\tEisu\tIMEOn
Composition\tEisu\tToggleAlphanumericMode
Conversion\tEisu\tToggleAlphanumericMode
Precomposition\tEisu\tToggleAlphanumericMode
Precomposition\tCtrl Shift Insert\tIMEOn
Precomposition\tCtrl Shift Delete\tIMEOff
Composition\tBackspace\tBackspace
Composition\tSpace\tConvert
";

    #[test]
    fn extracts_toggle_on_off_from_fixture() {
        let keys = extract_ime_keys(FIXTURE_TSV);
        assert_eq!(
            keys,
            GjiImeKeys {
                on: vec![
                    "VK_DBE_ALPHANUMERIC".to_string(), // Eisu (DirectInputのみ)
                    "VK_F13".to_string(),
                    "VK_F21".to_string(),
                    "VK_IME_ON".to_string(), // ON
                ],
                off: vec!["VK_F22".to_string(), "VK_IME_OFF".to_string()], // OFF
                toggle: vec!["VK_KANJI".to_string()], // Hankaku/Zenkaku と Kanji がどちらも集約
            }
        );
    }

    #[test]
    fn empty_table_yields_empty_keys() {
        assert_eq!(extract_ime_keys(""), GjiImeKeys::default());
    }

    #[test]
    fn conflicting_key_is_dropped_not_panicking() {
        // 同じ状態グループ内で on/off が両方立つ、解釈不能な矛盾行。
        let text = "status\tkey\tcommand\nComposition\tF15\tIMEOn\nComposition\tF15\tIMEOff\n";
        let keys = extract_ime_keys(text);
        assert_eq!(keys, GjiImeKeys::default());
    }

    #[test]
    fn prediction_and_suggestion_are_recognized_as_ime_on_states() {
        // STATUSES_WHEN_IME_ON の5状態のうちメインfixtureがカバーしないのは
        // Prediction/Suggestion。この2つが正しく toggle 判定に効くことを
        // 個別に固定する（この定数を削っても検知できるように）。
        let text = "status\tkey\tcommand
DirectInput\tF20\tIMEOn
Prediction\tF20\tIMEOff
DirectInput\tF19\tIMEOn
Suggestion\tF19\tIMEOff
";
        let keys = extract_ime_keys(text);
        assert_eq!(
            keys,
            GjiImeKeys {
                on: vec![],
                off: vec![],
                toggle: vec!["VK_F19".to_string(), "VK_F20".to_string()],
            }
        );
    }
}
