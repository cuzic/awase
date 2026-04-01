use super::*;

// ── 全角→半角変換テスト ──

#[test]
fn fullwidth_alpha_to_halfwidth() {
    assert_eq!(fullwidth_to_halfwidth('ａ'), Some('a'));
    assert_eq!(fullwidth_to_halfwidth('ｚ'), Some('z'));
    assert_eq!(fullwidth_to_halfwidth('Ａ'), Some('A'));
    assert_eq!(fullwidth_to_halfwidth('Ｚ'), Some('Z'));
}

#[test]
fn fullwidth_digit_to_halfwidth() {
    assert_eq!(fullwidth_to_halfwidth('０'), Some('0'));
    assert_eq!(fullwidth_to_halfwidth('９'), Some('9'));
}

#[test]
fn fullwidth_symbol_to_halfwidth() {
    assert_eq!(fullwidth_to_halfwidth('！'), Some('!'));
    assert_eq!(fullwidth_to_halfwidth('？'), Some('?'));
    assert_eq!(fullwidth_to_halfwidth('＃'), Some('#'));
}

#[test]
fn non_fullwidth_returns_none() {
    assert_eq!(fullwidth_to_halfwidth('a'), None);
    assert_eq!(fullwidth_to_halfwidth('あ'), None);
}

#[test]
fn fullwidth_string_conversion() {
    assert_eq!(convert_fullwidth_str("ｋａ"), "ka");
    assert_eq!(convert_fullwidth_str("ｓｉ"), "si");
    assert_eq!(convert_fullwidth_str("Ａ"), "A");
    assert_eq!(convert_fullwidth_str("１２３"), "123");
}

// ── parse_value テスト ──

#[test]
fn parse_value_none() {
    assert_eq!(parse_value("無"), YabValue::None);
    assert_eq!(parse_value(""), YabValue::None);
    assert_eq!(parse_value("  "), YabValue::None);
}

#[test]
fn parse_value_special_keys() {
    assert_eq!(parse_value("後"), YabValue::Special(SpecialKey::Backspace));
    assert_eq!(parse_value("逃"), YabValue::Special(SpecialKey::Escape));
    assert_eq!(parse_value("入"), YabValue::Special(SpecialKey::Enter));
    assert_eq!(parse_value("空"), YabValue::Special(SpecialKey::Space));
    assert_eq!(parse_value("消"), YabValue::Special(SpecialKey::Delete));
}

#[test]
fn parse_value_single_quoted_literal() {
    assert_eq!(parse_value("'．'"), YabValue::Literal("．".to_string()));
    assert_eq!(parse_value("'ー'"), YabValue::Literal("ー".to_string()));
}

#[test]
fn parse_value_fullwidth_romaji() {
    assert_eq!(
        parse_value("ｋａ"),
        YabValue::Romaji {
            romaji: "ka".to_string(),
            kana: None
        }
    );
    assert_eq!(
        parse_value("ｓｉ"),
        YabValue::Romaji {
            romaji: "si".to_string(),
            kana: None
        }
    );
    assert_eq!(
        parse_value("ｗｏ"),
        YabValue::Romaji {
            romaji: "wo".to_string(),
            kana: None
        }
    );
}

#[test]
fn parse_value_fullwidth_uppercase() {
    assert_eq!(
        parse_value("Ａ"),
        YabValue::Romaji {
            romaji: "A".to_string(),
            kana: None
        }
    );
    assert_eq!(
        parse_value("Ｂ"),
        YabValue::Romaji {
            romaji: "B".to_string(),
            kana: None
        }
    );
}

#[test]
fn parse_value_fullwidth_digit() {
    assert_eq!(parse_value("１"), YabValue::KeySequence("1".to_string()));
    assert_eq!(parse_value("２"), YabValue::KeySequence("2".to_string()));
}

#[test]
fn parse_value_fullwidth_symbol() {
    assert_eq!(parse_value("！"), YabValue::KeySequence("!".to_string()));
}

#[test]
fn parse_value_double_quoted_literal() {
    assert_eq!(parse_value("\"？\""), YabValue::Literal("？".to_string()));
    assert_eq!(parse_value("\"！\""), YabValue::Literal("！".to_string()));
}

