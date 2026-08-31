use super::*;

// ── 全角↔半角変換テスト ──

#[test]
fn fullwidth_alpha_to_halfwidth() {
    assert_eq!('ａ'.to_halfwidth_ascii(), Some('a'));
    assert_eq!('ｚ'.to_halfwidth_ascii(), Some('z'));
    assert_eq!('Ａ'.to_halfwidth_ascii(), Some('A'));
    assert_eq!('Ｚ'.to_halfwidth_ascii(), Some('Z'));
}

#[test]
fn fullwidth_digit_to_halfwidth() {
    assert_eq!('０'.to_halfwidth_ascii(), Some('0'));
    assert_eq!('９'.to_halfwidth_ascii(), Some('9'));
}

#[test]
fn fullwidth_symbol_to_halfwidth() {
    assert_eq!('！'.to_halfwidth_ascii(), Some('!'));
    assert_eq!('？'.to_halfwidth_ascii(), Some('?'));
    assert_eq!('＃'.to_halfwidth_ascii(), Some('#'));
}

#[test]
fn non_fullwidth_returns_none() {
    assert_eq!('a'.to_halfwidth_ascii(), None);
    assert_eq!('あ'.to_halfwidth_ascii(), None);
}

#[test]
fn fullwidth_string_conversion() {
    assert_eq!("ｋａ".to_halfwidth_str(), "ka");
    assert_eq!("ｓｉ".to_halfwidth_str(), "si");
    assert_eq!("Ａ".to_halfwidth_str(), "A");
    assert_eq!("１２３".to_halfwidth_str(), "123");
}

// ── parse_value テスト ──

#[test]
fn parse_value_none() {
    assert_eq!(YabValue::parse("無"), YabValue::None);
    assert_eq!(YabValue::parse(""), YabValue::None);
    assert_eq!(YabValue::parse("  "), YabValue::None);
}

#[test]
fn parse_value_special_keys() {
    assert_eq!(
        YabValue::parse("後"),
        YabValue::Special(SpecialKey::Backspace)
    );
    assert_eq!(YabValue::parse("逃"), YabValue::Special(SpecialKey::Escape));
    assert_eq!(YabValue::parse("入"), YabValue::Special(SpecialKey::Enter));
    assert_eq!(YabValue::parse("空"), YabValue::Special(SpecialKey::Space));
    assert_eq!(YabValue::parse("消"), YabValue::Special(SpecialKey::Delete));
}

#[test]
fn parse_value_extended_special_keys() {
    // やまぶきR互換: 挿/上/左/右/下/家/終/前/次
    assert_eq!(YabValue::parse("挿"), YabValue::Special(SpecialKey::Insert));
    assert_eq!(YabValue::parse("上"), YabValue::Special(SpecialKey::Up));
    assert_eq!(YabValue::parse("左"), YabValue::Special(SpecialKey::Left));
    assert_eq!(YabValue::parse("右"), YabValue::Special(SpecialKey::Right));
    assert_eq!(YabValue::parse("下"), YabValue::Special(SpecialKey::Down));
    assert_eq!(YabValue::parse("家"), YabValue::Special(SpecialKey::Home));
    assert_eq!(YabValue::parse("終"), YabValue::Special(SpecialKey::End));
    assert_eq!(YabValue::parse("前"), YabValue::Special(SpecialKey::PageUp));
    assert_eq!(
        YabValue::parse("次"),
        YabValue::Special(SpecialKey::PageDown)
    );
}

#[test]
fn parse_value_direct_vk() {
    assert_eq!(YabValue::parse("V1D"), YabValue::Vk(VkCode(0x1D)));
    assert_eq!(YabValue::parse("V0A"), YabValue::Vk(VkCode(0x0A)));
    // 小文字の v は対象外（やまぶきR仕様: 半角大文字の V のみ）
    assert_eq!(YabValue::parse("v1D"), YabValue::Literal("v1D".to_string()));
    // 16進として不正な場合はリテラルにフォールバック
    assert_eq!(YabValue::parse("VZZ"), YabValue::Literal("VZZ".to_string()));
}

#[test]
fn parse_value_function_key() {
    assert_eq!(YabValue::parse("機1"), YabValue::Vk(VkCode(0x70)));
    assert_eq!(YabValue::parse("機13"), YabValue::Vk(VkCode(0x7C)));
    assert_eq!(YabValue::parse("機24"), YabValue::Vk(VkCode(0x87)));
    // 範囲外はリテラルにフォールバック
    assert_eq!(
        YabValue::parse("機25"),
        YabValue::Literal("機25".to_string())
    );
    assert_eq!(YabValue::parse("機0"), YabValue::Literal("機0".to_string()));
}

#[test]
fn parse_value_literal_escape_sequences() {
    assert_eq!(
        YabValue::parse("'a\\nb'"),
        YabValue::Literal("a\nb".to_string())
    );
    assert_eq!(
        YabValue::parse("'a\\tb'"),
        YabValue::Literal("a\tb".to_string())
    );
    assert_eq!(
        YabValue::parse("'back\\\\slash'"),
        YabValue::Literal("back\\slash".to_string())
    );
    assert_eq!(
        YabValue::parse("'it\\'s'"),
        YabValue::Literal("it's".to_string())
    );
    assert_eq!(
        YabValue::parse("\"say \\\"hi\\\"\""),
        YabValue::Literal("say \"hi\"".to_string())
    );
    // \u + 16進コードポイント
    assert_eq!(
        YabValue::parse("'\\u25CF'"),
        YabValue::Literal("●".to_string())
    );
}

#[test]
fn parse_value_single_quoted_literal() {
    assert_eq!(YabValue::parse("'．'"), YabValue::Literal("．".to_string()));
    assert_eq!(YabValue::parse("'ー'"), YabValue::Literal("ー".to_string()));
}

// ── lint_raw_cell / lint（report 01M13EACMQ7D2VETW75N0BTZ9C）──

/// 実際のバグ報告に現れた誤字（`ｂ'ｕ`、正しくは `ｂｕ`）を検出する。
#[test]
fn lint_raw_cell_flags_unpaired_quote_mid_token() {
    let msg = YabValue::lint_raw_cell("ｂ'ｕ");
    assert!(msg.is_some(), "対になっていないクォートを検出すべき");
    assert!(msg.unwrap().contains("ｂ'ｕ"));
    // parse 自体は落とさず、これまで通りリテラルとして受理する。
    assert_eq!(
        YabValue::parse("ｂ'ｕ"),
        YabValue::Literal("ｂ'ｕ".to_string())
    );
}

#[test]
fn lint_raw_cell_does_not_flag_properly_paired_quote() {
    assert_eq!(YabValue::lint_raw_cell("'ぶ'"), None);
    assert_eq!(YabValue::lint_raw_cell("'it\\'s'"), None);
    assert_eq!(YabValue::lint_raw_cell("\"say \\\"hi\\\"\""), None);
}

#[test]
fn lint_raw_cell_does_not_flag_normal_cells() {
    for cell in ["無", "", "  ", "ｂｕ", "後", "V1D", "機1", "ｔａ"] {
        assert_eq!(
            YabValue::lint_raw_cell(cell),
            None,
            "cell {cell:?} should not be flagged"
        );
    }
}

#[test]
fn lint_scans_whole_layout_text_and_reports_line_numbers() {
    let input =
        "[ローマ字シフト無し]\n無,ｂｉ,ｚｕ,ｂ'ｕ,ｂｅ\n; comment\n無,ｈｉ,ｓｕ,ｆｕ,ｈｅ\n";
    let warnings = lint(input);
    assert_eq!(warnings.len(), 1, "got: {warnings:?}");
    assert!(warnings[0].starts_with("2行目:"), "got: {warnings:?}");
    assert!(warnings[0].contains("ｂ'ｕ"));
}

#[test]
fn lint_returns_empty_for_default_bundled_layout_row() {
    // layout/nicola.yab の右親指シフト面より（正しい `ｂｕ`）
    let input = "無,ｂｉ,ｚｕ,ｂｕ,ｂｅ,ｎｕ,ｙｕ,ｍｕ,ｗａ,ｌｏ,無";
    assert_eq!(lint(input), Vec::<String>::new());
}

/// `/code-review` 指摘: レイアウト名行（セクション見出しより前、最初の
/// 非コメント行）はデータ行ではないため、クォート文字を含んでいても
/// タイプミス扱いしてはならない。`YabLayout::serialize` はこの名前行を
/// そのまま先頭行として再出力するため、保存直後の再lintでも誤検知しない
/// ことを固定する。
#[test]
fn lint_does_not_flag_layout_name_line_containing_quote() {
    let input =
        "Tom's Layout\n[ローマ字シフト無し]\n無,無,無,無\n無,無,無,無\n無,無,無,無\n無,無,無,無\n";
    assert_eq!(lint(input), Vec::<String>::new());
}

/// レイアウト名行の直前・直後にコメント行があっても同様に誤検知しない。
#[test]
fn lint_does_not_flag_layout_name_line_with_surrounding_comments() {
    let input = "; comment\nBob's \"Special\" Layout\n; another comment\n[ローマ字シフト無し]\n無,無,無,無\n";
    assert_eq!(lint(input), Vec::<String>::new());
}

#[test]
fn parse_value_fullwidth_romaji() {
    assert_eq!(
        YabValue::parse("ｋａ"),
        YabValue::Romaji {
            romaji: "ka".to_string(),
            kana: None
        }
    );
    assert_eq!(
        YabValue::parse("ｓｉ"),
        YabValue::Romaji {
            romaji: "si".to_string(),
            kana: None
        }
    );
    assert_eq!(
        YabValue::parse("ｗｏ"),
        YabValue::Romaji {
            romaji: "wo".to_string(),
            kana: None
        }
    );
}

