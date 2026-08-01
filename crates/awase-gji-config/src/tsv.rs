//! `custom_keymap_table`（[`crate::wire::GjiRawConfig::custom_keymap_table`]）
//! の中身は `status\tkey\tcommand` ヘッダに続く TSV。
//! 例: `Composition\tHankaku/Zenkaku\tIMEOff`

/// TSV の1データ行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeymapRow {
    /// GJI 内部の入力状態（`Composition`/`Conversion`/`DirectInput`/
    /// `Precomposition`/`Prediction`/`Suggestion` 等）。
    pub status: String,
    /// キー表記（`"F21"`, `"Hankaku/Zenkaku"`, `"Ctrl Shift Insert"` 等、
    /// 修飾キーが付く場合は空白区切りでメインキーの前に並ぶ）。
    pub key: String,
    /// GJI 内部コマンド名（`"IMEOn"`, `"IMEOff"`, `"Backspace"` 等）。
    pub command: String,
}

/// `custom_keymap_table` の TSV 文字列を行の列にパースする。
///
/// ヘッダ行（`status\tkey\tcommand`）は読み飛ばす。列数が3に満たない行、
/// 空行は無視する（壊れたデータでもパニックしない）。
#[must_use]
pub fn parse_custom_keymap_table(text: &str) -> Vec<KeymapRow> {
    text.lines()
        .filter_map(|line| {
            let mut columns = line.split('\t');
            let status = columns.next()?;
            let key = columns.next()?;
            let command = columns.next()?;
            if status == "status" && key == "key" && command == "command" {
                return None;
            }
            Some(KeymapRow {
                status: status.to_string(),
                key: key.to_string(),
                command: command.to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{parse_custom_keymap_table, KeymapRow};

    #[test]
    fn parses_header_and_rows() {
        let text =
            "status\tkey\tcommand\nComposition\tHankaku/Zenkaku\tIMEOff\nDirectInput\tF21\tIMEOn\n";
        let rows = parse_custom_keymap_table(text);
        assert_eq!(
            rows,
            vec![
                KeymapRow {
                    status: "Composition".to_string(),
                    key: "Hankaku/Zenkaku".to_string(),
                    command: "IMEOff".to_string(),
                },
                KeymapRow {
                    status: "DirectInput".to_string(),
                    key: "F21".to_string(),
                    command: "IMEOn".to_string(),
                },
            ]
        );
    }

    #[test]
    fn empty_string_yields_no_rows() {
        assert!(parse_custom_keymap_table("").is_empty());
    }

    #[test]
    fn malformed_lines_are_skipped_without_panicking() {
        let text = "status\tkey\tcommand\n\nComposition\tOnlyTwoColumns\nDirectInput\tF21\tIMEOn\n";
        let rows = parse_custom_keymap_table(text);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, "F21");
    }
}
