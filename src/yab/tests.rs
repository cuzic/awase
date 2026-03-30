use super::*;
use crate::types::VkCode;

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
    assert_eq!(parse_value("１"), YabValue::Literal("1".to_string()));
    assert_eq!(parse_value("２"), YabValue::Literal("2".to_string()));
}

#[test]
fn parse_value_fullwidth_symbol() {
    assert_eq!(parse_value("！"), YabValue::Literal("!".to_string()));
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

    let layout = YabLayout::parse(input).unwrap();

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

    let layout = YabLayout::parse(input).unwrap();

    // 通常面の検証
    assert_eq!(
        layout.normal.get(&PhysicalPos::new(0, 0)),
        Some(&YabValue::Literal("1".to_string()))
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

    let layout = YabLayout::parse(input).unwrap();

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
    let result = YabLayout::parse(input);
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
    let result = YabLayout::parse(input);
    assert!(result.is_err());
}

#[test]
fn parse_empty_sections_ok() {
    let input = "; コメントのみ";
    let layout = YabLayout::parse(input).unwrap();
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

    let layout = YabLayout::parse(input).unwrap();
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

    let layout = YabLayout::parse(input).unwrap();

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

// ── SpecialKey::to_vk テスト ──

#[test]
fn test_special_key_to_vk() {
    assert_eq!(SpecialKey::Backspace.to_vk(), VkCode(0x08));
    assert_eq!(SpecialKey::Escape.to_vk(), VkCode(0x1B));
    assert_eq!(SpecialKey::Enter.to_vk(), VkCode(0x0D));
    assert_eq!(SpecialKey::Space.to_vk(), VkCode(0x20));
    assert_eq!(SpecialKey::Delete.to_vk(), VkCode(0x2E));
}

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

// ── vk_to_pos テスト ──

#[test]
fn test_vk_to_pos_number_row() {
    // Row 0: number row
    assert_eq!(vk_to_pos(0x31), Some(PhysicalPos::new(0, 0))); // 1
    assert_eq!(vk_to_pos(0x35), Some(PhysicalPos::new(0, 4))); // 5
    assert_eq!(vk_to_pos(0x30), Some(PhysicalPos::new(0, 9))); // 0
    assert_eq!(vk_to_pos(0xBD), Some(PhysicalPos::new(0, 10))); // -
    assert_eq!(vk_to_pos(0xDE), Some(PhysicalPos::new(0, 11))); // ^
    assert_eq!(vk_to_pos(0xDC), Some(PhysicalPos::new(0, 12))); // ¥
}

#[test]
fn test_vk_to_pos_q_row() {
    // Row 1: Q row
    assert_eq!(vk_to_pos(0x51), Some(PhysicalPos::new(1, 0))); // Q
    assert_eq!(vk_to_pos(0x59), Some(PhysicalPos::new(1, 5))); // Y
    assert_eq!(vk_to_pos(0xDB), Some(PhysicalPos::new(1, 11))); // [
}

#[test]
fn test_vk_to_pos_a_row() {
    // Row 2: A row
    assert_eq!(vk_to_pos(0x41), Some(PhysicalPos::new(2, 0))); // A
    assert_eq!(vk_to_pos(0x48), Some(PhysicalPos::new(2, 5))); // H
    assert_eq!(vk_to_pos(0xDD), Some(PhysicalPos::new(2, 11))); // ]
}

#[test]
fn test_vk_to_pos_z_row() {
    // Row 3: Z row
    assert_eq!(vk_to_pos(0x5A), Some(PhysicalPos::new(3, 0))); // Z
    assert_eq!(vk_to_pos(0x4E), Some(PhysicalPos::new(3, 5))); // N
    assert_eq!(vk_to_pos(0xE2), Some(PhysicalPos::new(3, 10))); // _
}

#[test]
fn test_vk_to_pos_all_codes() {
    // Exhaustively test every mapped VK code to ensure full coverage
    let expected: &[(u16, u8, u8)] = &[
        // Row 0: number row
        (0x31, 0, 0),
        (0x32, 0, 1),
        (0x33, 0, 2),
        (0x34, 0, 3),
        (0x35, 0, 4),
        (0x36, 0, 5),
        (0x37, 0, 6),
        (0x38, 0, 7),
        (0x39, 0, 8),
        (0x30, 0, 9),
        (0xBD, 0, 10),
        (0xDE, 0, 11),
        (0xDC, 0, 12),
        // Row 1: Q row
        (0x51, 1, 0),
        (0x57, 1, 1),
        (0x45, 1, 2),
        (0x52, 1, 3),
        (0x54, 1, 4),
        (0x59, 1, 5),
        (0x55, 1, 6),
        (0x49, 1, 7),
        (0x4F, 1, 8),
        (0x50, 1, 9),
        (0xC0, 1, 10),
        (0xDB, 1, 11),
        // Row 2: A row
        (0x41, 2, 0),
        (0x53, 2, 1),
        (0x44, 2, 2),
        (0x46, 2, 3),
        (0x47, 2, 4),
        (0x48, 2, 5),
        (0x4A, 2, 6),
        (0x4B, 2, 7),
        (0x4C, 2, 8),
        (0xBB, 2, 9),
        (0xBA, 2, 10),
        (0xDD, 2, 11),
        // Row 3: Z row
        (0x5A, 3, 0),
        (0x58, 3, 1),
        (0x43, 3, 2),
        (0x56, 3, 3),
        (0x42, 3, 4),
        (0x4E, 3, 5),
        (0x4D, 3, 6),
        (0xBC, 3, 7),
        (0xBE, 3, 8),
        (0xBF, 3, 9),
        (0xE2, 3, 10),
    ];

    for &(vk, row, col) in expected {
        assert_eq!(
            vk_to_pos(vk),
            Some(PhysicalPos::new(row, col)),
            "VK 0x{vk:02X} should map to ({row}, {col})"
        );
    }

    // Unknown VK codes
    assert_eq!(vk_to_pos(0x00), None);
    assert_eq!(vk_to_pos(0xFF), None);
    assert_eq!(vk_to_pos(0x10), None); // Shift key
}

// ── parse_face エラーパス ──

#[test]
fn test_parse_face_wrong_line_count() {
    let lines: Vec<String> = vec!["無".to_string(), "無".to_string()];
    assert!(parse_face(&lines).is_err());

    let lines5: Vec<String> = vec![
        "無".to_string(),
        "無".to_string(),
        "無".to_string(),
        "無".to_string(),
        "無".to_string(),
    ];
    assert!(parse_face(&lines5).is_err());
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

    let layout = YabLayout::parse(input).unwrap();
    assert_eq!(layout.name, "テスト配列");
}

// ── セクション外データ行エラー ──

#[test]
fn test_parse_data_outside_section_error() {
    let input = "\
テスト配列
不明なデータ行";

    let result = YabLayout::parse(input);
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

    let layout = YabLayout::parse(input).unwrap();
    assert!(!layout.normal.is_empty());
    assert!(layout.left_thumb.is_empty());
    assert!(layout.right_thumb.is_empty());
    assert!(layout.shift.is_empty());
}

// ── 全角数字・記号の変換テスト ──

#[test]
fn test_fullwidth_digits_and_symbols() {
    // 全角数字はリテラルになる
    assert_eq!(parse_value("３"), YabValue::Literal("3".to_string()));
    assert_eq!(parse_value("７"), YabValue::Literal("7".to_string()));
    // 全角記号もリテラルになる
    assert_eq!(parse_value("＃"), YabValue::Literal("#".to_string()));
    assert_eq!(parse_value("＆"), YabValue::Literal("&".to_string()));
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

    let result = YabLayout::parse(input);
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

    let layout = YabLayout::parse(input).unwrap();
    assert!(!layout.normal.is_empty());
}

#[test]
fn test_load_nicola_yab_file() {
    let path = std::path::Path::new("layout/nicola.yab");
    if !path.exists() {
        return; // Skip in CI
    }
    let content = std::fs::read_to_string(path).unwrap();
    let layout = YabLayout::parse(&content).unwrap();

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