#[test]
fn parse_value_fullwidth_uppercase() {
    assert_eq!(
        YabValue::parse("Ａ"),
        YabValue::Romaji {
            romaji: "A".to_string(),
            kana: None
        }
    );
    assert_eq!(
        YabValue::parse("Ｂ"),
        YabValue::Romaji {
            romaji: "B".to_string(),
            kana: None
        }
    );
}

#[test]
fn parse_value_fullwidth_digit() {
    assert_eq!(
        YabValue::parse("１"),
        YabValue::KeySequence("1".to_string())
    );
    assert_eq!(
        YabValue::parse("２"),
        YabValue::KeySequence("2".to_string())
    );
}

#[test]
fn parse_value_fullwidth_symbol() {
    assert_eq!(
        YabValue::parse("！"),
        YabValue::KeySequence("!".to_string())
    );
}

#[test]
fn parse_value_double_quoted_literal() {
    assert_eq!(
        YabValue::parse("\"？\""),
        YabValue::Literal("？".to_string())
    );
    assert_eq!(
        YabValue::parse("\"！\""),
        YabValue::Literal("！".to_string())
    );
}

#[test]
fn parse_value_key_sequence_round_trip() {
    let val = YabValue::KeySequence("?".to_string());
    let serialized = val.serialize();
    assert_eq!(serialized, "？");
    let parsed = YabValue::parse(&serialized);
    assert_eq!(parsed, YabValue::KeySequence("?".to_string()));
}

// ── 最小限のパーステスト ──

#[test]
fn parse_minimal_one_section() {
    let input = "\
; テスト用
[ローマ字シフト無し]
無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,ｋａ,無,無,無,無,無,無,無,無
無,ｓｉ,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無";

    let layout = YabLayout::parse(input, KeyboardModel::Jis).unwrap();

    // 通常面に "ka" が (1, 3) にマッピングされていること
    assert_eq!(
        layout.normal.get(&PhysicalPos::new(1, 3)),
        Some(&YabValue::Romaji {
            romaji: "ka".to_string(),
            kana: None
        })
    );
    // 通常面に "si" が (2, 1) にマッピングされていること
    assert_eq!(
        layout.normal.get(&PhysicalPos::new(2, 1)),
        Some(&YabValue::Romaji {
            romaji: "si".to_string(),
            kana: None
        })
    );
    // '無' のキーは YabValue::None として格納されていること
    assert_eq!(
        layout.normal.get(&PhysicalPos::new(0, 0)),
        Some(&YabValue::None)
    );
}

// ── NICOLA 例のパーステスト ──

#[test]
fn parse_nicola_trimmed_example() {
    let input = "\
; NICOLA配列定義（縮小版）
[ローマ字シフト無し]
１,２,３,４,５,６,７,８,９,０,'ー','＾','￥'
'．','／',ｋａ,ｓｉ,ｎａ,ｎｉ,ｒａ,ｔｉ,ｋｕ,ｔｕ,'，','＠'
ｕ,ｓｉ,ｔｅ,ｋｅ,ｓｅ,ｈａ,ｔｏ,ｋｉ,ｉ,ｎｎ,無,無
'．','／',ｓｕ,ｈｅ,ｍｅ,ｓｏ,ｎｅ,ｈｏ,無,無,無
[ローマ字左親指シフト]
無,無,無,無,無,無,無,無,無,無,無,無,無
ｇａ,ｇｉ,ｇｕ,ｇｅ,ｇｏ,無,無,無,無,無,無,無
ｖｕ,ｚｉ,ｄｅ,ｇｅ,ｚｅ,無,無,無,無,無,無,無
無,無,ｚｕ,ｂｅ,ｍｅ,無,無,無,無,無,無
[ローマ字右親指シフト]
無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,ｍｉ,ｏ,ｎｏ,ｙｏ,ｔｕ,'，',無
無,無,無,無,無,ｍｉ,ｙｏ,ｎｉ,ｒｕ,ｍａ,'：',無
無,無,無,無,無,ｙａ,ａ,ｒｅ,ｗｏ,無,無
[ローマ字小指シフト]
'！','\"','＃','＄','％','＆','＇','（','）',無,'＝','～','｜'
Ａ,Ｂ,Ｃ,Ｄ,Ｅ,Ｆ,Ｇ,Ｈ,Ｉ,Ｊ,無,無
Ｋ,Ｌ,Ｍ,Ｎ,Ｏ,Ｐ,Ｑ,Ｒ,Ｓ,Ｔ,無,無
Ｕ,Ｖ,Ｗ,Ｘ,Ｙ,Ｚ,無,'＜','＞','？',無";

    let layout = YabLayout::parse(input, KeyboardModel::Jis).unwrap();

    // 通常面の検証
    assert_eq!(
        layout.normal.get(&PhysicalPos::new(0, 0)),
        Some(&YabValue::KeySequence("1".to_string()))
    );
    assert_eq!(
        layout.normal.get(&PhysicalPos::new(1, 2)),
        Some(&YabValue::Romaji {
            romaji: "ka".to_string(),
            kana: None
        })
    );
    assert_eq!(
        layout.normal.get(&PhysicalPos::new(0, 10)),
        Some(&YabValue::Literal("ー".to_string()))
    );

    // 左親指面の検証
    assert_eq!(
        layout.left_thumb.get(&PhysicalPos::new(1, 0)),
        Some(&YabValue::Romaji {
            romaji: "ga".to_string(),
            kana: None
        })
    );

    // 右親指面の検証
    assert_eq!(
        layout.right_thumb.get(&PhysicalPos::new(1, 5)),
        Some(&YabValue::Romaji {
            romaji: "mi".to_string(),
            kana: None
        })
    );

    // 小指シフト面の検証
    assert_eq!(
        layout.shift.get(&PhysicalPos::new(0, 0)),
        Some(&YabValue::Literal("！".to_string()))
    );
    assert_eq!(
        layout.shift.get(&PhysicalPos::new(1, 0)),
        Some(&YabValue::Romaji {
            romaji: "A".to_string(),
            kana: None
        })
    );
}

// ── 特殊キーワードテスト ──

#[test]
fn parse_special_keywords_in_section() {
    let input = "\
[ローマ字シフト無し]
後,逃,入,空,消,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無";

    let layout = YabLayout::parse(input, KeyboardModel::Jis).unwrap();

    assert_eq!(
        layout.normal.get(&PhysicalPos::new(0, 0)),
        Some(&YabValue::Special(SpecialKey::Backspace))
    );
    assert_eq!(
        layout.normal.get(&PhysicalPos::new(0, 1)),
        Some(&YabValue::Special(SpecialKey::Escape))
    );
    assert_eq!(
        layout.normal.get(&PhysicalPos::new(0, 2)),
        Some(&YabValue::Special(SpecialKey::Enter))
    );
    assert_eq!(
        layout.normal.get(&PhysicalPos::new(0, 3)),
        Some(&YabValue::Special(SpecialKey::Space))
    );
    assert_eq!(
        layout.normal.get(&PhysicalPos::new(0, 4)),
        Some(&YabValue::Special(SpecialKey::Delete))
    );
}

// ── エラーケーステスト ──

#[test]
fn parse_section_with_wrong_line_count() {
    let input = "\
[ローマ字シフト無し]
無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無";
    // 3 行しかない → エラー
    let result = YabLayout::parse(input, KeyboardModel::Jis);
    assert!(result.is_err());
}

#[test]
fn parse_too_many_columns() {
    let input = "\
[ローマ字シフト無し]
無,無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無";
    // Row 0 に 14 個の値 → エラー
    let result = YabLayout::parse(input, KeyboardModel::Jis);
    assert!(result.is_err());
}

#[test]
fn parse_empty_sections_ok() {
    let input = "; コメントのみ";
    let layout = YabLayout::parse(input, KeyboardModel::Jis).unwrap();
    assert!(layout.normal.is_empty());
    assert!(layout.left_thumb.is_empty());
    assert!(layout.right_thumb.is_empty());
    assert!(layout.shift.is_empty());
}

#[test]
fn parse_comments_and_blank_lines_ignored() {
    let input = "\
; これはコメント
; もうひとつコメント

[ローマ字シフト無し]
; コメント中のデータ行ではない
ｋａ,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無";

    let layout = YabLayout::parse(input, KeyboardModel::Jis).unwrap();
    assert_eq!(
        layout.normal.get(&PhysicalPos::new(0, 0)),
        Some(&YabValue::Romaji {
            romaji: "ka".to_string(),
            kana: None
        })
    );
}

// ── 複数セクションのパーステスト ──

#[test]
fn parse_multiple_sections() {
    let input = "\
[ローマ字シフト無し]
ｋａ,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無
[ローマ字左親指シフト]
ｇａ,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無
[ローマ字右親指シフト]
ｍａ,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無
[ローマ字小指シフト]
Ａ,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無";

    let layout = YabLayout::parse(input, KeyboardModel::Jis).unwrap();

    assert_eq!(
        layout.normal.get(&PhysicalPos::new(0, 0)),
        Some(&YabValue::Romaji {
            romaji: "ka".to_string(),
            kana: None
        })
    );
    assert_eq!(
        layout.left_thumb.get(&PhysicalPos::new(0, 0)),
        Some(&YabValue::Romaji {
            romaji: "ga".to_string(),
            kana: None
        })
    );
    assert_eq!(
        layout.right_thumb.get(&PhysicalPos::new(0, 0)),
        Some(&YabValue::Romaji {
            romaji: "ma".to_string(),
            kana: None
        })
    );
    assert_eq!(
        layout.shift.get(&PhysicalPos::new(0, 0)),
        Some(&YabValue::Romaji {
            romaji: "A".to_string(),
            kana: None
        })
    );
}

