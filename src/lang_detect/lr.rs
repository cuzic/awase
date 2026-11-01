//! 追加式ロジスティック回帰（shadow）: 入力言語の判別（日本語 / 非日本語）を、キー位置の 1〜3 連の重みの**和**で数える。
//!
//! 今の判定器（仮名 n-gram と英語のラテン文字 2 連の尤度比）は、日本語側が鋭く英語側が緩い非対称があり、日本語として自然な
//! 列でも英数と確定することがある（2026-11-01 14:33 JST、`ccndie` = すすめてくた が llr = -14.6）。この表は、独立した実文
//! （日本語 Wikipedia・Tatoeba・英語・他言語・コード）で、日本語か否かを直接学習したもの（`tools/lang-detect-model/`）。
//! 同じ誤確定率（FPR）での非日本語の検出率が、今の判定器より 20〜35 ポイント高い（`tools/lang-detect-model/README.md`）。
//!
//! **使い方**: `EpisodeDetector` が数えた打鍵ごとの増分を `EpisodeOutcome` に残し、journal の `LangDetect` に `lr_llr`・`lr_trace`・
//! `lr_not_japanese_at` として書く(常に)。`KanaModel::with_lr_decision` を付けると、『日本語でない』の確定にも使う
//! (設定 `auto_detect.decider = "lr"`、既定)。日本語の確定(親指の重なり・仮名 n-gram の尤度比)は従来のまま。
//!
//! # 表の形式（`data/lang_lr_v1.txt`、`export_lr.py` が生成）
//! - `id=` 識別子、`scale=` 重みに掛ける値、`uni=` 47・`bi=` 2303・`tri=` 112847 個の int8 を 16 進 2 桁ずつ（2 の補数）。
//! - 打鍵 `k`（格子の行を順に並べた添字 0..46）の増分 = `(uni[k] + bi[c1 * 47 + k] + tri[(c2 * 49 + c1) * 47 + k]) * scale`。
//!   `c1`・`c2` は直前・2 つ前の文脈（キー 0..46、`BOS` = 47）。エピソードの先頭は `c1 = c2 = BOS`（`MID` = 48 は学習で語の途中を
//!   表すが、実機は語の途中かを知らないので使わない）。
//! - `bias=` は学習時の切片（クラス質量の事前）。累積に足さない（評価も足していない）。
//!
//! 純粋（OS・時刻・生の VK を持たない。入力は `PhysicalPos` だけ。ADR-019）。

use crate::scanmap::PhysicalPos;

/// 同梱の表（`export_lr.py` の出力）。
const EMBEDDED: &str = include_str!("../../data/lang_lr_v1.txt");

/// キーの添字の並び（JIS の QWERTY の格子。行ごと）。`tools/lang-detect-model/ldm/common.py` の `GRID` と同じ。
const GRID: [&str; 4] = [
    "1234567890-^\\",
    "qwertyuiop@[",
    "asdfghjkl;:]",
    "zxcvbnm,./",
];
/// 各行の先頭の添字。
const ROW_OFFSET: [usize; 4] = [0, 13, 25, 37];
const N_KEYS: usize = 47;
/// エピソードの先頭を表す文脈。
const BOS: usize = N_KEYS;
const N_CTX: usize = N_KEYS + 2;

/// shadow の『日本語でない』とみなす累積 log-odds（`-` この値以下）。
///
/// **測ったもの**: `tools/lang-detect-model/` の test 分割（135,302 エピソード）で、日本語を `NotJapanese` に確定する率（FPR）が
/// 約 0.5% になる閾値。int8 に量子化した重みで、英 83.3%・他言語 77.6%・コード 96.3% を検出し（float32 は 83.2/78.0/96.2%）、
/// FPR 1% の 3.5 と合わせて 3.5〜4.5 が実用域。クラス質量（日本語 0.5）に依存する値で、実機の事前確率は未測定。
pub const SHADOW_NOT_JAPANESE: f64 = 4.0;

