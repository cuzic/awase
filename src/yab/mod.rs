use std::collections::HashMap;

use anyhow::{bail, Context, Result};

use crate::kana_table::build_romaji_to_kana;
use crate::scanmap::PhysicalPos;

/// .yab ファイルからパースされた値
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YabValue {
    /// ローマ字文字列（VK コードで送信する）（例: "ka", "si", "wo"）
    /// `kana` にはパース時に逆引きした仮名文字を保持する。
    /// 拗音など単一 `char` に収まらないローマ字の場合は `None`。
    Romaji { romaji: String, kana: Option<char> },
    /// リテラル文字（`KEYEVENTF_UNICODE` で送信する）
    Literal(String),
    /// 特殊キー
    Special(SpecialKey),
    /// 割り当てなし（パススルー）
    None,
}

/// 特殊キーの種別
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialKey {
    /// 後 (Backspace)
    Backspace,
    /// 逃 (Escape)
    Escape,
    /// 入 (Enter)
    Enter,
    /// 空 (Space)
    Space,
    /// 消 (Delete)
    Delete,
}

/// キーマッピングのセクション（レイアウトの一面）
pub type YabFace = HashMap<PhysicalPos, YabValue>;

/// パース済みの .yab レイアウト全体
#[derive(Debug)]
pub struct YabLayout {
    /// レイアウト名
    pub name: String,
    /// 通常面
    pub normal: YabFace,
    /// 左親指シフト面
    pub left_thumb: YabFace,
    /// 右親指シフト面
    pub right_thumb: YabFace,
    /// 小指シフト面
    pub shift: YabFace,
}

/// 各行のキー数上限
const MAX_COLS: [usize; 4] = [13, 12, 12, 11];

/// 全角文字を半角文字に変換する。
/// 全角 ASCII 範囲 (U+FF01..U+FF5E) に該当する場合、対応する半角文字を返す。
fn fullwidth_to_halfwidth(ch: char) -> Option<char> {
    let cp = u32::from(ch);
    // 全角 ASCII: U+FF01 ('！') .. U+FF5E ('～')
    // 対応する半角: U+0021 ('!') .. U+007E ('~')
    if (0xFF01..=0xFF5E).contains(&cp) {
        char::from_u32(cp - 0xFEE0)
    } else {
        None
    }
}

/// 全角文字列を半角文字列に変換する。
/// 各文字について全角→半角変換を試み、変換できない文字はそのまま残す。
fn convert_fullwidth_str(s: &str) -> String {
    s.chars()
        .map(|ch| fullwidth_to_halfwidth(ch).unwrap_or(ch))
        .collect()
}

/// 文字列が全角 ASCII 文字のみで構成されているかを判定する。
fn is_all_fullwidth_ascii(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|ch| (0xFF01..=0xFF5E).contains(&u32::from(ch)))
}

/// 単一の CSV 値をパースして `YabValue` に変換する。
fn parse_value(raw: &str) -> YabValue {
    let trimmed = raw.trim();

    if trimmed.is_empty() || trimmed == "無" {
        return YabValue::None;
    }

    // 特殊キーワード
    match trimmed {
        "後" => return YabValue::Special(SpecialKey::Backspace),
        "逃" => return YabValue::Special(SpecialKey::Escape),
        "入" => return YabValue::Special(SpecialKey::Enter),
        "空" => return YabValue::Special(SpecialKey::Space),
        "消" => return YabValue::Special(SpecialKey::Delete),
        _ => {}
    }

    // シングルクォートで囲まれたリテラル（例: '．'）
    if trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() > 2 {
        let inner = &trimmed[1..trimmed.len() - 1];
        return YabValue::Literal(inner.to_string());
    }

    // 全角 ASCII 文字列 → 半角変換してローマ字として扱う
    if is_all_fullwidth_ascii(trimmed) {
        let half = convert_fullwidth_str(trimmed);
        // 半角変換後がアルファベットのみならローマ字
        if half.chars().all(|ch| ch.is_ascii_alphabetic()) {
            return YabValue::Romaji {
                romaji: half,
                kana: None,
            };
        }
        // 数字や記号はリテラル
        return YabValue::Literal(half);
    }

    // それ以外はリテラルとして扱う
    YabValue::Literal(trimmed.to_string())
}