// SpecialKey::to_vk テストは awase-windows に移動済み

// ── classify_section テスト ──

#[test]
fn test_classify_section() {
    assert_eq!(
        classify_section("ローマ字シフト無し"),
        Some(FaceKind::Normal)
    );
    assert_eq!(
        classify_section("ローマ字左親指シフト"),
        Some(FaceKind::LeftThumb)
    );
    assert_eq!(
        classify_section("ローマ字右親指シフト"),
        Some(FaceKind::RightThumb)
    );
    assert_eq!(
        classify_section("ローマ字小指シフト"),
        Some(FaceKind::Shift)
    );
    assert_eq!(
        classify_section("ローマ字小指左親指シフト"),
        Some(FaceKind::LeftThumbShift)
    );
    assert_eq!(
        classify_section("ローマ字小指右親指シフト"),
        Some(FaceKind::RightThumbShift)
    );
    assert_eq!(classify_section("unknown"), None);
    assert_eq!(classify_section(""), None);
}

// vk_to_pos テストは awase-windows に移動済み

// ── parse_face エラーパス ──

#[test]
fn test_parse_face_wrong_line_count() {
    let lines: Vec<String> = vec!["無".to_string(), "無".to_string()];
    assert!(parse_face(&lines, KeyboardModel::Jis).is_err());

    let lines5: Vec<String> = vec![
        "無".to_string(),
        "無".to_string(),
        "無".to_string(),
        "無".to_string(),
        "無".to_string(),
    ];
    assert!(parse_face(&lines5, KeyboardModel::Jis).is_err());
}

// ── YabLayout::parse 名前行テスト ──

#[test]
fn test_parse_layout_name_line() {
    let input = "\
テスト配列
[ローマ字シフト無し]
無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無";

    let layout = YabLayout::parse(input, KeyboardModel::Jis).unwrap();
    assert_eq!(layout.name, "テスト配列");
}

// ── セクション外データ行エラー ──

#[test]
fn test_parse_data_outside_section_error() {
    let input = "\
テスト配列
不明なデータ行";

    let result = YabLayout::parse(input, KeyboardModel::Jis);
    assert!(result.is_err());
}

// ── 一部セクションのみ ──

#[test]
fn test_parse_layout_missing_sections() {
    let input = "\
[ローマ字シフト無し]
ｋａ,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無";

    let layout = YabLayout::parse(input, KeyboardModel::Jis).unwrap();
    assert!(!layout.normal.is_empty());
    assert!(layout.left_thumb.is_empty());
    assert!(layout.right_thumb.is_empty());
    assert!(layout.shift.is_empty());
}

// ── 全角数字・記号の変換テスト ──

#[test]
fn test_fullwidth_digits_and_symbols() {
    // 全角数字はキーシーケンスになる
    assert_eq!(
        YabValue::parse("３"),
        YabValue::KeySequence("3".to_string())
    );
    assert_eq!(
        YabValue::parse("７"),
        YabValue::KeySequence("7".to_string())
    );
    // 全角記号もキーシーケンスになる
    assert_eq!(
        YabValue::parse("＃"),
        YabValue::KeySequence("#".to_string())
    );
    assert_eq!(
        YabValue::parse("＆"),
        YabValue::KeySequence("&".to_string())
    );
    // 全角の範囲外端の文字
    assert_eq!('～'.to_halfwidth_ascii(), Some('~')); // U+FF5E -> '~'
}

// ── 不明セクション名テスト ──

#[test]
fn test_parse_unknown_section_data_is_error() {
    // 不明セクション内のデータ行はセクション外扱いになりエラー
    let input = "\
[不明なセクション]
ｋａ,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無";

    let result = YabLayout::parse(input, KeyboardModel::Jis);
    assert!(result.is_err());
}

#[test]
fn test_parse_unknown_section_no_data_ok() {
    // 不明セクション直後に既知セクションが来る場合はOK
    let input = "\
[不明なセクション]
[ローマ字シフト無し]
ｋａ,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無";

    let layout = YabLayout::parse(input, KeyboardModel::Jis).unwrap();
    assert!(!layout.normal.is_empty());
}

#[test]
fn test_parse_yamabuki_thumb_shift_sections_are_loaded() {
    // やまぶきRのローマ字小指親指複合2面は実フェイスとして取り込む。
    let input = "\
[ローマ字シフト無し]
ｋａ,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無
[ローマ字小指左親指シフト]
ｚａ,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無
[ローマ字小指右親指シフト]
ｚｉ,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無";

    let layout = YabLayout::parse(input, KeyboardModel::Jis).unwrap();
    assert_eq!(
        layout.left_thumb_shift.get(&PhysicalPos::new(0, 0)),
        Some(&YabValue::Romaji {
            romaji: "za".to_string(),
            kana: None
        })
    );
    assert_eq!(
        layout.right_thumb_shift.get(&PhysicalPos::new(0, 0)),
        Some(&YabValue::Romaji {
            romaji: "zi".to_string(),
            kana: None
        })
    );
}

#[test]
fn test_parse_yamabuki_compat_sections_are_accepted_but_ignored() {
    // やまぶきRの英数系6面は、rust-nicolaのランタイムではまだ参照しない
    // （受理のみ）。データがあってもパースエラーにならないこと、かつ通常の4面には
    // 影響しないことを確認する。
    let input = "\
[ローマ字シフト無し]
ｋａ,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無
[英数シフト無し]
Ａ,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無
[英数左親指シフト]
無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無
[英数右親指シフト]
無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無
[英数小指シフト]
無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無
[英数小指左親指シフト]
無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無
[英数小指右親指シフト]
無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無";

    let layout = YabLayout::parse(input, KeyboardModel::Jis).unwrap();
    // 通常面はやまぶき互換セクションの影響を受けず正しくパースされる
    assert_eq!(
        layout.normal.get(&PhysicalPos::new(0, 0)),
        Some(&YabValue::Romaji {
            romaji: "ka".to_string(),
            kana: None
        })
    );
    assert!(layout.left_thumb.is_empty());
    assert!(layout.right_thumb.is_empty());
    assert!(layout.shift.is_empty());
    assert!(layout.left_thumb_shift.is_empty());
    assert!(layout.right_thumb_shift.is_empty());
}

#[test]
fn test_parse_tolerates_character_key_simultaneous_shift_blocks() {
    // やまぶきRの「文字キー同時打鍵シフト配列」（`<x>` + 4行ブロックの繰り返しが
    // 基本4行の後に続く拗音拡張ファイル等で使われる）は、rust-nicola のランタイムでは
    // まだ参照しない（受理のみ）。基本4行だけを取り込み、それ以降はエラーにせず無視する。
    let input = "\
[ローマ字シフト無し]
１,２,３,４,５,６,７,８,９,０,－,＜,＞
．,ｋａ,ｔａ,ｋｏ,ｓａ,ｒａ,ｔｉ,ｋｕ,ｔｕ,'，',，,゛
ｕ,ｓｉ,ｔｅ,ｋｅ,ｓｅ,ｈａ,ｔｏ,ｋｉ,ｉ,ｎｎ,後,逃
'．',ｈｉ,ｓｕ,ｆｕ,ｈｅ,ｍｅ,ｓｏ,ｎｅ,ｈｏ,'・','＼'
<r>
無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,ｃｈａ,ｋｕｌａ,ｔｕｌａ,ｐｙａ,無,無
無,無,無,無,無,無,無,ｋｙａ,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無
<h>
無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,ｒｙａ,無,無,無,無,無,無,無,無,無
無,ｓｈａ,無,無,無,無,無,無,無,無,無,無
無,ｈｙａ,無,ｆａ,無,無,無,無,無,無,無";

    let layout = YabLayout::parse(input, KeyboardModel::Jis).unwrap();
    // 基本4行はそのまま反映される（本家NICOLAと同じ「う」「ち」等）
    assert_eq!(
        layout.normal.get(&PhysicalPos::new(2, 0)),
        Some(&YabValue::Romaji {
            romaji: "u".to_string(),
            kana: None
        })
    );
    assert_eq!(
        layout.normal.get(&PhysicalPos::new(1, 6)),
        Some(&YabValue::Romaji {
            romaji: "ti".to_string(),
            kana: None
        })
    );
    // `<r>`/`<h>` ブロック（同時打鍵シフト面）の存在によって、定義していない
    // 他の面（左親指シフト等）にデータが混入したり、パースが壊れたりしない
    assert!(layout.left_thumb.is_empty());
    assert!(layout.right_thumb.is_empty());
    assert!(layout.shift.is_empty());
}

#[test]
fn test_parse_face_with_too_few_lines_still_errors() {
    // 同時打鍵シフトブロックを許容するようになっても、基本4行に満たない場合は
    // 引き続きエラーであるべき（無条件に許容しているわけではない）。
    let input = "\
[ローマ字シフト無し]
無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無";

    let result = YabLayout::parse(input, KeyboardModel::Jis);
    assert!(result.is_err());
}