/// 打鍵ごとに持ち越す文脈（直前・2 つ前）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LrContext {
    c1: u8,
    c2: u8,
}

impl Default for LrContext {
    /// エピソードの先頭（`BOS`・`BOS`）
    #[allow(clippy::cast_possible_truncation)] // BOS = 47 < 256
    fn default() -> Self {
        Self {
            c1: BOS as u8,
            c2: BOS as u8,
        }
    }
}

/// 追加式ロジスティック回帰の重み。
#[derive(Debug)]
pub struct LrModel {
    id: String,
    scale: f32,
    uni: Vec<i8>,
    bi: Vec<i8>,
    tri: Vec<i8>,
}

impl LrModel {
    /// 同梱の表から作る。
    ///
    /// # Panics
    /// 同梱の表が壊れているとき（ビルド時に決まるので、テストで検出する）。
    #[must_use]
    pub fn embedded() -> Self {
        Self::parse(EMBEDDED).expect("data/lang_lr_v1.txt")
    }

    /// 表の文字列から作る。
    ///
    /// # Errors
    /// 行が足りない・桁数が合わない・数値が読めないとき。
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut id = None;
        let mut scale = None;
        let mut tables: [Option<Vec<i8>>; 3] = [None, None, None];
        let sizes = [N_KEYS, N_CTX * N_KEYS, N_CTX * N_CTX * N_KEYS];
        for line in text
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let Some((name, value)) = line.split_once('=') else {
                return Err(format!("= の無い行: {line:.20}"));
            };
            match name {
                "id" => id = Some(value.to_string()),
                "scale" => scale = Some(value.parse::<f32>().map_err(|e| e.to_string())?),
                "bias" => {}
                "uni" | "bi" | "tri" => {
                    let i = ["uni", "bi", "tri"]
                        .iter()
                        .position(|n| *n == name)
                        .unwrap_or(0);
                    tables[i] = Some(decode_i8(value, sizes[i])?);
                }
                other => return Err(format!("知らない項目: {other}")),
            }
        }
        let [Some(uni), Some(bi), Some(tri)] = tables else {
            return Err("uni・bi・tri のどれかが無い".into());
        };
        Ok(Self {
            id: id.ok_or("id= が無い")?,
            scale: scale.ok_or("scale= が無い")?,
            uni,
            bi,
            tri,
        })
    }

    /// 識別子（ログ用）
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 位置に対応するキーの添字。格子に無い位置は `None`。
    #[must_use]
    pub fn key_index(pos: PhysicalPos) -> Option<usize> {
        let (row, col) = (usize::from(pos.row), usize::from(pos.col));
        let line = GRID.get(row)?;
        (col < line.len()).then(|| ROW_OFFSET[row] + col)
    }

    /// 1 打鍵ぶんの増分（log-odds、日本語が正）と、次へ持ち越す文脈。格子に無い位置は増分 0 で、文脈をエピソードの先頭に戻す。
    #[must_use]
    pub fn step(&self, ctx: LrContext, pos: PhysicalPos) -> (f64, LrContext) {
        let Some(k) = Self::key_index(pos) else {
            return (0.0, LrContext::default());
        };
        let (c1, c2) = (usize::from(ctx.c1), usize::from(ctx.c2));
        let sum = i32::from(self.uni[k])
            + i32::from(self.bi[c1 * N_KEYS + k])
            + i32::from(self.tri[(c2 * N_CTX + c1) * N_KEYS + k]);
        // N_KEYS = 47 < 256 なので u8 に収まる
        #[allow(clippy::cast_possible_truncation)]
        let next = LrContext {
            c1: k as u8,
            c2: ctx.c1,
        };
        (f64::from(sum) * f64::from(self.scale), next)
    }
}

/// 累積が `-SHADOW_NOT_JAPANESE` 以下になった最初の打鍵（0 始まりの添字）。無ければ `None`。
#[must_use]
pub fn first_not_japanese(trace: &[f32]) -> Option<usize> {
    let mut cum = 0.0_f64;
    trace.iter().position(|d| {
        cum += f64::from(*d);
        cum <= -SHADOW_NOT_JAPANESE
    })
}