#[test]
fn parse_value_key_sequence_round_trip() {
    let val = YabValue::KeySequence("?".to_string());
    let serialized = serialize_value(&val);
    assert_eq!(serialized, "？");
    let parsed = parse_value(&serialized);
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
    // 無のキーは含まれないこと
    assert_eq!(layout.normal.get(&PhysicalPos::new(0, 0)), None);
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
    assert_eq!(parse_value("３"), YabValue::KeySequence("3".to_string()));
    assert_eq!(parse_value("７"), YabValue::KeySequence("7".to_string()));
    // 全角記号もキーシーケンスになる
    assert_eq!(parse_value("＃"), YabValue::KeySequence("#".to_string()));
    assert_eq!(parse_value("＆"), YabValue::KeySequence("&".to_string()));
    // 全角の範囲外端の文字
    assert_eq!(fullwidth_to_halfwidth('～'), Some('~')); // U+FF5E -> '~'
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

// ── halfwidth_to_fullwidth テスト ──

#[test]
fn test_halfwidth_to_fullwidth_alpha() {
    assert_eq!(halfwidth_to_fullwidth("ka"), "ｋａ");
    assert_eq!(halfwidth_to_fullwidth("si"), "ｓｉ");
    assert_eq!(halfwidth_to_fullwidth("A"), "Ａ");
    assert_eq!(halfwidth_to_fullwidth("Z"), "Ｚ");
}

#[test]
fn test_halfwidth_to_fullwidth_digits() {
    assert_eq!(halfwidth_to_fullwidth("123"), "１２３");
    assert_eq!(halfwidth_to_fullwidth("0"), "０");
}

#[test]
fn test_halfwidth_to_fullwidth_symbols() {
    assert_eq!(halfwidth_to_fullwidth("!"), "！");
    assert_eq!(halfwidth_to_fullwidth("#"), "＃");
    assert_eq!(halfwidth_to_fullwidth("~"), "～");
}

#[test]
fn test_halfwidth_to_fullwidth_empty() {
    assert_eq!(halfwidth_to_fullwidth(""), "");
}

// ── serialize_value テスト ──

#[test]
fn test_serialize_value_romaji() {
    let val = YabValue::Romaji {
        romaji: "ka".to_string(),
        kana: None,
    };
    assert_eq!(serialize_value(&val), "ｋａ");
}

#[test]
fn test_serialize_value_literal_unicode() {
    let val = YabValue::Literal("ー".to_string());
    assert_eq!(serialize_value(&val), "'ー'");
}

#[test]
fn test_serialize_value_literal_ascii_digit() {
    // Literal は常にクォート付き
    let val = YabValue::Literal("1".to_string());
    assert_eq!(serialize_value(&val), "'1'");
}

#[test]
fn test_serialize_value_literal_ascii_symbol() {
    // Literal は常にクォート付き
    let val = YabValue::Literal("!".to_string());
    assert_eq!(serialize_value(&val), "'!'");
}

#[test]
fn test_serialize_value_key_sequence_digit() {
    let val = YabValue::KeySequence("1".to_string());
    assert_eq!(serialize_value(&val), "１");
}

#[test]
fn test_serialize_value_key_sequence_symbol() {
    let val = YabValue::KeySequence("!".to_string());
    assert_eq!(serialize_value(&val), "！");
}

#[test]
fn test_serialize_value_special_keys() {
    assert_eq!(
        serialize_value(&YabValue::Special(SpecialKey::Backspace)),
        "後"
    );
    assert_eq!(
        serialize_value(&YabValue::Special(SpecialKey::Escape)),
        "逃"
    );
    assert_eq!(serialize_value(&YabValue::Special(SpecialKey::Enter)), "入");
    assert_eq!(serialize_value(&YabValue::Special(SpecialKey::Space)), "空");
    assert_eq!(
        serialize_value(&YabValue::Special(SpecialKey::Delete)),
        "消"
    );
}

#[test]
fn test_serialize_value_none() {
    assert_eq!(serialize_value(&YabValue::None), "無");
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
fn test_serialize_round_trip_nicola_file() {
    let path = std::path::Path::new("layout/nicola.yab");
    if !path.exists() {
        return; // Skip in CI
    }
    let content = std::fs::read_to_string(path).unwrap();
    let model = KeyboardModel::Jis;
    let layout1 = YabLayout::parse(&content, model).unwrap();
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
        }
    }
}