#[test]
fn test_load_nicola_yab_file() {
    let path = std::path::Path::new("layout/nicola.yab");
    if !path.exists() {
        return; // Skip in CI
    }
    let content = std::fs::read_to_string(path).unwrap();
    let layout = YabLayout::parse(&content, KeyboardModel::Jis).unwrap();

    // Verify basic structure
    assert!(!layout.normal.is_empty());
    assert!(!layout.left_thumb.is_empty());
    assert!(!layout.right_thumb.is_empty());
    assert!(!layout.shift.is_empty());

    // Spot check: A key (row 2, col 0) in normal face should be "u" (う)
    let a_pos = PhysicalPos::new(2, 0);
    assert_eq!(
        layout.normal.get(&a_pos),
        Some(&YabValue::Romaji {
            romaji: "u".into(),
            kana: None
        })
    );

    // Spot check: A key in left thumb face should be "wo" (を)
    assert_eq!(
        layout.left_thumb.get(&a_pos),
        Some(&YabValue::Romaji {
            romaji: "wo".into(),
            kana: None
        })
    );
}

#[test]
fn test_nicola_keytop_yab_file_outputs_keytop_symbols_at_jis_extra_positions() {
    // 2026-08-31: report 01M15R86FJW24278GGD3ETS9QX（docs/bug-reports-triage.md
    // 参照）を機に、標準JISキーボードのキートップ印字通りの記号を出す版を
    // layout/nicola_keytop.yab として追加した（新規インストールの既定）。
    // layout/nicola.yab（Backspace/Escapeソフトウェア代用版）は既存ユーザーの
    // 設定・ファイルを無言で変えないよう内容を変更していない
    // （NeverOverwrite/Copy-IfAbsentで保護されるため、既定を変えても
    // アップグレードでは配布されない。Opusレビュー指摘）。
    let path = std::path::Path::new("layout/nicola_keytop.yab");
    if !path.exists() {
        return; // Skip in CI
    }
    let content = std::fs::read_to_string(path).unwrap();
    let layout = YabLayout::parse(&content, KeyboardModel::Jis).unwrap();

    let literal = |s: &str| Some(YabValue::Literal(s.to_string()));

    // 数字段（row0）12-13列目 = physical ^ / ¥ キー。全面共通。
    for face in [&layout.normal, &layout.left_thumb, &layout.right_thumb] {
        assert_eq!(face.get(&PhysicalPos::new(0, 11)).cloned(), literal("＾"));
        assert_eq!(face.get(&PhysicalPos::new(0, 12)).cloned(), literal("￥"));
    }

    // Q段（row1）11列目 = physical @ キー。シフト無し面は本家仕様の「、」を維持し、
    // 親指シフト面のみ未定義スロットに「＠」を割り当てる。
    assert_eq!(
        layout.normal.get(&PhysicalPos::new(1, 10)).cloned(),
        literal("、")
    );
    assert_eq!(
        layout.left_thumb.get(&PhysicalPos::new(1, 10)).cloned(),
        literal("＠")
    );
    assert_eq!(
        layout.right_thumb.get(&PhysicalPos::new(1, 10)).cloned(),
        literal("＠")
    );

    // Q段（row1）12列目 = physical [ キー。全面共通。
    for face in [&layout.normal, &layout.left_thumb, &layout.right_thumb] {
        assert_eq!(face.get(&PhysicalPos::new(1, 11)).cloned(), literal("［"));
    }

    // A段（row2）11-12列目 = physical : / ] キー。全面共通で、旧 Backspace/Escape
    // 代用（後/逃）を置き換えている。
    for face in [&layout.normal, &layout.left_thumb, &layout.right_thumb] {
        assert_eq!(face.get(&PhysicalPos::new(2, 10)).cloned(), literal("："));
        assert_eq!(face.get(&PhysicalPos::new(2, 11)).cloned(), literal("］"));
    }
}

#[test]
fn test_nicola_yab_still_uses_bs_esc_placeholders() {
    // layout/nicola.yab は既存ユーザーへ無言で挙動を変えないため、
    // Backspace/Escapeソフトウェア代用（後/逃）のまま維持している
    // （layout/nicola_keytop.yab が新規インストールの既定）。
    let path = std::path::Path::new("layout/nicola.yab");
    if !path.exists() {
        return; // Skip in CI
    }
    let content = std::fs::read_to_string(path).unwrap();
    let layout = YabLayout::parse(&content, KeyboardModel::Jis).unwrap();

    for face in [&layout.normal, &layout.left_thumb, &layout.right_thumb] {
        assert_eq!(
            face.get(&PhysicalPos::new(2, 10)),
            Some(&YabValue::Special(SpecialKey::Backspace))
        );
        assert_eq!(
            face.get(&PhysicalPos::new(2, 11)),
            Some(&YabValue::Special(SpecialKey::Escape))
        );
    }
}

#[test]
fn test_nicola_keytop_yab_does_not_reintroduce_bs_esc_placeholders() {
    // layout/nicola_keytop.yab に「後」「逃」（Backspace/Escapeソフトウェア
    // 代用）が将来の編集で再混入していないことを機械的に縛る。
    let path = std::path::Path::new("layout/nicola_keytop.yab");
    if !path.exists() {
        return; // Skip in CI
    }
    let content = std::fs::read_to_string(path).unwrap();
    assert!(
        !content.contains('後'),
        "nicola_keytop.yab should not contain the Backspace placeholder (後)"
    );
    assert!(
        !content.contains('逃'),
        "nicola_keytop.yab should not contain the Escape placeholder (逃)"
    );
}

#[test]
fn test_nicola_yab_and_nicola_keytop_yab_share_identical_kana_positions() {
    // layout/nicola.yab と layout/nicola_keytop.yab はNICOLA本家仕様のかな
    // 44キー配置を共有しているはず。記号の余りスロット（数字段12-13列目、
    // Q段11-12列目、A段11-12列目）以外で内容が乖離していないことを機械的に
    // 縛る（4ファイル体系での手作業コピーずれを検出するため、Opusレビュー指摘）。
    let nicola_path = std::path::Path::new("layout/nicola.yab");
    let keytop_path = std::path::Path::new("layout/nicola_keytop.yab");
    if !nicola_path.exists() || !keytop_path.exists() {
        return; // Skip in CI
    }
    let nicola = YabLayout::parse(
        &std::fs::read_to_string(nicola_path).unwrap(),
        KeyboardModel::Jis,
    )
    .unwrap();
    let keytop = YabLayout::parse(
        &std::fs::read_to_string(keytop_path).unwrap(),
        KeyboardModel::Jis,
    )
    .unwrap();

    // 記号スロットとして意図的に内容が異なる位置（row, col）。
    let exceptions: &[(u8, u8)] = &[
        (0, 11),
        (0, 12), // 数字段: ＾／￥ vs 無／無
        (1, 11), // Q段12列目: ［ vs 無
        (2, 10),
        (2, 11), // A段: ：／］ vs 後／逃
    ];

    for row in 0..4u8 {
        for col in 0..13u8 {
            if exceptions.contains(&(row, col)) {
                continue;
            }
            let pos = PhysicalPos::new(row, col);
            for (face_name, nicola_face, keytop_face) in [
                ("normal", &nicola.normal, &keytop.normal),
                ("left_thumb", &nicola.left_thumb, &keytop.left_thumb),
                ("right_thumb", &nicola.right_thumb, &keytop.right_thumb),
                ("shift", &nicola.shift, &keytop.shift),
            ] {
                // Q段11列目（物理@キー）は親指シフト面のみ意図的に異なる
                // （、→＠、シフト無し面は本家仕様の、を両ファイルとも維持）。
                if row == 1 && col == 10 && face_name != "normal" {
                    continue;
                }
                assert_eq!(
                    nicola_face.get(&pos),
                    keytop_face.get(&pos),
                    "{face_name} face differs at ({row}, {col})"
                );
            }
        }
    }
}

#[test]
fn test_load_nicola_us_yab_file() {
    let path = std::path::Path::new("layout/nicola_us.yab");
    if !path.exists() {
        return; // Skip in CI
    }
    let content = std::fs::read_to_string(path).unwrap();
    let layout = YabLayout::parse(&content, KeyboardModel::Us).unwrap();

    assert_eq!(layout.name, "NICOLA配列(US)");
    assert!(!layout.normal.is_empty());
    assert!(!layout.left_thumb.is_empty());
    assert!(!layout.right_thumb.is_empty());
    assert!(!layout.shift.is_empty());

    // JIS 版と同じ物理位置は同じ値を共有するはず（A キー = row2,col0 = "u"）
    let a_pos = PhysicalPos::new(2, 0);
    assert_eq!(
        layout.normal.get(&a_pos),
        Some(&YabValue::Romaji {
            romaji: "u".into(),
            kana: None
        })
    );

    // US 配列には無い JIS 拡張列（row2 の 12 番目 = col11）は存在しない
    let jis_only_pos = PhysicalPos::new(2, 11);
    assert_eq!(layout.normal.get(&jis_only_pos), None);
}

