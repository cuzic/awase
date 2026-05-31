use rustc_hash::FxHashMap;
use std::fmt::Write as _;

use itertools::Itertools as _;

use anyhow::{bail, Context, Result};

use crate::kana_table::KanaTable;
use crate::scanmap::{KeyboardModel, PhysicalPos};

// Re-export SpecialKey for backward compatibility (previously defined here)
pub use crate::types::SpecialKey;

/// .yab ファイルからパースされた値
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YabValue {
    /// ローマ字文字列（例: "ka", "si", "wo"）
    /// `kana` にはパース時に逆引きした仮名文字を保持する。
    /// 拗音など単一 `char` に収まらないローマ字の場合は `None`。
    Romaji { romaji: String, kana: Option<char> },
    /// リテラル文字（Unicode 文字として直接送信する）（.yab ではクォート付き）
    Literal(String),
    /// キーシーケンスとして出力（IME がキーストロークを変換する）（.yab ではクォート無し全角記号）
    KeySequence(String),
    /// 特殊キー
    Special(SpecialKey),
    /// 割り当てなし（パススルー）
    None,
}

/// 最大キー数: 4 行 × 13 列 (JIS)
const MAX_KEYS: usize = 4 * 13;
/// 列数上限
const MAX_COLS: usize = 13;
/// 行数上限
const MAX_ROWS: usize = 4;

/// キーマッピングのセクション（レイアウトの一面）
///
/// `PhysicalPos` を `row * 13 + col` の固定インデックスに変換し、
/// O(1) ルックアップを実現する。
#[derive(Clone)]
pub struct YabFace(Box<[Option<YabValue>; MAX_KEYS]>);

impl std::fmt::Debug for YabFace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // HashMap 風の出力を生成
        let mut map = f.debug_map();
        for (row, col) in (0..MAX_ROWS).cartesian_product(0..MAX_COLS) {
            let idx = row * MAX_COLS + col;
            if let Some(ref val) = self.0[idx] {
                map.entry(
                    &PhysicalPos::new(
                        u8::try_from(row).expect("row < MAX_ROWS fits u8"),
                        u8::try_from(col).expect("col < MAX_COLS fits u8"),
                    ),
                    val,
                );
            }
        }
        map.finish()
    }
}

/// `PhysicalPos` を配列インデックスに変換する。範囲外なら `None`。
const fn pos_to_index(pos: PhysicalPos) -> Option<usize> {
    let r = pos.row as usize;
    let c = pos.col as usize;
    if r >= MAX_ROWS || c >= MAX_COLS {
        None
    } else {
        Some(r * MAX_COLS + c)
    }
}

impl YabValue {
    /// 単一の CSV 値をパースして `YabValue` に変換する。
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        let trimmed = raw.trim();

        if trimmed.is_empty() || trimmed == "無" {
            return Self::None;
        }

        if let Some((_, sk)) = SPECIAL_KEYWORDS.iter().find(|(k, _)| *k == trimmed) {
            return Self::Special(*sk);
        }

        if let Some(inner) = strip_paired_quote(trimmed) {
            return Self::Literal(inner.to_string());
        }

        if trimmed.is_all_fullwidth_ascii() {
            return classify_fullwidth(trimmed);
        }

        Self::Literal(trimmed.to_string())
    }

    /// `YabValue` を .yab テキスト形式に変換する。
    #[must_use]
    pub fn serialize(&self) -> String {
        match self {
            Self::Romaji { romaji, .. } => romaji.to_fullwidth_str(),
            Self::Literal(s) => format!("'{s}'"),
            Self::KeySequence(s) => s.to_fullwidth_str(),
            Self::Special(SpecialKey::Backspace) => "後".to_string(),
            Self::Special(SpecialKey::Escape) => "逃".to_string(),
            Self::Special(SpecialKey::Enter) => "入".to_string(),
            Self::Special(SpecialKey::Space) => "空".to_string(),
            Self::Special(SpecialKey::Delete) => "消".to_string(),
            Self::None => "無".to_string(),
        }
    }
}