/// セクションの 4 行分の CSV データを `YabFace` にパースする。
fn parse_face(lines: &[String]) -> Result<YabFace> {
    if lines.len() != 4 {
        bail!("Expected 4 data lines in section, got {}", lines.len());
    }

    let mut face = YabFace::new();

    for (row, line) in lines.iter().enumerate() {
        let values: Vec<&str> = line.split(',').collect();
        let max_col = MAX_COLS[row];
        if values.len() > max_col {
            bail!(
                "Row {row} has {} values, but maximum is {max_col}",
                values.len()
            );
        }

        for (col, val) in values.iter().enumerate() {
            let yab_val = parse_value(val);
            if yab_val != YabValue::None {
                let row_u8 = u8::try_from(row).expect("row index always fits in u8");
                let col_u8 = u8::try_from(col).expect("col index always fits in u8");
                let pos = PhysicalPos::new(row_u8, col_u8);
                face.insert(pos, yab_val);
            }
        }
    }

    Ok(face)
}

impl SpecialKey {
    /// 特殊キーに対応する仮想キーコード (VK code) を返す。
    #[must_use]
    pub const fn to_vk(self) -> u16 {
        match self {
            Self::Backspace => 0x08,
            Self::Escape => 0x1B,
            Self::Enter => 0x0D,
            Self::Space => 0x20,
            Self::Delete => 0x2E,
        }
    }
}

/// セクション名からフェイスの種類を判別する。
fn classify_section(name: &str) -> Option<FaceKind> {
    match name {
        "ローマ字シフト無し" => Some(FaceKind::Normal),
        "ローマ字左親指シフト" => Some(FaceKind::LeftThumb),
        "ローマ字右親指シフト" => Some(FaceKind::RightThumb),
        "ローマ字小指シフト" => Some(FaceKind::Shift),
        _ => None,
    }
}

/// レイアウトフェイスの種類
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum FaceKind {
    Normal,
    LeftThumb,
    RightThumb,
    Shift,
}

impl YabLayout {
    /// .yab 形式の文字列をパースして `YabLayout` を構築する。
    ///
    /// # Errors
    ///
    /// フォーマットが不正な場合や必須セクションが欠落している場合にエラーを返す。
    pub fn parse(input: &str) -> Result<Self> {
        let mut name = String::new();
        let mut sections: HashMap<FaceKind, Vec<String>> = HashMap::new();
        let mut current_section: Option<FaceKind> = None;
        let mut current_lines: Vec<String> = Vec::new();

        for (line_num, raw_line) in input.lines().enumerate() {
            let line = raw_line.trim();

            // 空行・コメント行はスキップ
            if line.is_empty() || line.starts_with(';') {
                continue;
            }

            // セクションヘッダ
            if line.starts_with('[') && line.ends_with(']') {
                // 前のセクションを保存
                if let Some(kind) = current_section {
                    sections.insert(kind, std::mem::take(&mut current_lines));
                }

                let section_name = &line[1..line.len() - 1];

                // 最初のセクションの前に名前が未設定なら、ファイル名相当として
                // セクション名から推測する（実際のファイルでは別途設定される場合がある）
                if name.is_empty() {
                    name = section_name.to_string();
                }

                current_section = classify_section(section_name);
                current_lines.clear();
                continue;
            }

            // データ行（セクション内）
            if current_section.is_some() {
                current_lines.push(line.to_string());
            } else {
                // セクション外のデータ行
                // 名前の行として扱う（最初の非コメント・非セクション行）
                if name.is_empty() {
                    name = line.to_string();
                }
                // セクション外の不明な行は無視しない — エラーにする
                // （ただし名前行は許容）
                if !name.is_empty() && line != name {
                    bail!(
                        "Line {}: unexpected data outside section: {line}",
                        line_num + 1
                    );
                }
            }
        }

        // 最後のセクションを保存
        if let Some(kind) = current_section {
            sections.insert(kind, current_lines);
        }

        let normal = if let Some(lines) = sections.get(&FaceKind::Normal) {
            parse_face(lines).context("Failed to parse normal face")?
        } else {
            YabFace::new()
        };

        let left_thumb = if let Some(lines) = sections.get(&FaceKind::LeftThumb) {
            parse_face(lines).context("Failed to parse left thumb face")?
        } else {
            YabFace::new()
        };

        let right_thumb = if let Some(lines) = sections.get(&FaceKind::RightThumb) {
            parse_face(lines).context("Failed to parse right thumb face")?
        } else {
            YabFace::new()
        };

        let shift = if let Some(lines) = sections.get(&FaceKind::Shift) {
            parse_face(lines).context("Failed to parse shift face")?
        } else {
            YabFace::new()
        };

        Ok(Self {
            name,
            normal,
            left_thumb,
            right_thumb,
            shift,
        })
    }