#[test]
fn test_load_nicola_f_yab_file() {
    let path = std::path::Path::new("layout/nicola_f.yab");
    if !path.exists() {
        return; // Skip in CI
    }
    let content = std::fs::read_to_string(path).unwrap();
    let layout = YabLayout::parse(&content, KeyboardModel::Jis).unwrap();

    // Verify basic structure
    assert!(!layout.normal.is_empty());
    assert!(!layout.left_thumb.is_empty());
    assert!(!layout.right_thumb.is_empty());
    assert!(!layout.shift.is_empty());

    // NICOLA-F配列（もどき）はローマ字ではなく仮名を直接リテラルで持つ。
    // 物理位置 (row2, col0) は NICOLA 本家と同じ「う」。
    let a_pos = PhysicalPos::new(2, 0);
    assert_eq!(
        layout.normal.get(&a_pos),
        Some(&YabValue::Literal("う".to_string()))
    );
    assert_eq!(
        layout.left_thumb.get(&a_pos),
        Some(&YabValue::Literal("を".to_string()))
    );

    let literal = |s: &str| Some(YabValue::Literal(s.to_string()));

    // 2026-08-31追記: 数字段12-13列目・Q段11-12列目は、この機種でも通常の
    // JIS記号キーが存在するため layout/nicola_keytop.yab と同じ記号を持つ
    // （Opusレビュー指摘、README.mdがこの機種向けに本ファイルを勧めているのに
    // 記号が欠落していた不具合の修正）。
    for face in [&layout.normal, &layout.left_thumb, &layout.right_thumb] {
        assert_eq!(face.get(&PhysicalPos::new(0, 11)).cloned(), literal("＾"));
        assert_eq!(face.get(&PhysicalPos::new(0, 12)).cloned(), literal("￥"));
        assert_eq!(face.get(&PhysicalPos::new(1, 11)).cloned(), literal("［"));
    }
    assert_eq!(
        layout.normal.get(&PhysicalPos::new(1, 10)).cloned(),
        literal("、")
    );
    assert_eq!(
        layout.left_thumb.get(&PhysicalPos::new(1, 10)).cloned(),
        literal("＠")
    );

    // A段（row2）11-12列目は、この機種の「後退」「取消」専用キーが物理的に
    // Backspace/Escapeのスキャンコードを出すため、ソフトウェア側では
    // 到達不能＝無のまま（ファイル冒頭コメント参照）。
    for face in [&layout.normal, &layout.left_thumb, &layout.right_thumb] {
        assert_eq!(face.get(&PhysicalPos::new(2, 10)), Some(&YabValue::None));
        assert_eq!(face.get(&PhysicalPos::new(2, 11)), Some(&YabValue::None));
    }
}

#[test]
fn test_load_nicola_kb232_yab_file() {
    // report 01M15R86FJW24278GGD3ETS9QX（富士通純正キーボード「FMV-KB232」、
    // docs/bug-reports-triage.md参照）で提供された、実機動作確認済みの配列。
    let path = std::path::Path::new("layout/nicola_kb232.yab");
    if !path.exists() {
        return; // Skip in CI
    }
    let content = std::fs::read_to_string(path).unwrap();
    let layout = YabLayout::parse(&content, KeyboardModel::Jis).unwrap();

    // BUG-95のクォート崩れ検出(yab::lint)に引っかからないこと。
    assert!(
        lint(&content).is_empty(),
        "nicola_kb232.yab should not trigger yab::lint warnings"
    );

    assert!(!layout.normal.is_empty());
    assert!(!layout.left_thumb.is_empty());
    assert!(!layout.right_thumb.is_empty());
    assert!(!layout.shift.is_empty());

    // nicola_f.yab と同じくローマ字ではなく仮名を直接リテラルで持つ形式。
    let a_pos = PhysicalPos::new(2, 0);
    assert_eq!(
        layout.normal.get(&a_pos),
        Some(&YabValue::Literal("う".to_string()))
    );

    let literal = |s: &str| Some(YabValue::Literal(s.to_string()));

    // KB232固有の記号配置。nicola_keytop.yab/nicola_f.yabのどちらとも一致しない
    // （NICOLA本家仕様で定義済みの「、」の位置自体がQ段11列目からA段11列目へ
    // 動いている等、単純な「余っているスロットへの記号追加」ではない）。
    assert_eq!(
        layout.normal.get(&PhysicalPos::new(0, 12)).cloned(),
        literal("￥")
    );
    assert_eq!(
        layout.normal.get(&PhysicalPos::new(0, 11)),
        Some(&YabValue::None)
    );
    assert_eq!(
        layout.normal.get(&PhysicalPos::new(1, 10)).cloned(),
        literal("＠")
    );
    assert_eq!(
        layout.normal.get(&PhysicalPos::new(1, 11)).cloned(),
        literal("［")
    );
    assert_eq!(
        layout.normal.get(&PhysicalPos::new(2, 10)).cloned(),
        literal("、")
    );
    assert_eq!(
        layout.normal.get(&PhysicalPos::new(2, 11)).cloned(),
        literal("］")
    );
    assert_eq!(
        layout.normal.get(&PhysicalPos::new(3, 10)).cloned(),
        literal("￥")
    );

    // かな44キー配置は layout/nicola.yab と完全一致するはず（表記形式
    // （リテラル vs ローマ字）が違うだけで物理位置ごとの意味は同じ）。
    // /code-review指摘（PR #132）: 以前は normal 面の一部位置だけを手書き
    // 列挙しており、left_thumb/right_thumb 面（濁音・半濁音等の残り約18キー）
    // が未検証だった。nicola.yab で Romaji として定義されている全位置を
    // 3面とも走査することで、44キー全体を機械的に網羅する。
    let nicola = YabLayout::parse(
        &std::fs::read_to_string("layout/nicola.yab").unwrap(),
        KeyboardModel::Jis,
    )
    .unwrap();
    let kana_table = KanaTable::build();
    let mut checked = 0;
    for (face_name, nicola_face, kb232_face) in [
        ("normal", &nicola.normal, &layout.normal),
        ("left_thumb", &nicola.left_thumb, &layout.left_thumb),
        ("right_thumb", &nicola.right_thumb, &layout.right_thumb),
    ] {
        for row in 0..4u8 {
            for col in 0..13u8 {
                let pos = PhysicalPos::new(row, col);
                let Some(YabValue::Romaji { romaji, .. }) = nicola_face.get(&pos) else {
                    continue; // かな以外(記号/無/Special)は本テストの対象外
                };
                let expected_kana = kana_table
                    .kana_for_romaji(romaji)
                    .unwrap_or_else(|| panic!("no kana mapping for romaji {romaji:?}"));
                match kb232_face.get(&pos) {
                    Some(YabValue::Literal(lit)) => {
                        assert_eq!(
                            lit.chars().next(),
                            Some(expected_kana),
                            "{face_name} face kana mismatch at ({row},{col})"
                        );
                        checked += 1;
                    }
                    other => panic!(
                        "{face_name} face: unexpected value at ({row},{col}) in \
                         nicola_kb232.yab: {other:?}"
                    ),
                }
            }
        }
    }
    // NICOLA本家の物理44キーは面ごとに異なる仮名を割り当てるため、
    // normal/left_thumb/right_thumb の合計は44より多くなる（実測81）。
    // ここでは「ループが実際に仮名セルを走査した」ことのサニティチェックのみ行う。
    assert_eq!(
        checked, 81,
        "kana cell count changed — verify nicola.yab wasn't edited unexpectedly"
    );
}

// ── to_fullwidth_str テスト ──

#[test]
fn test_halfwidth_to_fullwidth_alpha() {
    assert_eq!("ka".to_fullwidth_str(), "ｋａ");
    assert_eq!("si".to_fullwidth_str(), "ｓｉ");
    assert_eq!("A".to_fullwidth_str(), "Ａ");
    assert_eq!("Z".to_fullwidth_str(), "Ｚ");
}

#[test]
fn test_halfwidth_to_fullwidth_digits() {
    assert_eq!("123".to_fullwidth_str(), "１２３");
    assert_eq!("0".to_fullwidth_str(), "０");
}

#[test]
fn test_halfwidth_to_fullwidth_symbols() {
    assert_eq!("!".to_fullwidth_str(), "！");
    assert_eq!("#".to_fullwidth_str(), "＃");
    assert_eq!("~".to_fullwidth_str(), "～");
}

#[test]
fn test_halfwidth_to_fullwidth_empty() {
    assert_eq!("".to_fullwidth_str(), "");
}

// ── serialize_value テスト ──

#[test]
fn test_serialize_value_romaji() {
    let val = YabValue::Romaji {
        romaji: "ka".to_string(),
        kana: None,
    };
    assert_eq!(val.serialize(), "ｋａ");
}

#[test]
fn test_serialize_value_literal_unicode() {
    let val = YabValue::Literal("ー".to_string());
    assert_eq!(val.serialize(), "'ー'");
}

#[test]
fn test_serialize_value_literal_ascii_digit() {
    // Literal は常にクォート付き
    let val = YabValue::Literal("1".to_string());
    assert_eq!(val.serialize(), "'1'");
}

#[test]
fn test_serialize_value_literal_ascii_symbol() {
    // Literal は常にクォート付き
    let val = YabValue::Literal("!".to_string());
    assert_eq!(val.serialize(), "'!'");
}

#[test]
fn test_serialize_value_key_sequence_digit() {
    let val = YabValue::KeySequence("1".to_string());
    assert_eq!(val.serialize(), "１");
}

#[test]
fn test_serialize_value_key_sequence_symbol() {
    let val = YabValue::KeySequence("!".to_string());
    assert_eq!(val.serialize(), "！");
}

#[test]
fn test_serialize_value_special_keys() {
    assert_eq!(YabValue::Special(SpecialKey::Backspace).serialize(), "後");
    assert_eq!(YabValue::Special(SpecialKey::Escape).serialize(), "逃");
    assert_eq!(YabValue::Special(SpecialKey::Enter).serialize(), "入");
    assert_eq!(YabValue::Special(SpecialKey::Space).serialize(), "空");
    assert_eq!(YabValue::Special(SpecialKey::Delete).serialize(), "消");
    assert_eq!(YabValue::Special(SpecialKey::Insert).serialize(), "挿");
    assert_eq!(YabValue::Special(SpecialKey::Up).serialize(), "上");
    assert_eq!(YabValue::Special(SpecialKey::Left).serialize(), "左");
    assert_eq!(YabValue::Special(SpecialKey::Right).serialize(), "右");
    assert_eq!(YabValue::Special(SpecialKey::Down).serialize(), "下");
    assert_eq!(YabValue::Special(SpecialKey::Home).serialize(), "家");
    assert_eq!(YabValue::Special(SpecialKey::End).serialize(), "終");
    assert_eq!(YabValue::Special(SpecialKey::PageUp).serialize(), "前");
    assert_eq!(YabValue::Special(SpecialKey::PageDown).serialize(), "次");
}

