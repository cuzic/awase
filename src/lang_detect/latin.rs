//! 判定の背景: ラテン文字の 2 連 `P(文字 | 直前の文字)`（ADR-280）。
//!
//! 打鍵位置の格子（QWERTY の JIS 配列。`PhysicalPos` の行・列）の 45 文字の遷移確率。仮名の尤度比の分母に使う
//! （一様分布 `ln(1/N_keys)` の代わり）。英単語の文字の並び（`th`・`ow`・`ll` 等）が「日本語の仮名の並びとして不自然」な
//! だけでなく「英語としては自然」と評価されるので、英数側へ早く確定する。多言語・コード・英語の混合で作った表（言語に依らない
//! 偏りを避ける）。表は `data/latin_bigram.txt`、生成手順は `tools/lang-detect-eval/gen_latin_bigram.py`。
//!
//! 純粋（OS・時刻・生の VK を持たない。入力は `PhysicalPos` だけ。ADR-019）。

use rustc_hash::FxHashMap;

use crate::scanmap::PhysicalPos;

/// 格子の文字（行ごと）。表の `chars=` は、この格子の全文字を並べたもの。
const LATIN_GRID: [&str; 4] = ["1234567890-", "qwertyuiop@[", "asdfghjkl;:]", "zxcvbnm,./"];

/// 同梱の表（`gen_latin_bigram.py` の出力）。
const EMBEDDED: &str = include_str!("../../data/latin_bigram.txt");

/// 表の識別子（ログ用）。表の形式・元を変えたら上げる。
const EMBEDDED_ID: &str = "latin-bigram-v1";

/// ラテン文字の 2 連の背景（`-ln p` を 1/16 刻みの整数にした表を、`ln p` に戻して持つ）。
#[derive(Debug)]
pub struct LatinBackground {
    /// 位置 → 文字の添字
    index: FxHashMap<PhysicalPos, usize>,
    /// `ln P(c)`（先頭の打鍵）
    ln_uni: Vec<f64>,
    /// `ln P(c | prev)`。添字は `prev * n + c`
    ln_bi: Vec<f64>,
    n: usize,
    id: &'static str,
}

impl LatinBackground {
    /// 同梱の表から作る。
    ///
    /// # Panics
    /// 同梱の表が壊れているとき（ビルド時に決まるので、テストで検出する）。
    #[must_use]
    pub fn embedded() -> Self {
        Self::parse(EMBEDDED, EMBEDDED_ID).expect("data/latin_bigram.txt")
    }

    /// 同梱の表の識別子（`embedded()` で作ったものの `id()` と同じ）
    #[must_use]
    pub const fn embedded_id() -> &'static str {
        EMBEDDED_ID
    }

    /// 表の文字列（形式は `gen_latin_bigram.py` の doc）から作る。
    ///
    /// # Errors
    /// 行の数・桁数・文字が格子と合わないとき。
    pub fn parse(text: &str, id: &'static str) -> Result<Self, String> {
        let mut lines = text
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty());
        let chars: Vec<char> = lines
            .next()
            .and_then(|l| l.strip_prefix("chars="))
            .ok_or("chars= の行が無い")?
            .chars()
            .collect();
        let n = chars.len();
        let mut index = FxHashMap::default();
        for (i, &c) in chars.iter().enumerate() {
            let pos = grid_position(c).ok_or_else(|| format!("格子に無い文字 {c:?}"))?;
            if index.insert(pos, i).is_some() {
                return Err(format!("文字 {c:?} が重複"));
            }
        }
        let uni_line = lines
            .next()
            .and_then(|l| l.strip_prefix("uni="))
            .ok_or("uni= の行が無い")?;
        let ln_uni = decode_row(uni_line, n)?;
        let mut ln_bi = Vec::with_capacity(n * n);
        for _ in 0..n {
            ln_bi.extend(decode_row(lines.next().ok_or("行が足りない")?, n)?);
        }
        if lines.next().is_some() {
            return Err("行が多い".into());
        }
        Ok(Self {
            index,
            ln_uni,
            ln_bi,
            n,
            id,
        })
    }

    /// 識別子（ログ用）
    #[must_use]
    pub const fn id(&self) -> &'static str {
        self.id
    }

    /// 位置に対応する文字の添字。格子に無いキー（`^`・`\` など）は `None`。
    #[must_use]
    pub fn index_of(&self, pos: PhysicalPos) -> Option<usize> {
        self.index.get(&pos).copied()
    }

    /// `ln P(cur | prev)`。`prev` が `None`（先頭・直前が仮名に読めない）なら `ln P(cur)`。
    #[must_use]
    pub fn ln_p(&self, prev: Option<usize>, cur: usize) -> f64 {
        match prev {
            Some(p) => self.ln_bi[p * self.n + cur],
            None => self.ln_uni[cur],
        }
    }
}