    /// ローマ字→かな逆引きテーブルを使い、各 `YabValue::Romaji` の `kana` フィールドを解決する。
    #[must_use]
    pub fn resolve_kana(mut self) -> Self {
        let table = build_romaji_to_kana();
        resolve_face_kana(&mut self.normal, &table);
        resolve_face_kana(&mut self.left_thumb, &table);
        resolve_face_kana(&mut self.right_thumb, &table);
        resolve_face_kana(&mut self.shift, &table);
        self
    }
}

/// `YabFace` 内の全 `YabValue::Romaji` の `kana` フィールドをテーブルから解決する。
fn resolve_face_kana(face: &mut YabFace, table: &HashMap<String, char>) {
    for value in face.values_mut() {
        if let YabValue::Romaji {
            ref romaji,
            ref mut kana,
        } = value
        {
            *kana = table.get(romaji.as_str()).copied();
        }
    }
}

/// 仮想キーコード (VK code) から `PhysicalPos` への変換。
///
/// JIS キーボードの一般的なマッピングに基づく。
/// 修飾キーや特殊キーは `None` を返す。
#[must_use]
pub const fn vk_to_pos(vk: u16) -> Option<PhysicalPos> {
    let (row, col) = match vk {
        // Row 0: number row (0x30..=0x39 → '0'..'9')
        0x31 => (0, 0),  // 1
        0x32 => (0, 1),  // 2
        0x33 => (0, 2),  // 3
        0x34 => (0, 3),  // 4
        0x35 => (0, 4),  // 5
        0x36 => (0, 5),  // 6
        0x37 => (0, 6),  // 7
        0x38 => (0, 7),  // 8
        0x39 => (0, 8),  // 9
        0x30 => (0, 9),  // 0
        0xBD => (0, 10), // VK_OEM_MINUS (-)
        0xDE => (0, 11), // VK_OEM_7 (^) — JIS layout
        0xDC => (0, 12), // VK_OEM_5 (¥)

        // Row 1: Q row
        0x51 => (1, 0),  // Q
        0x57 => (1, 1),  // W
        0x45 => (1, 2),  // E
        0x52 => (1, 3),  // R
        0x54 => (1, 4),  // T
        0x59 => (1, 5),  // Y
        0x55 => (1, 6),  // U
        0x49 => (1, 7),  // I
        0x4F => (1, 8),  // O
        0x50 => (1, 9),  // P
        0xC0 => (1, 10), // VK_OEM_3 (@) — JIS layout
        0xDB => (1, 11), // VK_OEM_4 ([)

        // Row 2: A row
        0x41 => (2, 0),  // A
        0x53 => (2, 1),  // S
        0x44 => (2, 2),  // D
        0x46 => (2, 3),  // F
        0x47 => (2, 4),  // G
        0x48 => (2, 5),  // H
        0x4A => (2, 6),  // J
        0x4B => (2, 7),  // K
        0x4C => (2, 8),  // L
        0xBB => (2, 9),  // VK_OEM_PLUS (;) — JIS layout
        0xBA => (2, 10), // VK_OEM_1 (:)
        0xDD => (2, 11), // VK_OEM_6 (])

        // Row 3: Z row
        0x5A => (3, 0),  // Z
        0x58 => (3, 1),  // X
        0x43 => (3, 2),  // C
        0x56 => (3, 3),  // V
        0x42 => (3, 4),  // B
        0x4E => (3, 5),  // N
        0x4D => (3, 6),  // M
        0xBC => (3, 7),  // VK_OEM_COMMA (,)
        0xBE => (3, 8),  // VK_OEM_PERIOD (.)
        0xBF => (3, 9),  // VK_OEM_2 (/)
        0xE2 => (3, 10), // VK_OEM_102 (_) — JIS layout

        _ => return None,
    };
    Some(PhysicalPos::new(row, col))
}

#[cfg(test)]
mod tests;