#[test]
fn test_serialize_value_vk() {
    assert_eq!(YabValue::Vk(VkCode(0x1D)).serialize(), "V1D");
    assert_eq!(YabValue::Vk(VkCode(0x7C)).serialize(), "V7C");
}

#[test]
fn test_serialize_value_none() {
    assert_eq!(YabValue::None.serialize(), "無");
}

// ── serialize round-trip テスト ──

#[test]
fn test_serialize_round_trip_minimal() {
    let input = "\
テスト配列
[ローマ字シフト無し]
無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,ｋａ,無,無,無,無,無,無,無,無
無,ｓｉ,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無
[ローマ字左親指シフト]
無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無
[ローマ字右親指シフト]
無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無
[ローマ字小指シフト]
無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無";

    let model = KeyboardModel::Jis;
    let layout1 = YabLayout::parse(input, model).unwrap();
    let serialized = layout1.serialize(model);
    let layout2 = YabLayout::parse(&serialized, model).unwrap();

    // Compare key values
    assert_eq!(layout1.name, layout2.name);
    assert_eq!(
        layout1.normal.get(&PhysicalPos::new(1, 3)),
        layout2.normal.get(&PhysicalPos::new(1, 3))
    );
    assert_eq!(
        layout1.normal.get(&PhysicalPos::new(2, 1)),
        layout2.normal.get(&PhysicalPos::new(2, 1))
    );
}

#[test]
fn test_serialize_round_trip_all_variants() {
    let input = "\
テスト
[ローマ字シフト無し]
後,逃,入,空,消,'ー',ｋａ,１,！,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無
[ローマ字左親指シフト]
ｇａ,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無
[ローマ字右親指シフト]
ｍａ,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無
[ローマ字小指シフト]
Ａ,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無";

    let model = KeyboardModel::Jis;
    let layout1 = YabLayout::parse(input, model).unwrap();
    let serialized = layout1.serialize(model);
    let layout2 = YabLayout::parse(&serialized, model).unwrap();

    // Verify all variant types round-trip correctly
    let row0 = [
        (0, YabValue::Special(SpecialKey::Backspace)),
        (1, YabValue::Special(SpecialKey::Escape)),
        (2, YabValue::Special(SpecialKey::Enter)),
        (3, YabValue::Special(SpecialKey::Space)),
        (4, YabValue::Special(SpecialKey::Delete)),
        (5, YabValue::Literal("ー".to_string())),
        (
            6,
            YabValue::Romaji {
                romaji: "ka".to_string(),
                kana: None,
            },
        ),
        (7, YabValue::KeySequence("1".to_string())),
        (8, YabValue::KeySequence("!".to_string())),
    ];

    for (col, expected) in &row0 {
        let pos = PhysicalPos::new(0, *col as u8);
        assert_eq!(
            layout2.normal.get(&pos),
            Some(expected),
            "Mismatch at col {col}"
        );
    }

    // Check other faces
    assert_eq!(
        layout2.left_thumb.get(&PhysicalPos::new(0, 0)),
        Some(&YabValue::Romaji {
            romaji: "ga".to_string(),
            kana: None,
        })
    );
    assert_eq!(
        layout2.right_thumb.get(&PhysicalPos::new(0, 0)),
        Some(&YabValue::Romaji {
            romaji: "ma".to_string(),
            kana: None,
        })
    );
    assert_eq!(
        layout2.shift.get(&PhysicalPos::new(0, 0)),
        Some(&YabValue::Romaji {
            romaji: "A".to_string(),
            kana: None,
        })
    );
}

#[test]
fn test_serialize_omits_empty_thumb_shift_faces() {
    let input = "\
テスト
[ローマ字シフト無し]
無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無";

    let serialized = YabLayout::parse(input, KeyboardModel::Jis)
        .unwrap()
        .serialize(KeyboardModel::Jis);
    assert!(!serialized.contains("[ローマ字小指左親指シフト]"));
    assert!(!serialized.contains("[ローマ字小指右親指シフト]"));
}

#[test]
fn test_serialize_round_trip_non_empty_thumb_shift_faces() {
    let input = "\
テスト
[ローマ字シフト無し]
無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無
[ローマ字小指左親指シフト]
ｚａ,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無
[ローマ字小指右親指シフト]
ｚｉ,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無";

    let layout1 = YabLayout::parse(input, KeyboardModel::Jis).unwrap();
    let serialized = layout1.serialize(KeyboardModel::Jis);
    assert!(serialized.contains("[ローマ字小指左親指シフト]"));
    assert!(serialized.contains("[ローマ字小指右親指シフト]"));
    let layout2 = YabLayout::parse(&serialized, KeyboardModel::Jis).unwrap();
    assert_eq!(
        layout2.left_thumb_shift.get(&PhysicalPos::new(0, 0)),
        layout1.left_thumb_shift.get(&PhysicalPos::new(0, 0))
    );
    assert_eq!(
        layout2.right_thumb_shift.get(&PhysicalPos::new(0, 0)),
        layout1.right_thumb_shift.get(&PhysicalPos::new(0, 0))
    );
}

#[test]
fn test_serialize_round_trip_nicola_file() {
    let path = std::path::Path::new("layout/nicola.yab");
    if !path.exists() {
        return; // Skip in CI
    }
    let content = std::fs::read_to_string(path).unwrap();
    let model = KeyboardModel::Jis;
    let layout1 = YabLayout::parse(&content, model).unwrap();
    assert!(layout1.left_thumb_shift.is_empty());
    assert!(layout1.right_thumb_shift.is_empty());
    let serialized = layout1.serialize(model);
    let layout2 = YabLayout::parse(&serialized, model).unwrap();

    // Spot check several positions across faces
    for row in 0..4u8 {
        for col in 0..13u8 {
            let pos = PhysicalPos::new(row, col);
            assert_eq!(
                layout1.normal.get(&pos),
                layout2.normal.get(&pos),
                "normal mismatch at ({row}, {col})"
            );
            assert_eq!(
                layout1.left_thumb.get(&pos),
                layout2.left_thumb.get(&pos),
                "left_thumb mismatch at ({row}, {col})"
            );
            assert_eq!(
                layout1.right_thumb.get(&pos),
                layout2.right_thumb.get(&pos),
                "right_thumb mismatch at ({row}, {col})"
            );
            assert_eq!(
                layout1.shift.get(&pos),
                layout2.shift.get(&pos),
                "shift mismatch at ({row}, {col})"
            );
            assert_eq!(
                layout1.left_thumb_shift.get(&pos),
                layout2.left_thumb_shift.get(&pos),
                "left_thumb_shift mismatch at ({row}, {col})"
            );
            assert_eq!(
                layout1.right_thumb_shift.get(&pos),
                layout2.right_thumb_shift.get(&pos),
                "right_thumb_shift mismatch at ({row}, {col})"
            );
        }
    }
}

#[test]
fn test_serialize_comment_only_header_has_no_stray_name_line() {
    // 実運用の nicola.yab は先頭が `;` コメントのみで、明示的な名前行を持たない。
    // かつて process_yab_line がこのケースで最初のセクション名
    // （例: "ローマ字シフト無し"）を誤って `name` に採用していたため、
    // 設定 GUI の「名前をつけて保存」で書き出すと `[ローマ字シフト無し]` の
    // 直前に同名の余分な行が出力される不具合があった。
    let input = "\
;NICOLA配列
;http://nicola.sunicom.co.jp/spec/kikaku.htm

[ローマ字シフト無し]
無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無
[ローマ字左親指シフト]
無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無
[ローマ字右親指シフト]
無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無
[ローマ字小指シフト]
無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無";

    let model = KeyboardModel::Jis;
    let layout = YabLayout::parse(input, model).unwrap();
    assert_eq!(
        layout.name, "",
        "comment-only header must not leak into name"
    );

    let serialized = layout.serialize(model);
    assert!(
        serialized.starts_with("[ローマ字シフト無し]"),
        "serialized output must not have a stray name line before the first section, got: {serialized:?}"
    );

    // グループ間に空行を入れて読みやすくする。
    let expected_separator = "無\n\n[ローマ字左親指シフト]";
    assert!(
        serialized.contains(expected_separator),
        "expected a blank line between groups, got: {serialized:?}"
    );
}

// ── strip_paired_quote 境界値テスト ──

#[test]
fn parse_value_mismatched_single_quote_is_not_stripped() {
    // starts_with('\'') && ends_with('\'') の `&&` が `||` に壊れると、
    // 片方だけ一致した文字列も誤って引用符付きリテラルとして剥がされてしまう。
    assert_eq!(
        YabValue::parse("'abc"),
        YabValue::Literal("'abc".to_string())
    );
}

#[test]
fn parse_value_mismatched_double_quote_is_not_stripped() {
    assert_eq!(
        YabValue::parse("\"abc"),
        YabValue::Literal("\"abc".to_string())
    );
}

#[test]
fn parse_value_two_char_quote_pair_is_not_stripped() {
    // s.len() > 2 の `>` が `>=` に壊れると、中身が空の2文字（クォート2つのみ）も
    // 剥がされて空文字列になってしまう。中身が最低1文字必要なことを確認する。
    assert_eq!(YabValue::parse("''"), YabValue::Literal("''".to_string()));
    assert_eq!(
        YabValue::parse("\"\""),
        YabValue::Literal("\"\"".to_string())
    );
}

// ── is_all_fullwidth_ascii 境界値テスト ──