impl YabFace {
    /// 空の面を作成する。
    #[must_use]
    pub fn new() -> Self {
        // const { None } の配列を Box で確保
        Self(Box::new([const { None }; MAX_KEYS]))
    }

    /// 指定位置の値を参照する。
    #[must_use]
    pub fn get(&self, pos: &PhysicalPos) -> Option<&YabValue> {
        let idx = pos_to_index(*pos)?;
        self.0[idx].as_ref()
    }

    /// 指定位置に値を挿入する。
    ///
    /// # Panics
    ///
    /// `pos` が範囲外の場合パニックする。
    pub fn insert(&mut self, pos: PhysicalPos, value: YabValue) {
        let idx = pos_to_index(pos).expect("PhysicalPos out of range for YabFace");
        self.0[idx] = Some(value);
    }

    /// 指定位置にキーが定義されているか判定する。
    #[must_use]
    pub fn contains_key(&self, pos: &PhysicalPos) -> bool {
        pos_to_index(*pos).is_some_and(|idx| self.0[idx].is_some())
    }

    /// 全値への可変イテレータ（`Some` エントリのみ）。
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut YabValue> {
        self.0.iter_mut().filter_map(|slot| slot.as_mut())
    }

    /// 定義されているキーの数を返す。
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.iter().filter(|slot| slot.is_some()).count()
    }

    /// キーが一つも定義されていないか判定する。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.iter().all(Option::is_none)
    }

    /// .yab テキストの CSV 行に変換する。
    ///
    /// # Panics
    ///
    /// `row_sizes` の列数が `u8::MAX` を超える場合パニックするが、実際には起こらない。
    #[must_use]
    pub fn serialize(&self, row_sizes: &[usize; 4]) -> String {
        row_sizes
            .iter()
            .enumerate()
            .map(|(row, &cols)| {
                (0..cols)
                    .map(|col| {
                        let pos = PhysicalPos::new(
                            u8::try_from(row).expect("row < MAX_ROWS fits u8"),
                            u8::try_from(col).expect("col < MAX_COLS fits u8"),
                        );
                        self.get(&pos)
                            .map_or_else(|| "無".to_string(), YabValue::serialize)
                    })
                    .join(",")
            })
            .join("\n")
    }

    /// 全 `YabValue::Romaji` の `kana` フィールドをテーブルから解決する。
    pub fn resolve_kana(&mut self, table: &KanaTable) {
        for value in self.values_mut() {
            if let YabValue::Romaji {
                ref romaji,
                ref mut kana,
            } = value
            {
                *kana = table.kana_for_romaji(romaji);
            }
        }
    }
}

impl Default for YabFace {
    fn default() -> Self {
        Self::new()
    }
}

/// パース済みの .yab レイアウト全体
#[derive(Debug, Clone)]
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

/// 全角↔半角変換のキャラクタ拡張。
trait FullwidthCharExt {
    /// 全角 ASCII 範囲 (U+FF01..U+FF5E) なら対応する半角文字を返す。
    fn to_halfwidth_ascii(self) -> Option<char>;
}

impl FullwidthCharExt for char {
    fn to_halfwidth_ascii(self) -> Option<char> {
        let cp = u32::from(self);
        // 全角 ASCII: U+FF01 ('！') .. U+FF5E ('～')
        // 対応する半角: U+0021 ('!') .. U+007E ('~')
        if (0xFF01..=0xFF5E).contains(&cp) {
            Self::from_u32(cp - 0xFEE0)
        } else {
            None
        }
    }
}

/// 全角↔半角変換の文字列拡張。
trait FullwidthStrExt {
    fn to_halfwidth_str(&self) -> String;
    fn to_fullwidth_str(&self) -> String;
    fn is_all_fullwidth_ascii(&self) -> bool;
}

impl FullwidthStrExt for str {
    fn to_halfwidth_str(&self) -> String {
        self.chars()
            .map(|ch| ch.to_halfwidth_ascii().unwrap_or(ch))
            .collect()
    }

