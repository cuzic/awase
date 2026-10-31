//! cold の最初の 1 キーを TIP が見ない（`drop_first_key`）条件の、実測の目録と推定（段階 3）。
//!
//! 擬似 Mozc の `drop_first_key` は今は宣言（真偽）だが、実機ではいつ起きるかが分からない。ここでは
//! 「起きた・起きなかった」の観測を出典付きで持ち、起きる割合を区間で推定する。**観測の数が足りないことを
//! 数字で見えるようにする**のが目的で、点推定を確率として使うことは禁止する（[`DropRate::is_conclusive`]）。
//!
//! 目録は cold の最初の送信の結末だけを数える（warm の送信・2 回目以降の送信は数えない）。n が分かっていない
//! 母集団（例: ADR-270 の「入力中の文字が空から始まった打鍵の塊 19 件」は cold の最初の送信とは限らない）は
//! 入れない。観測が増えたら（#916 以降の journal の `mozc_key_seq`）ここに足す。

#![allow(clippy::cast_precision_loss)] // 件数は小さい

/// cold の最初の送信 1 件の結末。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observation {
    pub id: &'static str,
    /// 出典（ADR・BUG・ログの時刻）。
    pub source: &'static str,
    /// 窓の種類（クラスが分かるものはクラス、分からないものは系統）。
    pub window: &'static str,
    /// cold になる直前の無操作の長さ（ms）。ログに無いものは `None`。
    pub idle_ms: Option<u64>,
    /// 最初の 1 キーを TIP が見ず、文書へ素通りしたか。
    pub dropped: bool,
}

/// 観測の目録（2026-10-31 時点）。
pub const OBSERVATIONS: &[Observation] = &[
    Observation {
        id: "adr270-wo-2026-10-19",
        source: "ADR-270 実測(2026-10-19 09:23:04 の awase.log): 「をもと」の最初の wo、入力中の文字が 16ms でできた",
        window: "unknown",
        idle_ms: None,
        dropped: false,
    },
    Observation {
        id: "bug225-ko-2026-10-28",
        source: "BUG-225(2026-10-28 00:33:38 UTC、dragonflyg4): フォーカス移動後の F2→R で ko、key_seq 330→331",
        window: "windows-terminal-family",
        idle_ms: None,
        dropped: true,
    },
    Observation {
        id: "bug226-ko-2026-10-28",
        source: "BUG-226(2026-10-28 07:41:55 UTC、dragonflyg4): LINE、cold=184、ko が kおちら",
        window: "Qt663QWindowIcon",
        idle_ms: Some(70_875),
        dropped: true,
    },
];

/// 起きた割合の推定（Wilson 区間、95%）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DropRate {
    pub dropped: usize,
    pub n: usize,
    pub low: f64,
    pub high: f64,
}

impl DropRate {
    /// 区間が十分に狭く、確率として使ってよいか。幅が 0.3 を超えるうちは使わない。
    pub fn is_conclusive(&self) -> bool {
        self.n > 0 && self.high - self.low <= 0.3
    }
}

/// `dropped` 件が起き、`n` 件中の割合の Wilson 区間（z=1.96）。
pub fn wilson(dropped: usize, n: usize) -> DropRate {
    if n == 0 {
        return DropRate {
            dropped,
            n,
            low: 0.0,
            high: 1.0,
        };
    }
    let (k, nf, z) = (dropped as f64, n as f64, 1.96_f64);
    let p = k / nf;
    let denom = 1.0 + z * z / nf;
    let centre = p + z * z / (2.0 * nf);
    let margin = z * ((p * (1.0 - p) + z * z / (4.0 * nf)) / nf).sqrt();
    DropRate {
        dropped,
        n,
        low: ((centre - margin) / denom).max(0.0),
        high: ((centre + margin) / denom).min(1.0),
    }
}

/// 目録全体の推定。
pub fn overall() -> DropRate {
    let dropped = OBSERVATIONS.iter().filter(|o| o.dropped).count();
    wilson(dropped, OBSERVATIONS.len())
}