#[test]
fn parse_value_plain_ascii_is_literal_not_romaji() {
    // is_all_fullwidth_ascii の本体が無条件 true に置換される、または
    // `!self.is_empty() && ...` の `&&` が `||` に壊れると、非空文字列は
    // 中身に関わらず「全角のみ」と誤判定され、classify_fullwidth に回されて
    // Romaji/KeySequence になってしまう。
    assert_eq!(YabValue::parse("abc"), YabValue::Literal("abc".to_string()));
}

// ── process_yab_line: セクションヘッダ判定 ──

#[test]
fn test_process_yab_line_requires_both_bracket_ends_for_section_header() {
    // `line.starts_with('[') && line.ends_with(']')` の `&&` が `||` に壊れると、
    // 閉じ括弧の無い行も誤ってセクションヘッダとして扱われ、
    // `&line[1..line.len()-1]` で末尾の実際の文字を巻き込んで切り捨ててしまう。
    let input = "\
[abc
[ローマ字シフト無し]
無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無";

    let layout = YabLayout::parse(input, KeyboardModel::Jis).unwrap();
    assert_eq!(
        layout.name, "[abc",
        "line missing the closing ']' must be treated as a name line, not a section header"
    );
    assert!(
        !layout.normal.is_empty(),
        "the real section header on the next line must still be recognized"
    );
}

// ── YabFace: contains_key / len / values_mut / resolve_kana ──

#[test]
fn yab_face_contains_key_true_for_defined_false_for_undefined() {
    let mut face = YabFace::new();
    let pos = PhysicalPos::new(0, 0);
    assert!(!face.contains_key(&pos));

    face.insert(pos, YabValue::Literal("x".to_string()));
    assert!(face.contains_key(&pos));
    assert!(!face.contains_key(&PhysicalPos::new(1, 1)));
}

#[test]
fn yab_face_len_reflects_insert_count() {
    let mut face = YabFace::new();
    assert_eq!(face.len(), 0);

    face.insert(PhysicalPos::new(0, 0), YabValue::Literal("x".to_string()));
    assert_eq!(face.len(), 1);

    face.insert(PhysicalPos::new(0, 1), YabValue::Literal("y".to_string()));
    assert_eq!(face.len(), 2);
}

#[test]
fn yab_face_values_mut_iterates_all_defined_entries() {
    let mut face = YabFace::new();
    face.insert(
        PhysicalPos::new(0, 0),
        YabValue::Romaji {
            romaji: "ka".to_string(),
            kana: None,
        },
    );
    face.insert(
        PhysicalPos::new(0, 1),
        YabValue::Romaji {
            romaji: "si".to_string(),
            kana: None,
        },
    );
    assert_eq!(face.values_mut().count(), 2);
}

#[test]
fn yab_face_resolve_kana_populates_kana_field_for_romaji_values() {
    let table = KanaTable::build();
    let mut face = YabFace::new();
    let pos = PhysicalPos::new(0, 0);
    face.insert(
        pos,
        YabValue::Romaji {
            romaji: "ka".to_string(),
            kana: None,
        },
    );

    face.resolve_kana(&table);

    match face.get(&pos) {
        Some(YabValue::Romaji { kana, .. }) => {
            assert_eq!(
                *kana,
                Some('か'),
                "resolve_kana must fill in the kana field"
            );
        }
        other => panic!("expected Romaji, got {other:?}"),
    }
}

// ── ADR-115: 打鍵列機能 ──

#[test]
fn parse_ctrl_vk_recognizes_cv_prefix() {
    assert_eq!(
        YabValue::parse("CV4D"),
        YabValue::CtrlChord { vk: VkCode(0x4D), raw: "CV4D".to_string() }
    );
}

#[test]
fn parse_ctrl_vk_distinct_from_plain_vk() {
    // "V4D"（Ctrl無し）は既存の Vk 経路のまま。
    assert_eq!(YabValue::parse("V4D"), YabValue::Vk(VkCode(0x4D)));
}

#[test]
fn parse_ctrl_vk_rejects_non_hex() {
    // 認識できない場合は既存のフォールバックへ（Literal → 先頭1文字）。
    assert_eq!(YabValue::parse("CVXY"), YabValue::Literal("CVXY".to_string()));
}

#[test]
fn macro_ref_recognizes_at_prefix() {
    assert_eq!(
        YabValue::parse("@bracket_paren"),
        YabValue::MacroRef("bracket_paren".to_string())
    );
}

#[test]
fn macro_ref_allows_japanese_name() {
    assert_eq!(
        YabValue::parse("@括弧ペア"),
        YabValue::MacroRef("括弧ペア".to_string())
    );
}

#[test]
fn macro_ref_empty_name_falls_back_to_literal() {
    // "@" 単独は今日と同じ Literal フォールバック（レビュー指摘 M4）。
    assert_eq!(YabValue::parse("@"), YabValue::Literal("@".to_string()));
}

#[test]
fn split_unquoted_plus_splits_outside_quotes() {
    assert_eq!(split_unquoted_plus("'．'+CV4D"), vec!["'．'", "CV4D"]);
}

#[test]
fn split_unquoted_plus_ignores_plus_inside_quotes() {
    assert_eq!(split_unquoted_plus("'a+b'"), vec!["'a+b'"]);
}