    fn to_fullwidth_str(&self) -> String {
        self.chars()
            .map(|ch| {
                let cp = u32::from(ch);
                // 半角 ASCII: U+0021 ('!') .. U+007E ('~')
                // 対応する全角: U+FF01 ('！') .. U+FF5E ('～')
                if (0x0021..=0x007E).contains(&cp) {
                    char::from_u32(cp + 0xFEE0).unwrap_or(ch)
                } else {
                    ch
                }
            })
            .collect()
    }

    fn is_all_fullwidth_ascii(&self) -> bool {
        !self.is_empty()
            && self
                .chars()
                .all(|ch| (0xFF01..=0xFF5E).contains(&u32::from(ch)))
    }
}

/// 特殊キーワードと対応する `SpecialKey` のテーブル
const SPECIAL_KEYWORDS: &[(&str, SpecialKey)] = &[
    ("後", SpecialKey::Backspace),
    ("逃", SpecialKey::Escape),
    ("入", SpecialKey::Enter),
    ("空", SpecialKey::Space),
    ("消", SpecialKey::Delete),
];

/// シングルまたはダブルクォートで囲まれた文字列の内側を返す（len > 2 の場合のみ）。
fn strip_paired_quote(s: &str) -> Option<&str> {
    let is_single = s.starts_with('\'') && s.ends_with('\'');
    let is_double = s.starts_with('"') && s.ends_with('"');
    if (is_single || is_double) && s.len() > 2 {
        Some(&s[1..s.len() - 1])
    } else {
        None
    }
}

/// 全角 ASCII 文字列を半角変換し、Romaji または KeySequence として返す。
fn classify_fullwidth(trimmed: &str) -> YabValue {
    let half = trimmed.to_halfwidth_str();
    if half.chars().all(|ch| ch.is_ascii_alphabetic()) {
        YabValue::Romaji {
            romaji: half,
            kana: None,
        }
    } else {
        YabValue::KeySequence(half)
    }
}