fn decode_i8(hex: &str, n: usize) -> Result<Vec<i8>, String> {
    if hex.len() != n * 2 {
        return Err(format!("桁数 {} が {} でない", hex.len(), n * 2));
    }
    (0..n)
        .map(|i| {
            u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
                .map(|b| i8::from_ne_bytes([b]))
                .map_err(|e| e.to_string())
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::cast_possible_truncation)] // 見本の増分を f32 にする
mod tests {
    use super::*;

    fn pos_of(c: char) -> PhysicalPos {
        for (row, line) in GRID.iter().enumerate() {
            if let Some(col) = line.find(c) {
                return PhysicalPos::new(u8::try_from(row).unwrap(), u8::try_from(col).unwrap());
            }
        }
        panic!("格子に無い文字: {c:?}");
    }

    #[test]
    fn embedded_table_parses_with_the_expected_sizes() {
        let m = LrModel::embedded();
        assert_eq!(m.id(), "lr-bos-v1");
        assert_eq!(m.uni.len(), 47);
        assert_eq!(m.bi.len(), 49 * 47);
        assert_eq!(m.tri.len(), 49 * 49 * 47);
        assert!(m.scale > 0.0);
    }

    #[test]
    fn key_index_follows_the_jis_rows() {
        assert_eq!(LrModel::key_index(pos_of('1')), Some(0));
        assert_eq!(LrModel::key_index(pos_of('\\')), Some(12));
        assert_eq!(LrModel::key_index(pos_of('q')), Some(13));
        assert_eq!(LrModel::key_index(pos_of('[')), Some(24));
        assert_eq!(LrModel::key_index(pos_of('a')), Some(25));
        assert_eq!(LrModel::key_index(pos_of(']')), Some(36));
        assert_eq!(LrModel::key_index(pos_of('z')), Some(37));
        assert_eq!(LrModel::key_index(pos_of('/')), Some(46));
        assert_eq!(LrModel::key_index(PhysicalPos::new(4, 0)), None);
        assert_eq!(LrModel::key_index(PhysicalPos::new(3, 10)), None);
    }

    /// Python（`export_lr.py`）が量子化後の重みで数えた累積 log-odds と一致する。
    #[test]
    fn matches_the_python_vectors() {
        let m = LrModel::embedded();
        let mut n = 0;
        for line in include_str!("../../tests/fixtures/lang_lr_vectors.tsv")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let (keys, cums) = line.split_once('\t').expect("tab");
            let want: Vec<f64> = cums.split(',').map(|v| v.parse().expect("f64")).collect();
            let mut ctx = LrContext::default();
            let mut cum = 0.0;
            for (i, c) in keys.chars().enumerate() {
                let (d, next) = m.step(ctx, pos_of(c));
                cum += d;
                ctx = next;
                assert!(
                    (cum - want[i]).abs() < 2e-3,
                    "{keys} 打鍵{}: {cum} != {}",
                    i + 1,
                    want[i]
                );
            }
            n += 1;
        }
        assert!(n >= 80, "見本が少ない: {n}");
    }

    /// 実機の誤発火（2026-11-01 14:33 JST、`ccndie`）は、shadow では確定しない。英単語は確定する。
    #[test]
    fn the_misfire_is_not_decided_but_english_is() {
        let m = LrModel::embedded();
        let trace = |s: &str| {
            let mut ctx = LrContext::default();
            s.chars()
                .map(|c| {
                    let (d, next) = m.step(ctx, pos_of(c));
                    ctx = next;
                    d as f32
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(first_not_japanese(&trace("ccndie")), None);
        assert_eq!(first_not_japanese(&trace("windows")), Some(2));
        assert!(first_not_japanese(&trace("hello")).is_some());
        assert!(first_not_japanese(&trace("git")).is_some());
        assert_eq!(first_not_japanese(&[]), None);
    }
}