fn grid_position(c: char) -> Option<PhysicalPos> {
    for (row, line) in LATIN_GRID.iter().enumerate() {
        if let Some(col) = line.find(c) {
            return Some(PhysicalPos::new(
                u8::try_from(row).ok()?,
                u8::try_from(col).ok()?,
            ));
        }
    }
    None
}

/// 16 進 2 桁の列を、`ln p = -q / 16` に戻す。
fn decode_row(line: &str, n: usize) -> Result<Vec<f64>, String> {
    if line.len() != n * 2 {
        return Err(format!("桁数 {} が {} でない", line.len(), n * 2));
    }
    (0..n)
        .map(|i| {
            u8::from_str_radix(&line[i * 2..i * 2 + 2], 16)
                .map(|q| -f64::from(q) / 16.0)
                .map_err(|e| e.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_table_parses_and_is_a_distribution() {
        let bg = LatinBackground::embedded();
        assert_eq!(bg.n, 45);
        assert_eq!(bg.id(), "latin-bigram-v1");
        // 量子化(1/16 刻み)の誤差の範囲で、各行の確率の和が 1 に近い。
        let sum_uni: f64 = bg.ln_uni.iter().map(|x| x.exp()).sum();
        assert!((sum_uni - 1.0).abs() < 0.15, "uni の和 {sum_uni}");
        for p in 0..bg.n {
            let sum: f64 = (0..bg.n).map(|c| bg.ln_bi[p * bg.n + c].exp()).sum();
            assert!((sum - 1.0).abs() < 0.25, "行 {p} の和 {sum}");
        }
    }

    #[test]
    fn grid_positions_follow_the_qwerty_rows() {
        let bg = LatinBackground::embedded();
        assert!(bg.index_of(PhysicalPos::new(1, 0)).is_some()); // q
        assert!(bg.index_of(PhysicalPos::new(0, 10)).is_some()); // -
        assert!(bg.index_of(PhysicalPos::new(0, 11)).is_none()); // ^ は格子の外
        assert!(bg.index_of(PhysicalPos::new(3, 9)).is_some()); // /
    }

    #[test]
    fn english_pairs_are_likelier_than_unlikely_pairs() {
        let bg = LatinBackground::embedded();
        let ix = |c: char| bg.index_of(grid_position(c).expect("grid")).expect("index");
        // "th" は "tq" より自然、"ow" は "oq" より自然。
        assert!(bg.ln_p(Some(ix('t')), ix('h')) > bg.ln_p(Some(ix('t')), ix('q')) + 2.0);
        assert!(bg.ln_p(Some(ix('o')), ix('w')) > bg.ln_p(Some(ix('o')), ix('q')) + 2.0);
    }

    #[test]
    fn malformed_tables_are_rejected() {
        assert!(LatinBackground::parse("", "x").is_err());
        assert!(LatinBackground::parse("chars=ab\nuni=00\n", "x").is_err());
        assert!(LatinBackground::parse("chars=q\nuni=0\n", "x").is_err());
        assert!(LatinBackground::parse("chars=q\nuni=00\n00\n00\n", "x").is_err());
        assert!(LatinBackground::parse("chars=q\nuni=00\n00\n", "x").is_ok());
    }
}