#[test]
fn split_unquoted_plus_handles_escaped_quotes_in_layout_examples() {
    // 実レイアウトの [小指拡張親指シフト1] に実在する3形。
    assert_eq!(split_unquoted_plus(r#""\"""#), vec![r#""\"""#]);
    assert_eq!(split_unquoted_plus(r#""\'""#), vec![r#""\'""#]);
    assert_eq!(split_unquoted_plus(r"'\\'"), vec![r"'\\'"]);
}

#[test]
fn cell_segments_none_when_no_plus() {
    assert_eq!(cell_segments("'．'"), None);
    assert_eq!(cell_segments("CV4D"), None);
}

#[test]
fn cell_segments_none_for_degenerate_empty_segments() {
    // 先頭/末尾/連続する `+` は分割しない（レビュー指摘 Major1/M2）。
    assert_eq!(cell_segments("+CV4D"), None);
    assert_eq!(cell_segments("'あ'+"), None);
    assert_eq!(cell_segments("a++b"), None);
}

#[test]
fn parse_cell_single_segment_matches_plain_parse() {
    // + を含まないセルは今日と完全に同じ結果を返す。
    for raw in ["'あ'", "ｋａ", "CV4D", "無", "後"] {
        assert_eq!(parse_cell(raw), YabValue::parse(raw));
    }
}

#[test]
fn parse_cell_degenerate_plus_matches_plain_parse() {
    // 空セグメントを生む "+" 単独等は分割せず今日と同じ結果。
    for raw in ["+", "'あ'+", "+CV4D", "a++b"] {
        assert_eq!(parse_cell(raw), YabValue::parse(raw));
    }
}

#[test]
fn parse_cell_builds_inline_sequence_for_kuten_confirm() {
    let v = parse_cell("'．'+CV4D");
    match v {
        YabValue::InlineSequence { items, raw } => {
            assert_eq!(raw, "'．'+CV4D");
            assert_eq!(items, vec![
                YabValue::Literal("．".to_string()),
                YabValue::CtrlChord { vk: VkCode(0x4D), raw: "CV4D".to_string() },
            ]);
        }
        other => panic!("expected InlineSequence, got {other:?}"),
    }
}

#[test]
fn parse_cell_builds_inline_sequence_for_bracket_pair() {
    // Issue #118 の実例（5要素）。
    let v = parse_cell("'『'+CV4D+'』'+CV4D+左");
    match v {
        YabValue::InlineSequence { items, .. } => {
            assert_eq!(
                items,
                vec![
                    YabValue::Literal("『".to_string()),
                    YabValue::CtrlChord { vk: VkCode(0x4D), raw: "CV4D".to_string() },
                    YabValue::Literal("』".to_string()),
                    YabValue::CtrlChord { vk: VkCode(0x4D), raw: "CV4D".to_string() },
                    YabValue::Special(SpecialKey::Left),
                ]
            );
        }
        other => panic!("expected InlineSequence, got {other:?}"),
    }
}

#[test]
fn parse_cell_allows_macro_ref_segment() {
    let v = parse_cell("'。'+@confirm");
    match v {
        YabValue::InlineSequence { items, .. } => {
            assert_eq!(
                items,
                vec![
                    YabValue::Literal("。".to_string()),
                    YabValue::MacroRef("confirm".to_string()),
                ]
            );
        }
        other => panic!("expected InlineSequence, got {other:?}"),
    }
}

#[test]
fn serialize_ctrl_chord_and_inline_sequence_round_trip_via_raw() {
    for raw in ["CV4D", "CV0D", "'．'+CV4D", "'『'+CV4D+'』'+CV4D+左"] {
        let parsed = parse_cell(raw);
        assert_eq!(parsed.serialize(), raw, "raw round-trip must be byte-exact for {raw:?}");
    }
}

#[test]
fn serialize_macro_ref_reconstructs_at_name() {
    assert_eq!(YabValue::MacroRef("bracket_paren".to_string()).serialize(), "@bracket_paren");
}

#[test]
fn lint_raw_cell_regression_unaffected_by_new_syntax() {
    // 既存のクォート不整合誤字検出（report 01M13EACMQ7D2VETW75N0BTZ9C）が
    // 新構文追加後も変化しないこと。
    assert!(YabValue::lint_raw_cell("ｂ'ｕ").is_some());
    assert!(YabValue::lint_raw_cell("ｂｕ").is_none());
    assert!(YabValue::lint_raw_cell("CV4D").is_none());
}

#[test]
fn lint_detects_typo_inside_plus_joined_segment() {
    // `+` 区切りのどのセグメントに誤字があっても検出される。
    let warnings = lint("[ローマ字シフト無し]\nｂ'ｕ+CV4D,無,無,無,無,無,無,無,無,無,無,無,無\n無,無,無,無,無,無,無,無,無,無,無,無,無\n無,無,無,無,無,無,無,無,無,無,無,無,無\n無,無,無,無,無,無,無,無,無,無,無,無,無\n");
    assert!(!warnings.is_empty(), "typo inside a + segment must still be detected");
}

#[test]
fn resolve_kana_descends_into_inline_sequence() {
    let table = KanaTable::build();
    let mut face = YabFace::new();
    let pos = PhysicalPos::new(0, 0);
    face.insert(pos, parse_cell("ｋａ+CV4D"));
    face.resolve_kana(&table);
    match face.get(&pos) {
        Some(YabValue::InlineSequence { items, .. }) => match &items[0] {
            YabValue::Romaji { kana, .. } => assert_eq!(*kana, Some('か')),
            other => panic!("expected Romaji, got {other:?}"),
        },
        other => panic!("expected InlineSequence, got {other:?}"),
    }
}

// ── ADR-115: resolve_keystroke_syntax ──

fn km(name: &str, steps: &[&str]) -> crate::config::KeystrokeMacro {
    crate::config::KeystrokeMacro {
        name: name.to_string(),
        steps: steps.iter().map(|s| (*s).to_string()).collect(),
    }
}

fn layout_with(pos: PhysicalPos, value: YabValue) -> YabLayout {
    let mut face = YabFace::new();
    face.insert(pos, value);
    YabLayout {
        name: "test".to_string(),
        normal: face,
        left_thumb: YabFace::new(),
        right_thumb: YabFace::new(),
        shift: YabFace::new(),
        left_thumb_shift: YabFace::new(),
        right_thumb_shift: YabFace::new(),
    }
}

#[test]
fn resolve_off_ctrl_chord_reverts_to_today_literal() {
    let pos = PhysicalPos::new(0, 0);
    let layout = layout_with(pos, parse_cell("CV41"));
    let (resolved, warnings) = resolve_keystroke_syntax(
        layout,
        &[],
        crate::config::KeystrokeSequencePolicy::Off,
    );
    assert!(warnings.is_empty());
    assert_eq!(resolved.normal.get(&pos), Some(&YabValue::Literal("CV41".to_string())));
}

#[test]
fn resolve_off_inline_sequence_reverts_to_today_parse_result() {
    let pos = PhysicalPos::new(0, 0);
    let layout = layout_with(pos, parse_cell("'．'+CV4D"));
    let (resolved, _warnings) = resolve_keystroke_syntax(
        layout,
        &[],
        crate::config::KeystrokeSequencePolicy::Off,
    );
    // 今日の YabValue::parse("'．'+CV4D") と完全に一致すること。
    assert_eq!(resolved.normal.get(&pos), Some(&YabValue::parse("'．'+CV4D")));
}

#[test]
fn resolve_off_inline_sequence_matches_today_quote_stripping_edge_case() {
    // 実装タスクレビュー指摘 M2: raw 全体が同じクォート文字で始まり
    // 終わるケースで、今日の strip_paired_quote 挙動と一致すること。
    let pos = PhysicalPos::new(0, 0);
    let raw = "'（'+'）'";
    let layout = layout_with(pos, parse_cell(raw));
    let (resolved, _warnings) = resolve_keystroke_syntax(
        layout,
        &[],
        crate::config::KeystrokeSequencePolicy::Off,
    );
    assert_eq!(resolved.normal.get(&pos), Some(&YabValue::parse(raw)));
}

#[test]
fn resolve_off_macro_ref_reverts_to_at_name_literal() {
    let pos = PhysicalPos::new(0, 0);
    let layout = layout_with(pos, parse_cell("@confirm"));
    let (resolved, warnings) = resolve_keystroke_syntax(
        layout,
        &[km("confirm", &["CV4D"])],
        crate::config::KeystrokeSequencePolicy::Off,
    );
    assert!(warnings.is_empty());
    assert_eq!(resolved.normal.get(&pos), Some(&YabValue::Literal("@confirm".to_string())));
}

#[test]
fn resolve_on_ctrl_chord_stays_ctrl_chord() {
    let pos = PhysicalPos::new(0, 0);
    let layout = layout_with(pos, parse_cell("CV4D"));
    let (resolved, warnings) = resolve_keystroke_syntax(
        layout,
        &[],
        crate::config::KeystrokeSequencePolicy::On,
    );
    assert!(warnings.is_empty());
    assert_eq!(
        resolved.normal.get(&pos),
        Some(&YabValue::CtrlChord { vk: VkCode(0x4D), raw: "CV4D".to_string() })
    );
}

#[test]
fn resolve_on_inline_sequence_becomes_flat_sequence() {
    let pos = PhysicalPos::new(0, 0);
    let layout = layout_with(pos, parse_cell("'．'+CV4D"));
    let (resolved, warnings) = resolve_keystroke_syntax(
        layout,
        &[],
        crate::config::KeystrokeSequencePolicy::On,
    );
    assert!(warnings.is_empty());
    assert_eq!(
        resolved.normal.get(&pos),
        Some(&YabValue::Sequence(vec![
            YabValue::Literal("．".to_string()),
            YabValue::CtrlChord { vk: VkCode(0x4D), raw: "CV4D".to_string() },
        ]))
    );
}

#[test]
fn resolve_on_rejects_vk_inside_inline_sequence_with_warning() {
    // 実装タスクレビュー/r5レビュー Critical C1 の回帰: Vk は InlineSequence
    // 経由でも stuck key を招くため必ず拒否される。
    let pos = PhysicalPos::new(0, 0);
    let layout = layout_with(pos, parse_cell("'あ'+V1D"));
    let (resolved, warnings) = resolve_keystroke_syntax(
        layout,
        &[],
        crate::config::KeystrokeSequencePolicy::On,
    );
    assert!(!warnings.is_empty(), "Vk element must produce a warning");
    // Vk が拒否されて Literal("あ") だけが残る → 1要素なので Sequence で
    // 包まずそのまま返る。
    assert_eq!(resolved.normal.get(&pos), Some(&YabValue::Literal("あ".to_string())));
}

#[test]
fn resolve_on_macro_ref_inside_inline_sequence_flattens_without_nesting() {
    // 実装タスクレビュー/r5レビュー Critical C2 の回帰: InlineSequence 内の
    // MacroRef はマクロの steps を Sequence で包まず平坦に展開する。
    let pos = PhysicalPos::new(0, 0);
    let layout = layout_with(pos, parse_cell("'。'+@confirm"));
    let (resolved, warnings) = resolve_keystroke_syntax(
        layout,
        &[km("confirm", &["CV4D"])],
        crate::config::KeystrokeSequencePolicy::On,
    );
    assert!(warnings.is_empty());
    match resolved.normal.get(&pos) {
        Some(YabValue::Sequence(items)) => {
            assert_eq!(items.len(), 2, "must be flat, not nested: {items:?}");
            for it in items {
                assert!(!matches!(it, YabValue::Sequence(_)), "found nested Sequence: {it:?}");
            }
        }
        other => panic!("expected flat Sequence, got {other:?}"),
    }
}

#[test]
fn resolve_on_undefined_macro_ref_becomes_none_with_warning() {
    let pos = PhysicalPos::new(0, 0);
    let layout = layout_with(pos, parse_cell("@typo"));
    let (resolved, warnings) = resolve_keystroke_syntax(
        layout,
        &[],
        crate::config::KeystrokeSequencePolicy::On,
    );
    assert!(warnings.iter().any(|w| w.contains("@typo")));
    assert_eq!(resolved.normal.get(&pos), Some(&YabValue::None));
}

#[test]
fn resolve_on_all_elements_rejected_collapses_to_none() {
    // V1D+V1C は全要素が Vk で拒否される → Sequence(vec![]) ではなく None。
    let pos = PhysicalPos::new(0, 0);
    let layout = layout_with(pos, parse_cell("V1D+V1C"));
    let (resolved, warnings) = resolve_keystroke_syntax(
        layout,
        &[],
        crate::config::KeystrokeSequencePolicy::On,
    );
    assert_eq!(warnings.len(), 2);
    assert_eq!(resolved.normal.get(&pos), Some(&YabValue::None));
}

#[test]
fn resolve_on_empty_macro_steps_collapses_to_none() {
    let pos = PhysicalPos::new(0, 0);
    let layout = layout_with(pos, parse_cell("@empty"));
    let (resolved, _warnings) = resolve_keystroke_syntax(
        layout,
        &[km("empty", &[])],
        crate::config::KeystrokeSequencePolicy::On,
    );
    assert_eq!(resolved.normal.get(&pos), Some(&YabValue::None));
}

#[test]
fn resolve_on_macro_rejects_romaji_step_with_alternative_hint() {
    let pos = PhysicalPos::new(0, 0);
    let layout = layout_with(pos, parse_cell("@confirm"));
    let (resolved, warnings) = resolve_keystroke_syntax(
        layout,
        &[km("confirm", &["ｋａ", "CV4D"])],
        crate::config::KeystrokeSequencePolicy::On,
    );
    assert!(warnings.iter().any(|w| w.contains("+") && w.contains("ローマ字")));
    // "ｋａ" が拒否されて "CV4D" だけが残る → 1要素なのでそのまま返る。
    assert_eq!(
        resolved.normal.get(&pos),
        Some(&YabValue::CtrlChord { vk: VkCode(0x4D), raw: "CV4D".to_string() })
    );
}