/// セクションの 4 行分の CSV データを `YabFace` にパースする。
fn parse_face(lines: &[String], model: KeyboardModel) -> Result<YabFace> {
    if lines.len() != 4 {
        bail!("Expected 4 data lines in section, got {}", lines.len());
    }

    let row_sizes = model.row_sizes();
    let mut face = YabFace::new();

    for (row, line) in lines.iter().enumerate() {
        let values: Vec<&str> = line.split(',').collect();
        let max_col = row_sizes[row];
        if values.len() > max_col {
            bail!(
                "Row {row} has {} values, but maximum is {max_col} for {model} keyboard",
                values.len()
            );
        }

        for (col, val) in values.iter().enumerate() {
            let yab_val = YabValue::parse(val);
            let row_u8 = u8::try_from(row).expect("row index always fits in u8");
            let col_u8 = u8::try_from(col).expect("col index always fits in u8");
            let pos = PhysicalPos::new(row_u8, col_u8);
            // YabValue::None（'無'）も格納する。
            // lookup_face が Some(Suppress) を返すことで
            // 「明示的な無出力」と「配列未定義」を区別できる。
            face.insert(pos, yab_val);
        }
    }

    Ok(face)
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

/// 指定されたセクションが存在すれば `parse_face` を呼び、なければ空の `YabFace` を返す。
fn parse_optional_face(
    sections: &FxHashMap<FaceKind, Vec<String>>,
    kind: FaceKind,
    model: KeyboardModel,
    context_msg: &'static str,
) -> Result<YabFace> {
    sections.get(&kind).map_or_else(
        || Ok(YabFace::new()),
        |lines| parse_face(lines, model).context(context_msg),
    )
}

/// `parse` のループ本体: 1行分の処理を行う。
fn process_yab_line(
    line_num: usize,
    line: &str,
    name: &mut String,
    current_section: &mut Option<FaceKind>,
    current_lines: &mut Vec<String>,
    sections: &mut FxHashMap<FaceKind, Vec<String>>,
) -> Result<()> {
    // 空行・コメント行はスキップ
    if line.is_empty() || line.starts_with(';') {
        return Ok(());
    }

    // セクションヘッダ
    if line.starts_with('[') && line.ends_with(']') {
        // 前のセクションを保存
        if let Some(kind) = *current_section {
            sections.insert(kind, std::mem::take(current_lines));
        }

        let section_name = &line[1..line.len() - 1];

        // 最初のセクションの前に名前が未設定なら、セクション名を名前として使う
        if name.is_empty() {
            *name = section_name.to_string();
        }

        *current_section = classify_section(section_name);
        current_lines.clear();
        return Ok(());
    }

    // データ行（セクション内）
    if current_section.is_some() {
        current_lines.push(line.to_string());
        return Ok(());
    }

    // セクション外のデータ行: 最初の非コメント・非セクション行を名前として扱う
    if name.is_empty() {
        *name = line.to_string();
        return Ok(());
    }

    // セクション外の不明な行はエラー（名前行は許容済み）
    if line != name.as_str() {
        bail!(
            "Line {}: unexpected data outside section: {line}",
            line_num + 1
        );
    }
    Ok(())
}

impl YabLayout {
    /// .yab 形式の文字列をパースして `YabLayout` を構築する。
    ///
    /// `model` で指定されたキーボードモデルに応じて各行の最大キー数が決まる。
    ///
    /// # Errors
    ///
    /// フォーマットが不正な場合や必須セクションが欠落している場合にエラーを返す。
    pub fn parse(input: &str, model: KeyboardModel) -> Result<Self> {
        let mut name = String::new();
        let mut sections: FxHashMap<FaceKind, Vec<String>> = FxHashMap::default();
        let mut current_section: Option<FaceKind> = None;
        let mut current_lines: Vec<String> = Vec::new();

        for (line_num, raw_line) in input.lines().enumerate() {
            process_yab_line(
                line_num,
                raw_line.trim(),
                &mut name,
                &mut current_section,
                &mut current_lines,
                &mut sections,
            )?;
        }

        // 最後のセクションを保存
        if let Some(kind) = current_section {
            sections.insert(kind, current_lines);
        }

        let normal = parse_optional_face(
            &sections,
            FaceKind::Normal,
            model,
            "Failed to parse normal face",
        )?;
        let left_thumb = parse_optional_face(
            &sections,
            FaceKind::LeftThumb,
            model,
            "Failed to parse left thumb face",
        )?;
        let right_thumb = parse_optional_face(
            &sections,
            FaceKind::RightThumb,
            model,
            "Failed to parse right thumb face",
        )?;
        let shift = parse_optional_face(
            &sections,
            FaceKind::Shift,
            model,
            "Failed to parse shift face",
        )?;

        Ok(Self {
            name,
            normal,
            left_thumb,
            right_thumb,
            shift,
        })
    }

    /// .yab 形式の文字列にシリアライズする。
    ///
    /// `model` で指定されたキーボードモデルに応じて各行の列数が決まる。
    #[must_use]
    pub fn serialize(&self, model: KeyboardModel) -> String {
        let row_sizes = model.row_sizes();
        let sections = [
            ("ローマ字シフト無し", &self.normal),
            ("ローマ字左親指シフト", &self.left_thumb),
            ("ローマ字右親指シフト", &self.right_thumb),
            ("ローマ字小指シフト", &self.shift),
        ];

        let mut out = self.name.clone();
        out.push('\n');

        for (name, face) in &sections {
            let _ = writeln!(out, "[{name}]");
            out.push_str(&face.serialize(&row_sizes));
            out.push('\n');
        }

        out
    }

    /// ローマ字→かな逆引きテーブルを使い、各 `YabValue::Romaji` の `kana` フィールドを解決する。
    #[must_use]
    pub fn resolve_kana(mut self) -> Self {
        let table = KanaTable::build();
        self.normal.resolve_kana(&table);
        self.left_thumb.resolve_kana(&table);
        self.right_thumb.resolve_kana(&table);
        self.shift.resolve_kana(&table);
        self
    }
}

#[cfg(test)]
mod tests;
